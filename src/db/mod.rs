pub(crate) mod background; // FORK: background Claude sessions (src/fork/background.rs)
mod claude_worktrees; // FORK: Claude-created worktrees (src/fork/claude_worktrees.rs)
mod debug_log;
pub mod editors;
pub(crate) mod fan_out; // FORK: pending prompts and fan-out groups (src/fork/fan_out.rs)
mod followups; // FORK: queued follow-ups (src/app/fork_followups.rs)
mod hook_approvals; // FORK: repo lifecycle hook approvals (src/fork/hook_approval.rs)
mod messaging; // FORK: Claude inbox sockets (src/fork/messaging.rs)
mod migrations;
mod organize; // FORK: feature groups and notes (src/fork/organize.rs)
mod pr_terminal_state;
pub mod prompt_templates;
mod schedules; // FORK: scheduled prompts and commands (src/fork/schedules.rs)
mod session_names; // FORK: harness session names (src/fork/session_identity.rs)
mod session_status;
pub mod store;
pub mod todos;
mod token_cache;
pub mod unsent_prompts;

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use std::env;
use std::path::{Path, PathBuf};

use crate::worktree::WorktreeManager;

pub struct AiwDb {
    conn: Connection,
    pub path: PathBuf,
}

impl AiwDb {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        migrations::run(&conn)?;
        Ok(Self {
            conn,
            path: path.to_path_buf(),
        })
    }

    /// Open an existing database for reading only.
    ///
    /// [`Self::open`] is a write: it creates `~/.config/ai_workstation/`, creates the file,
    /// switches the journal to WAL, and applies any pending migration. That is
    /// right for launching AIW and wrong for `aiw doctor`, which promises to
    /// change nothing — a diagnostic that has to modify the thing it inspects
    /// can't report on it honestly, and on an older schema it would silently
    /// upgrade the database out from under whatever AIW version the user is
    /// still running.
    ///
    /// So: no `CREATE` flag (a missing database is an error, not a new file),
    /// no migrations, no journal change, and `SQLITE_OPEN_READ_ONLY` — which
    /// matters beyond intent. A read-write handle, even one that runs no
    /// `INSERT`, checkpoints the write-ahead log into the main file when it is
    /// the last connection to close, rewriting the very bytes the command
    /// promised not to touch. A read-only handle cannot, so `doctor` leaves the
    /// file identical. `query_only` is belt and braces on top.
    ///
    /// The cost is that a WAL database whose shared-memory index is missing —
    /// a writer died without checkpointing — cannot be read at all. That is
    /// reported like a missing database (advice about the machine, nothing
    /// about the store), which is the right trade for a command whose only
    /// promise is that it changes nothing.
    pub fn open_read_only(path: &Path) -> Result<Self> {
        if !path.exists() {
            anyhow::bail!("no AIW database at {}", path.display());
        }
        let conn = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.execute_batch("PRAGMA query_only=ON;")?;
        Ok(Self {
            conn,
            path: path.to_path_buf(),
        })
    }

    /// Open `path`, seeding or merging legacy stores first if needed.
    ///
    /// Merge priority:
    /// 1. Import a legacy worktree-local `aiw.db`.
    /// 2. Import a legacy worktree-local `projects.json`.
    /// 3. Import from the global `~/.config/ai_workstation/projects.json`.
    pub fn open_or_seed(path: &Path, _global_path: &Path) -> Result<Self> {
        let legacy_store = load_legacy_worktree_store();

        if !path.exists() {
            if let Some(store) = legacy_store {
                let db = AiwDb::open(path)?;
                db.save_store(&store)?;
                return Ok(db);
            }

            seed_from_json(path, Vec::new());
            return Self::open(path);
        }

        if let Some(store) = legacy_store {
            let db = Self::open(path)?;
            let mut merged = db.load_store()?;
            merged.merge_from(store);
            db.save_store(&merged)?;
        }

        Self::open(path)
    }

    pub fn load_store(&self) -> Result<crate::project::ProjectStore> {
        store::load(&self.conn)
    }

    /// The `store_meta` version currently on disk, with no store data: a
    /// cheap "has anyone written since?" probe before paying for a full
    /// [`Self::load_store_versioned`].
    pub fn current_store_version(&self) -> Result<u64> {
        store::current_version(&self.conn)
    }

    /// [`Self::load_store`] plus the store version it was read at. Use this
    /// (not `load_store`) for a load that will later save through
    /// [`Self::save_store_checked`] — see `store::load_versioned`.
    pub fn load_store_versioned(&self) -> Result<(crate::project::ProjectStore, u64)> {
        store::load_versioned(&self.conn)
    }

    /// Unconditional full-replace save, for the one-time seed/merge at
    /// [`Self::open_or_seed`] time — nothing else holds a loaded version to
    /// race against yet. Application code saving a live, previously-loaded
    /// store must use [`Self::save_store_checked`] instead so a concurrent
    /// writer (the GUI, the TUI, or another AIW process) is detected rather
    /// than silently overwritten; see `store::save`'s doc comment.
    ///
    /// Returns the version this save committed, read inside the same write
    /// transaction, so it is the version of *this* data and not of a write
    /// that landed after it.
    pub fn save_store(&self, store: &crate::project::ProjectStore) -> Result<u64> {
        store::save(&self.conn, store)
    }

    /// Cross-process-safe save: succeeds only if nothing has saved since
    /// `expected_version` (from [`Self::load_store_versioned`], or a
    /// previous save's returned version) was read. See `store::save_checked`.
    pub fn save_store_checked(
        &self,
        store: &crate::project::ProjectStore,
        expected_version: u64,
    ) -> Result<store::SaveOutcome> {
        store::save_checked(&self.conn, store, expected_version)
    }

    /// Fresh from disk, not the in-memory `ProjectStore` snapshot — see
    /// `db::prompt_templates` for why templates are read/written directly.
    pub fn load_prompt_templates(&self) -> Result<Vec<crate::prompt_library::PromptTemplate>> {
        prompt_templates::load(&self.conn)
    }

    pub fn insert_prompt_template(
        &self,
        template: &crate::prompt_library::PromptTemplate,
    ) -> Result<()> {
        prompt_templates::insert(&self.conn, template)
    }

    pub fn update_prompt_template(
        &self,
        template: &crate::prompt_library::PromptTemplate,
    ) -> Result<()> {
        prompt_templates::update(&self.conn, template)
    }

    pub fn delete_prompt_template(&self, id: &str) -> Result<()> {
        prompt_templates::delete(&self.conn, id)
    }

    pub fn load_unsent_prompts_for_workdir(
        &self,
        workdir: &Path,
    ) -> Result<Vec<unsent_prompts::UnsentPrompt>> {
        unsent_prompts::load_for_workdir(&self.conn, workdir)
    }

    pub fn insert_unsent_prompt(
        &self,
        id: &str,
        workdir: &Path,
        label: &str,
        body: &str,
        created_at: &DateTime<Utc>,
    ) -> Result<()> {
        unsent_prompts::insert(&self.conn, id, workdir, label, body, created_at)
    }

    pub fn delete_unsent_prompt(&self, id: &str) -> Result<()> {
        unsent_prompts::delete(&self.conn, id)
    }

    pub fn delete_unsent_prompts_for_workdir(&self, workdir: &Path) -> Result<()> {
        unsent_prompts::delete_for_workdir(&self.conn, workdir)
    }

    pub fn load_token_cache(&self) -> Result<Vec<crate::token_tracking::DbTokenCacheEntry>> {
        token_cache::load(&self.conn)
    }

    pub fn save_token_cache(
        &self,
        entries: &[crate::token_tracking::DbTokenCacheEntry],
    ) -> Result<()> {
        token_cache::save(&self.conn, entries)
    }

    pub fn evict_stale_token_cache(&self) -> Result<()> {
        token_cache::evict_stale(&self.conn)
    }

    /// Every cached terminal (merged/closed) PR, keyed by `(repo, branch)`.
    /// Loaded once at startup to seed the dashboard badge before the first
    /// sweep runs.
    pub fn load_all_pr_terminal_state(
        &self,
    ) -> Result<std::collections::HashMap<(String, String), crate::github::TerminalPr>> {
        pr_terminal_state::load_all(&self.conn)
    }

    /// Persist one branch's terminal PR state. Called by the sweep on every
    /// positive lookup — cheap, since it never needs to happen again for that
    /// branch.
    pub fn save_pr_terminal_state(
        &self,
        repo: &str,
        branch: &str,
        pr: &crate::github::TerminalPr,
    ) -> Result<()> {
        pr_terminal_state::save(&self.conn, repo, branch, pr)
    }

    /// Remove the terminal PR cached for a deleted feature's repository and
    /// branch so a future feature can safely reuse the branch name.
    pub fn delete_pr_terminal_state(&self, repo: &str, branch: &str) -> Result<()> {
        pr_terminal_state::delete(&self.conn, repo, branch)
    }

    pub fn append_log_entry(&self, entry: &crate::debug::LogEntry) -> Result<()> {
        debug_log::append(&self.conn, entry)
    }

    pub fn load_recent_log(&self, limit: usize) -> Result<Vec<crate::debug::LogEntry>> {
        debug_log::load_recent(&self.conn, limit)
    }

    #[allow(dead_code)] // exercised only by unit tests
    pub fn load_session_status(&self, session_id: &str) -> Result<Option<String>> {
        session_status::load(&self.conn, session_id)
    }

    /// Returns `(status_text, file_mtime_nanos)` for the cached entry.
    /// `file_mtime_nanos` is `None` when the row was written without a mtime
    /// (e.g. from an IPC push or the old schema).
    pub fn load_session_status_with_mtime(
        &self,
        session_id: &str,
    ) -> Result<Option<(String, Option<u64>)>> {
        session_status::load_with_mtime(&self.conn, session_id)
    }

    pub fn upsert_session_status(
        &self,
        session_id: &str,
        feature_id: &str,
        status_text: &str,
        file_mtime_nanos: Option<u64>,
    ) -> Result<()> {
        session_status::upsert(
            &self.conn,
            session_id,
            feature_id,
            status_text,
            file_mtime_nanos,
        )
    }

    pub fn delete_session_status(&self, session_id: &str) -> Result<()> {
        session_status::delete_session(&self.conn, session_id)
    }

    pub fn delete_feature_statuses(&self, feature_id: &str) -> Result<()> {
        session_status::delete_feature(&self.conn, feature_id)
    }
}

/// Editors AIW launched for a feature (see `db/editors.rs`), tracked so
/// stopping the feature can reclaim them.
impl AiwDb {
    #[allow(clippy::too_many_arguments)]
    pub fn record_launched_editor(
        &self,
        feature_id: &str,
        session_id: Option<&str>,
        kind: editors::EditorKind,
        pid: i64,
        worktree_path: &Path,
        dedicated: bool,
        command: &str,
    ) -> Result<editors::LaunchedEditor> {
        editors::record_launch(
            &self.conn,
            feature_id,
            session_id,
            kind,
            pid,
            worktree_path,
            dedicated,
            command,
        )
    }

    pub fn launched_editors_for_feature(
        &self,
        feature_id: &str,
    ) -> Result<Vec<editors::LaunchedEditor>> {
        editors::list_for_feature(&self.conn, feature_id)
    }

    pub fn all_launched_editors(&self) -> Result<Vec<editors::LaunchedEditor>> {
        editors::list_all(&self.conn)
    }

    pub fn set_launched_editor_owner(
        &self,
        id: &str,
        pid: i64,
        dedicated: bool,
        proc_started_at: &str,
    ) -> Result<()> {
        editors::set_owner(&self.conn, id, pid, dedicated, proc_started_at)
    }

    pub fn delete_launched_editor(&self, id: &str) -> Result<()> {
        editors::delete(&self.conn, id)
    }

    pub fn delete_launched_editors_for_feature(&self, feature_id: &str) -> Result<()> {
        editors::delete_for_feature(&self.conn, feature_id)
    }
}

// FORK(no-cloud): Remote Control's paired-device and Web Push accessors were
// removed with the feature. Their tables (migrations 39/43) are left in place,
// unused, so an existing `aiw.db` keeps opening.

/// Scoped TODO list persistence (worktree / project / global).
#[allow(dead_code)]
impl AiwDb {
    /// The TODO list for `scope`, or `None` if that scope has no list yet.
    pub fn todo_list(&self, scope: &todos::TodoScope) -> Result<Option<todos::TodoList>> {
        todos::load_list(&self.conn, scope)
    }

    /// A TODO list by its own id, whatever scope it is in.
    pub fn todo_list_by_id(&self, list_id: &str) -> Result<Option<todos::TodoList>> {
        todos::load_list_by_id(&self.conn, list_id)
    }

    /// Return the scope's TODO list, creating one hosted by `feature_id` if
    /// none exists.
    pub fn load_or_create_todo_list(
        &self,
        scope: &todos::TodoScope,
        feature_id: Option<&str>,
    ) -> Result<todos::TodoList> {
        todos::load_or_create_list(&self.conn, scope, feature_id)
    }

    /// Create the scope's TODO list under `feature_id`; errors if one exists.
    pub fn create_todo_list(
        &self,
        scope: &todos::TodoScope,
        feature_id: Option<&str>,
    ) -> Result<todos::TodoList> {
        todos::create_list(&self.conn, scope, feature_id)
    }

    /// Move a TODO into another list, appending it there and keeping its links.
    pub fn move_todo(&self, todo_id: &str, target_list_id: &str) -> Result<()> {
        todos::move_todo(&self.conn, todo_id, target_list_id)
    }

    /// Copy a TODO into another list as an unstarted item.
    pub fn copy_todo(&self, todo_id: &str, target_list_id: &str) -> Result<Option<todos::Todo>> {
        todos::copy_todo(&self.conn, todo_id, target_list_id)
    }

    pub fn set_todo_carry_over(&self, list_id: &str, carry_over: Option<&str>) -> Result<()> {
        todos::set_carry_over(&self.conn, list_id, carry_over)
    }

    pub fn set_todo_list_host_feature(&self, list_id: &str, feature_id: &str) -> Result<()> {
        todos::set_host_feature(&self.conn, list_id, feature_id)
    }

    pub fn delete_todo_list(&self, list_id: &str) -> Result<()> {
        todos::delete_list(&self.conn, list_id)
    }

    /// Delete every list belonging to a project — its project-scoped list and
    /// each of its worktree lists — when the project is deleted.
    pub fn delete_todo_lists_for_project(&self, project_id: &str) -> Result<()> {
        todos::delete_lists_for_project(&self.conn, project_id)
    }

    /// Delete the worktree-scoped list at `workdir`, if any.
    pub fn delete_worktree_todo_list(&self, project_id: &str, workdir: &str) -> Result<()> {
        todos::delete_worktree_list(&self.conn, project_id, workdir)
    }

    pub fn todos(&self, list_id: &str) -> Result<Vec<todos::Todo>> {
        todos::list_todos(&self.conn, list_id)
    }

    /// Look up a single TODO by id without knowing which list currently
    /// holds it (it may have been moved/copied since the caller last saw it).
    pub fn find_todo_by_id(&self, todo_id: &str) -> Result<Option<todos::Todo>> {
        todos::find_todo_by_id(&self.conn, todo_id)
    }

    /// Resolve a TODO by its stable id together with the scope of the list
    /// that currently owns it.  This follows TODO moves automatically.
    pub fn resolve_todo_by_id(&self, todo_id: &str) -> Result<Option<todos::ResolvedTodo>> {
        todos::resolve_todo_by_id(&self.conn, todo_id)
    }

    pub fn add_todo(
        &self,
        list_id: &str,
        title: &str,
        body: Option<&str>,
        priority: todos::TodoPriority,
    ) -> Result<todos::Todo> {
        todos::add_todo(&self.conn, list_id, title, body, priority)
    }

    pub fn update_todo(&self, todo: &todos::Todo) -> Result<()> {
        todos::update_todo(&self.conn, todo)
    }

    pub fn delete_todo(&self, todo_id: &str) -> Result<()> {
        todos::delete_todo(&self.conn, todo_id)
    }

    pub fn clear_todo_linked_feature(&self, feature_id: &str) -> Result<()> {
        todos::clear_linked_feature(&self.conn, feature_id)
    }

    pub fn clear_todo_linked_feature_for_todo(&self, todo_id: &str) -> Result<()> {
        todos::clear_linked_feature_for_todo(&self.conn, todo_id)
    }

    pub fn set_todo_linked_feature(&self, todo_id: &str, feature_id: &str) -> Result<()> {
        todos::set_linked_feature(&self.conn, todo_id, feature_id)
    }

    pub fn set_todo_agent_session(&self, todo_id: &str, session_id: &str) -> Result<()> {
        todos::set_agent_session(&self.conn, todo_id, session_id)
    }

    pub fn clear_todo_agent_session(&self, todo_id: &str) -> Result<()> {
        todos::clear_agent_session(&self.conn, todo_id)
    }

    pub fn todo_agent_session_associations(&self) -> Result<Vec<(String, String)>> {
        todos::agent_session_associations(&self.conn)
    }

    pub fn clear_missing_todo_agent_sessions(&self) -> Result<usize> {
        todos::clear_missing_agent_sessions(&self.conn)
    }

    pub fn set_todo_work_state(&self, todo_id: &str, work: &todos::TodoWorkState) -> Result<()> {
        todos::set_work_state(&self.conn, todo_id, work)
    }

    pub fn reserve_todo_agent_launch(&self, todo_id: &str) -> Result<bool> {
        todos::reserve_agent_launch(&self.conn, todo_id)
    }

    pub fn associate_reserved_todo_agent_session(
        &self,
        todo_id: &str,
        session_id: &str,
    ) -> Result<bool> {
        todos::associate_reserved_agent_session(&self.conn, todo_id, session_id)
    }

    pub fn rollback_reserved_todo_agent_launch(
        &self,
        todo_id: &str,
        launched_session: Option<&str>,
    ) -> Result<bool> {
        todos::rollback_reserved_agent_launch(&self.conn, todo_id, launched_session)
    }

    pub fn reorder_todos(&self, ordered_ids: &[String]) -> Result<()> {
        todos::reorder_todos(&self.conn, ordered_ids)
    }
}

fn seed_from_json(db_path: &Path, extra_candidates: Vec<PathBuf>) {
    let mut json_candidates: Vec<PathBuf> = vec![
        db_path
            .parent()
            .map(|p| p.join("projects.json"))
            .unwrap_or_default(),
        dirs::config_dir()
            .unwrap_or_default()
            .join("ai_workstation")
            .join("projects.json"),
        dirs::config_dir()
            .unwrap_or_default()
            .join("claude-super-vibeless")
            .join("projects.json"),
    ];
    json_candidates.splice(0..0, extra_candidates);

    for json_path in json_candidates {
        if json_path.exists() {
            if let Ok(store) = crate::project::ProjectStore::load(&json_path)
                && let Ok(db) = AiwDb::open(db_path)
            {
                let _ = db.save_store(&store);
            }
            return;
        }
    }
}

fn load_legacy_worktree_store() -> Option<crate::project::ProjectStore> {
    let current_dir = env::current_dir().ok()?;
    if !WorktreeManager::is_worktree(&current_dir) {
        return None;
    }

    let root = WorktreeManager::repo_root(&current_dir).ok()?;
    let db_path = root.join(".aiw").join("aiw.db");
    let json_path = root.join(".aiw").join("projects.json");

    let mut merged: Option<crate::project::ProjectStore> = None;

    if json_path.exists()
        && let Ok(store) = crate::project::ProjectStore::load(&json_path)
    {
        merged = Some(store);
    }

    if db_path.exists()
        && let Ok(source_db) = AiwDb::open(&db_path)
        && let Ok(store) = source_db.load_store()
    {
        match &mut merged {
            Some(existing) => existing.merge_from(store),
            None => merged = Some(store),
        }
    }

    merged
}

#[cfg(test)]
mod read_only_tests {
    use super::*;

    /// `aiw doctor` promises to change nothing, so its handle must not create
    /// the database, migrate it, or switch its journal mode.
    #[test]
    fn a_read_only_handle_refuses_to_create_the_database() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("missing").join("aiw.db");

        assert!(AiwDb::open_read_only(&path).is_err());
        assert!(!path.exists(), "doctor must not create the database");
        assert!(
            !path.parent().unwrap().exists(),
            "doctor must not create ~/.config/ai_workstation either"
        );
    }

    #[test]
    fn a_read_only_handle_neither_migrates_nor_writes() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        // A database at an older schema: opening it normally would upgrade it
        // out from under whatever AIW version the user is still running.
        {
            let conn = Connection::open(tmp.path()).unwrap();
            conn.execute_batch(
                "CREATE TABLE schema_version (version INTEGER PRIMARY KEY,
                    applied_at TEXT NOT NULL, description TEXT NOT NULL);
                 INSERT INTO schema_version VALUES (3, datetime('now'), 'seed');",
            )
            .unwrap();
        }

        let db = AiwDb::open_read_only(tmp.path()).unwrap();
        let version: i64 = db
            .conn
            .query_row("SELECT MAX(version) FROM schema_version", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(version, 3, "the schema must be left exactly as it was");

        assert!(
            db.conn
                .execute("INSERT INTO schema_version VALUES (99, 'now', 'x')", [])
                .is_err(),
            "query_only must reject writes"
        );

        let journal: String = db
            .conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        assert_ne!(
            journal.to_lowercase(),
            "wal",
            "the journal mode must not be switched by a read"
        );
    }

    /// The subtle half: a read-write handle that runs no statement at all still
    /// rewrites the database, because closing the last connection to a WAL
    /// database checkpoints the log into the main file. Only a genuinely
    /// read-only handle leaves the bytes alone.
    #[test]
    fn reading_a_wal_database_leaves_its_bytes_untouched() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("aiw.db");

        // A writer left open, so the log is still un-checkpointed while the
        // read happens — the state `aiw doctor` finds while AIW is running.
        let writer = AiwDb::open(&path).unwrap();
        writer
            .save_store(&crate::project::ProjectStore::empty())
            .unwrap();
        assert!(
            path.with_extension("db-wal").exists(),
            "expected an un-checkpointed write-ahead log"
        );
        let before = std::fs::read(&path).unwrap();

        {
            let reader = AiwDb::open_read_only(&path).unwrap();
            reader.load_store().unwrap();
        }

        assert_eq!(
            std::fs::read(&path).unwrap(),
            before,
            "the read must not checkpoint the log into the database file"
        );
        drop(writer);
    }
}
