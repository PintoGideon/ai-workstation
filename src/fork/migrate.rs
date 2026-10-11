//! FORK: first-run copy of an upstream AMF setup into AIW's own folders.
//!
//! AIW used to share AMF's folders. Since the rename it keeps everything in
//! its own: `~/.config/ai_workstation` (config, `aiw.db`, hooks) and
//! `~/.local/state/ai_workstation` (tmux socket, IPC, debug log). The first
//! time AIW starts and `~/.config/ai_workstation` does not exist yet, an AMF
//! config folder found where AMF keeps it is **copied** into the new folder.
//!
//! Guarantees:
//! - The AMF folder is only ever read: nothing in it is moved, modified,
//!   created or deleted, and the AMF database is never opened (SQLite would
//!   create `-wal`/`-shm` files next to it). AMF keeps working side by side.
//! - The database is copied as `amf.db` + `amf.db-wal` byte for byte, but
//!   only while neither file changes during the copy (a running AMF that is
//!   writing gets a few retries). The *copy* is then opened, its WAL folded
//!   in, and checked with `PRAGMA quick_check` before it becomes `aiw.db`.
//!   `amf.db-shm` is never copied: it is a shared-memory index SQLite
//!   rebuilds from the WAL, and a copy taken while AMF has it mapped could be
//!   stale.
//! - Everything is staged in a sibling folder and renamed into place at the
//!   end, so a failure leaves no half-populated `~/.config/ai_workstation`
//!   behind (which would otherwise stop the migration from ever retrying).
//!
//! What is not copied: AMF's pending notifications (they belong to AMF's
//! own running sessions and are answered through AMF's reply paths), the
//! OpenCode plugin folder (OpenCode support was removed), and helper scripts
//! / hook-refresh stamps, which AIW regenerates in its own folders.
//!
//! The result is recorded in [`MARKER_FILE`] inside the new folder; the TUI
//! shows its summary once as a toast ([`take_pending_notice`]).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Written into the new config folder by a migration.
pub const MARKER_FILE: &str = "migrated-from-amf.json";
/// AMF's database file name.
const LEGACY_DB: &str = "amf.db";
/// AIW's database file name.
const DB: &str = "aiw.db";
/// How often the database copy is retried while AMF keeps writing to it.
const DB_COPY_ATTEMPTS: u32 = 10;
const DB_COPY_RETRY_DELAY: Duration = Duration::from_millis(300);

/// What a migration did, kept in [`MARKER_FILE`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationRecord {
    /// The AMF config folder that was copied (left unchanged).
    pub from: PathBuf,
    pub at: DateTime<Utc>,
    pub projects: usize,
    pub features: usize,
    /// Top-level entries copied, as named in the new folder.
    pub copied: Vec<String>,
    /// Top-level entries deliberately left out, with the reason.
    pub skipped: Vec<String>,
    /// Whether the TUI has already told the user.
    #[serde(default)]
    pub notice_shown: bool,
}

#[derive(Debug)]
pub enum Outcome {
    /// `~/.config/ai_workstation` already exists, or there is no AMF setup.
    NotNeeded,
    Migrated(MigrationRecord),
}

/// Copy the AMF setup on this machine into AIW's config folder, if this is
/// AIW's first run. See the module docs for what is copied and the
/// guarantees about the AMF folder.
pub fn migrate_on_first_run() -> Result<Outcome> {
    migrate(
        &crate::project::legacy_amf_config_dir(),
        &crate::project::aiw_config_dir(),
    )
}

/// [`migrate_on_first_run`] with explicit folders.
pub fn migrate(old: &Path, new: &Path) -> Result<Outcome> {
    if new.exists() || !old.is_dir() {
        return Ok(Outcome::NotNeeded);
    }
    let parent = new
        .parent()
        .with_context(|| format!("{} has no parent folder", new.display()))?;
    std::fs::create_dir_all(parent)
        .with_context(|| format!("could not create {}", parent.display()))?;
    let name = new
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| crate::project::AIW_DIR_NAME.to_string());
    let staging = parent.join(format!(".{name}-migrating-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir(&staging)
        .with_context(|| format!("could not create {}", staging.display()))?;

    let staged = stage(old, &staging).and_then(|record| {
        write_record(&staging, &record)?;
        Ok(record)
    });
    let record = match staged {
        Ok(record) => record,
        Err(err) => {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(err.context(format!(
                "could not copy the AMF setup from {} (nothing was changed there)",
                old.display()
            )));
        }
    };
    if new.exists() {
        // Another AIW process (the GUI, a second TUI) finished first.
        let _ = std::fs::remove_dir_all(&staging);
        return Ok(Outcome::NotNeeded);
    }
    if let Err(err) = std::fs::rename(&staging, new) {
        let _ = std::fs::remove_dir_all(&staging);
        if new.exists() {
            return Ok(Outcome::NotNeeded);
        }
        return Err(anyhow::Error::new(err).context(format!(
            "could not move the copied setup to {}",
            new.display()
        )));
    }
    Ok(Outcome::Migrated(record))
}

/// Why a top-level entry of the AMF folder is not copied, if it isn't.
fn skip_reason(name: &str, is_dir: bool) -> Option<&'static str> {
    match name {
        "amf.db-wal" | "amf.db-shm" | "amf.db-journal" => Some("folded into aiw.db"),
        "notifications" => Some("pending requests of AMF's own running sessions"),
        "plugins" => Some("OpenCode plugins (OpenCode support was removed)"),
        "hooks" => Some("hook scripts are regenerated by AIW"),
        "last_hook_refresh_version" => Some("regenerated by AIW"),
        _ if !is_dir && name.ends_with(".sh") => Some("helper script, regenerated by AIW"),
        _ if !is_dir && name.ends_with(".lock") => Some("lock file"),
        _ => None,
    }
}

fn stage(old: &Path, staging: &Path) -> Result<MigrationRecord> {
    let mut entries: Vec<_> = std::fs::read_dir(old)
        .with_context(|| format!("could not read {}", old.display()))?
        .collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|entry| entry.file_name());

    let mut record = MigrationRecord {
        from: old.to_path_buf(),
        at: Utc::now(),
        projects: 0,
        features: 0,
        copied: Vec::new(),
        skipped: Vec::new(),
        notice_shown: false,
    };
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let source = entry.path();
        let is_dir = source.is_dir();
        if name == LEGACY_DB {
            let (projects, features) = copy_database(old, &staging.join(DB))?;
            record.projects = projects;
            record.features = features;
            record.copied.push(format!("{LEGACY_DB} -> {DB}"));
            continue;
        }
        if let Some(reason) = skip_reason(&name, is_dir) {
            record.skipped.push(format!("{name} ({reason})"));
            continue;
        }
        copy_tree(&source, &staging.join(&name))
            .with_context(|| format!("could not copy {}", source.display()))?;
        record.copied.push(name);
    }
    Ok(record)
}

/// Copy a file, folder or symlink (as a symlink) from `source` to `dest`.
fn copy_tree(source: &Path, dest: &Path) -> Result<()> {
    let meta = std::fs::symlink_metadata(source)?;
    if meta.file_type().is_symlink() {
        #[cfg(unix)]
        std::os::unix::fs::symlink(std::fs::read_link(source)?, dest)?;
        return Ok(());
    }
    if meta.is_dir() {
        std::fs::create_dir(dest)?;
        let mut children: Vec<_> = std::fs::read_dir(source)?.collect::<std::io::Result<_>>()?;
        children.sort_by_key(|child| child.file_name());
        for child in children {
            copy_tree(&child.path(), &dest.join(child.file_name()))?;
        }
        return Ok(());
    }
    if meta.is_file() {
        std::fs::copy(source, dest)?;
    }
    // Sockets, fifos and the like are runtime artefacts: nothing to copy.
    Ok(())
}

fn file_state(path: &Path) -> Option<(u64, SystemTime)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

fn wal_path(db: &Path) -> PathBuf {
    let mut name = db.as_os_str().to_owned();
    name.push("-wal");
    PathBuf::from(name)
}

fn shm_path(db: &Path) -> PathBuf {
    let mut name = db.as_os_str().to_owned();
    name.push("-shm");
    PathBuf::from(name)
}

/// Copy AMF's database into `dest` and return its (projects, features)
/// counts. Only reads `old`; see the module docs.
fn copy_database(old: &Path, dest: &Path) -> Result<(usize, usize)> {
    let source = old.join(LEGACY_DB);
    let source_wal = wal_path(&source);
    let mut copy_name = dest.as_os_str().to_owned();
    copy_name.push(".copy");
    let copy = PathBuf::from(copy_name);
    let copy_wal = wal_path(&copy);

    let mut consistent = false;
    for attempt in 0..DB_COPY_ATTEMPTS {
        if attempt > 0 {
            std::thread::sleep(DB_COPY_RETRY_DELAY);
        }
        let before = (file_state(&source), file_state(&source_wal));
        std::fs::copy(&source, &copy)
            .with_context(|| format!("could not copy {}", source.display()))?;
        let _ = std::fs::remove_file(&copy_wal);
        if source_wal.exists() {
            std::fs::copy(&source_wal, &copy_wal)
                .with_context(|| format!("could not copy {}", source_wal.display()))?;
        }
        let after = (file_state(&source), file_state(&source_wal));
        if before == after && before.0.is_some() {
            consistent = true;
            break;
        }
    }
    if !consistent {
        let _ = std::fs::remove_file(&copy);
        let _ = std::fs::remove_file(&copy_wal);
        bail!(
            "AMF kept writing to {} while it was being copied; quit AMF (or wait a moment) and start AIW again",
            source.display()
        );
    }

    let counts = (|| -> Result<(usize, usize)> {
        let conn = rusqlite::Connection::open(&copy)?;
        // Fold the copied WAL into the copy, so `aiw.db` stands on its own.
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
        let check: String = conn.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
        if check != "ok" {
            bail!("the copied database failed its integrity check: {check}");
        }
        let count = |table: &str| -> usize {
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get::<_, i64>(0)
            })
            .map(|n| n.max(0) as usize)
            .unwrap_or(0)
        };
        Ok((count("projects"), count("features")))
    })();
    let counts = match counts {
        Ok(counts) => counts,
        Err(err) => {
            for path in [&copy, &copy_wal, &shm_path(&copy)] {
                let _ = std::fs::remove_file(path);
            }
            return Err(err);
        }
    };
    for path in [copy_wal, shm_path(&copy)] {
        let _ = std::fs::remove_file(path);
    }
    std::fs::rename(&copy, dest).with_context(|| format!("could not create {}", dest.display()))?;
    Ok(counts)
}

fn write_record(dir: &Path, record: &MigrationRecord) -> Result<()> {
    let json = serde_json::to_string_pretty(record)?;
    std::fs::write(dir.join(MARKER_FILE), json + "\n")?;
    Ok(())
}

/// The one-time "your AMF setup was copied" message, if a migration happened
/// and its notice has not been shown yet. Marks it shown.
pub fn take_pending_notice(config_dir: &Path) -> Option<String> {
    let path = config_dir.join(MARKER_FILE);
    let mut record: MigrationRecord =
        serde_json::from_str(&std::fs::read_to_string(&path).ok()?).ok()?;
    if record.notice_shown {
        return None;
    }
    record.notice_shown = true;
    let _ = write_record(config_dir, &record);
    Some(notice_text(&record))
}

pub fn notice_text(record: &MigrationRecord) -> String {
    let projects = match record.projects {
        1 => "1 project".to_string(),
        n => format!("{n} projects"),
    };
    format!(
        "Copied your AMF setup ({projects}) into AI Workstation's own folders; AMF's folders were not changed. \
         Features from AMF show as stopped here: select one and start it to bring it back."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::AiwDb;
    use crate::project::{AgentKind, Feature, Project, ProjectStore, VibeMode};
    use std::collections::{BTreeMap, HashMap};
    use tempfile::TempDir;

    /// Every file under `dir` with its bytes, for "nothing changed" checks.
    fn snapshot(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        let mut files = BTreeMap::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(path) = stack.pop() {
            for entry in std::fs::read_dir(&path).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    files.insert(path.clone(), std::fs::read(&path).unwrap_or_default());
                }
            }
        }
        files
    }

    fn store_with_feature(tmux_session: &str) -> ProjectStore {
        let mut project = Project::new(
            "legacy".into(),
            "/tmp/legacy-repo".into(),
            true,
            AgentKind::Claude,
        );
        let mut feature = Feature::new(
            "old-feature".into(),
            "old-feature".into(),
            "/tmp/legacy-repo".into(),
            false,
            VibeMode::Vibe,
            AgentKind::Claude,
            false,
        );
        feature.tmux_session = tmux_session.to_string();
        project.features.push(feature);
        ProjectStore {
            version: crate::project::CURRENT_PROJECT_STORE_VERSION,
            projects: vec![project],
            session_bookmarks: vec![],
            available_harnesses: vec![],
            prompt_templates: Vec::new(),
            extra: HashMap::new(),
        }
    }

    /// An AMF folder as AMF leaves it, with the database still open in WAL
    /// mode (so `amf.db-wal` holds committed data) while it is copied.
    fn fake_amf_dir(root: &Path) -> (PathBuf, AiwDb) {
        let old = root.join("amf");
        std::fs::create_dir_all(old.join("notifications")).unwrap();
        std::fs::create_dir_all(old.join("plugins")).unwrap();
        std::fs::create_dir_all(old.join("gui-themes")).unwrap();
        std::fs::write(old.join("config.json"), r#"{"theme":"amf"}"#).unwrap();
        std::fs::write(old.join("notifications/n1.json"), "{}").unwrap();
        std::fs::write(old.join("plugins/input-request.js"), "//").unwrap();
        std::fs::write(old.join("gui-themes/mine.json"), "{}").unwrap();
        std::fs::write(old.join("codex-notify.sh"), "#!/bin/sh").unwrap();
        std::fs::write(old.join("last_hook_refresh_version"), "0.49.0").unwrap();
        // Left open: a small save stays below SQLite's auto-checkpoint size,
        // so the committed rows live only in `amf.db-wal` during the copy.
        let db = AiwDb::open(&old.join(LEGACY_DB)).unwrap();
        db.save_store(&store_with_feature("amf-old-feature"))
            .unwrap();
        (old, db)
    }

    #[test]
    fn copies_the_amf_setup_and_leaves_it_untouched() {
        let root = TempDir::new().unwrap();
        let (old, db) = fake_amf_dir(root.path());
        assert!(
            wal_path(&old.join(LEGACY_DB)).exists(),
            "fixture keeps a WAL"
        );
        let before = snapshot(&old);
        let new = root.path().join("ai_workstation");

        let Outcome::Migrated(record) = migrate(&old, &new).unwrap() else {
            panic!("expected a migration");
        };
        // Compared while "AMF" still has its database open: closing it is
        // AMF's own business (it checkpoints and removes its WAL).
        assert!(snapshot(&old) == before, "the AMF folder must not change");
        drop(db);
        assert_eq!(record.projects, 1);
        assert_eq!(record.features, 1);
        assert!(new.join("config.json").exists());
        assert!(new.join("gui-themes/mine.json").exists());
        assert!(new.join(DB).exists());
        assert!(!new.join(LEGACY_DB).exists());
        for skipped in [
            "notifications",
            "plugins",
            "codex-notify.sh",
            "last_hook_refresh_version",
        ] {
            assert!(!new.join(skipped).exists(), "{skipped} must not be copied");
        }
        assert!(!wal_path(&new.join(DB)).exists(), "the WAL is folded in");

        // The copied database opens and holds AMF's feature, with its stored
        // tmux session name.
        let copied = AiwDb::open(&new.join(DB)).unwrap().load_store().unwrap();
        assert_eq!(
            copied.projects[0].features[0].tmux_session,
            "amf-old-feature"
        );

        // The notice is shown exactly once.
        let notice = take_pending_notice(&new).expect("first notice");
        assert!(notice.contains("1 project"), "{notice}");
        assert!(take_pending_notice(&new).is_none());

        // A second run does nothing: the new folder exists now.
        assert!(matches!(migrate(&old, &new).unwrap(), Outcome::NotNeeded));
        // No staging folder is left behind.
        let leftovers: Vec<_> = std::fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains("migrating"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn nothing_happens_without_an_amf_folder_or_when_aiw_already_has_one() {
        let root = TempDir::new().unwrap();
        let new = root.path().join("ai_workstation");
        assert!(matches!(
            migrate(&root.path().join("amf"), &new).unwrap(),
            Outcome::NotNeeded
        ));
        assert!(!new.exists());

        let (old, _db) = fake_amf_dir(root.path());
        std::fs::create_dir_all(&new).unwrap();
        assert!(matches!(migrate(&old, &new).unwrap(), Outcome::NotNeeded));
        assert!(!new.join("config.json").exists());
    }

    #[test]
    fn a_failed_copy_leaves_no_new_folder_so_the_next_start_retries() {
        let root = TempDir::new().unwrap();
        let old = root.path().join("amf");
        std::fs::create_dir_all(&old).unwrap();
        // Not a database: the copy's integrity check fails.
        std::fs::write(old.join(LEGACY_DB), "not sqlite").unwrap();
        let before = snapshot(&old);
        let new = root.path().join("ai_workstation");

        assert!(migrate(&old, &new).is_err());
        assert!(!new.exists());
        assert_eq!(snapshot(&old), before);
        let leftovers = std::fs::read_dir(root.path()).unwrap().count();
        assert_eq!(leftovers, 1, "only the AMF folder remains");
    }
}
