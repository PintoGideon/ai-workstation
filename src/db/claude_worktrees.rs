//! FORK: the worktrees Claude created through AIW's `WorktreeCreate` hook
//! (`src/fork/claude_worktrees.rs`, DESIGN.md §19.6). Table created by
//! `migrations::FORK_SCHEMA`, and by [`AiwDb::open_for_hook`] for a hook
//! process that must not run AIW's migrations.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::AiwDb;
use crate::fork::claude_worktrees::ClaudeWorktree;

/// The table, exactly as in `FORK_SCHEMA` (a test keeps the two equal).
pub(super) const TABLE_DDL: &str = "CREATE TABLE IF NOT EXISTS aiw_claude_worktrees (id TEXT PRIMARY KEY, repo TEXT NOT NULL, path TEXT NOT NULL UNIQUE, branch TEXT NOT NULL, base TEXT NOT NULL, name TEXT NOT NULL, origin TEXT NOT NULL, parent_session_id TEXT, claude_session_id TEXT, requested_cwd TEXT NOT NULL, port_start INTEGER, port_count INTEGER, created_at TEXT NOT NULL);";

const COLUMNS: &str = "id, repo, path, branch, base, name, origin, parent_session_id, claude_session_id, requested_cwd, port_start, port_count, created_at";

fn from_row(row: &Row<'_>) -> rusqlite::Result<ClaudeWorktree> {
    Ok(ClaudeWorktree {
        id: row.get(0)?,
        repo: PathBuf::from(row.get::<_, String>(1)?),
        path: PathBuf::from(row.get::<_, String>(2)?),
        branch: row.get(3)?,
        base: row.get(4)?,
        name: row.get(5)?,
        origin: row.get(6)?,
        parent_session_id: row.get(7)?,
        claude_session_id: row.get(8)?,
        requested_cwd: PathBuf::from(row.get::<_, String>(9)?),
        port_start: row
            .get::<_, Option<i64>>(10)?
            .and_then(|p| u16::try_from(p).ok()),
        port_count: row
            .get::<_, Option<i64>>(11)?
            .and_then(|p| u16::try_from(p).ok()),
        created_at: row.get(12)?,
    })
}

impl AiwDb {
    /// Open the existing store from a hook process: no migrations (an older
    /// dashboard may be running on this schema), no journal change, a busy
    /// timeout so a write waits for the dashboard instead of failing, and
    /// only this table created if missing. A store that does not exist yet
    /// is an error: the hook never creates AIW's database.
    pub(crate) fn open_for_hook(path: &Path) -> Result<Self> {
        if !path.exists() {
            anyhow::bail!("no AIW database at {}", path.display());
        }
        let conn = Connection::open(path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch(TABLE_DDL)?;
        Ok(Self {
            conn,
            path: path.to_path_buf(),
        })
    }

    pub(crate) fn insert_claude_worktree(&self, wt: &ClaudeWorktree) -> Result<()> {
        self.conn.execute(
            &format!(
                "INSERT INTO aiw_claude_worktrees ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                 ON CONFLICT(path) DO UPDATE SET id = excluded.id, repo = excluded.repo, branch = excluded.branch,
                     base = excluded.base, name = excluded.name, origin = excluded.origin,
                     parent_session_id = excluded.parent_session_id, claude_session_id = excluded.claude_session_id,
                     requested_cwd = excluded.requested_cwd, port_start = excluded.port_start,
                     port_count = excluded.port_count, created_at = excluded.created_at"
            ),
            params![
                wt.id,
                wt.repo.to_string_lossy(),
                wt.path.to_string_lossy(),
                wt.branch,
                wt.base,
                wt.name,
                wt.origin,
                wt.parent_session_id,
                wt.claude_session_id,
                wt.requested_cwd.to_string_lossy(),
                wt.port_start.map(i64::from),
                wt.port_count.map(i64::from),
                wt.created_at,
            ],
        )?;
        Ok(())
    }

    /// Every recorded Claude worktree, oldest first.
    pub(crate) fn all_claude_worktrees(&self) -> Result<Vec<ClaudeWorktree>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {COLUMNS} FROM aiw_claude_worktrees ORDER BY created_at, id"
        ))?;
        let rows = stmt.query_map([], from_row)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub(crate) fn claude_worktree_by_path(&self, path: &Path) -> Result<Option<ClaudeWorktree>> {
        Ok(self
            .conn
            .query_row(
                &format!("SELECT {COLUMNS} FROM aiw_claude_worktrees WHERE path = ?1"),
                [path.to_string_lossy()],
                from_row,
            )
            .optional()?)
    }

    /// The newest worktree Claude named `name` in `repo`.
    pub(crate) fn claude_worktree_by_name(
        &self,
        repo: &Path,
        name: &str,
    ) -> Result<Option<ClaudeWorktree>> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    "SELECT {COLUMNS} FROM aiw_claude_worktrees WHERE repo = ?1 AND name = ?2
                     ORDER BY created_at DESC LIMIT 1"
                ),
                params![repo.to_string_lossy(), name],
                from_row,
            )
            .optional()?)
    }

    pub(crate) fn delete_claude_worktree(&self, id: &str) -> Result<()> {
        self.conn
            .execute("DELETE FROM aiw_claude_worktrees WHERE id = ?1", [id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn sample(id: &str, path: &str) -> ClaudeWorktree {
        ClaudeWorktree {
            id: id.into(),
            repo: PathBuf::from("/repo"),
            path: PathBuf::from(path),
            branch: format!("worktree-{id}"),
            base: "abc123".into(),
            name: id.into(),
            origin: "subagent".into(),
            parent_session_id: Some("fs-1".into()),
            claude_session_id: Some("conv-1".into()),
            requested_cwd: PathBuf::from("/repo"),
            port_start: Some(4110),
            port_count: Some(10),
            created_at: format!("2026-10-11T00:00:0{}Z", id.len() % 10),
        }
    }

    #[test]
    fn the_table_matches_fork_schema() {
        assert!(
            super::super::migrations::FORK_SCHEMA.contains(TABLE_DDL),
            "FORK_SCHEMA and TABLE_DDL disagree"
        );
    }

    #[test]
    fn records_round_trip_and_are_found_by_path_and_name() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("aiw.db");
        {
            let db = AiwDb::open(&path).unwrap();
            db.insert_claude_worktree(&sample("a", "/w/claude-a"))
                .unwrap();
            db.insert_claude_worktree(&sample("bb", "/w/claude-b"))
                .unwrap();
        }
        // A hook process opens the same store without migrations.
        let db = AiwDb::open_for_hook(&path).unwrap();
        let all = db.all_claude_worktrees().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0], sample("a", "/w/claude-a"));
        assert_eq!(
            db.claude_worktree_by_path(Path::new("/w/claude-b"))
                .unwrap()
                .unwrap()
                .id,
            "bb"
        );
        assert_eq!(
            db.claude_worktree_by_name(Path::new("/repo"), "a")
                .unwrap()
                .unwrap()
                .path,
            PathBuf::from("/w/claude-a")
        );
        assert!(
            db.claude_worktree_by_name(Path::new("/other"), "a")
                .unwrap()
                .is_none()
        );
        db.delete_claude_worktree("a").unwrap();
        assert_eq!(db.all_claude_worktrees().unwrap().len(), 1);
    }

    #[test]
    fn a_hook_never_creates_the_store() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("aiw.db");
        assert!(AiwDb::open_for_hook(&path).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn a_hook_adds_the_table_to_an_older_store() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("aiw.db");
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE unrelated (x INTEGER);")
                .unwrap();
        }
        let db = AiwDb::open_for_hook(&path).unwrap();
        db.insert_claude_worktree(&sample("a", "/w/a")).unwrap();
        assert_eq!(db.all_claude_worktrees().unwrap().len(), 1);
    }
}
