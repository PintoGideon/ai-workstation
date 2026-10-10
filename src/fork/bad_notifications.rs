//! FORK: quarantine notification files that cannot be parsed.
//!
//! A malformed `.json` in a notifications directory used to stay there for
//! ever and be re-parsed by every 5-second sweep. Now it is moved to
//! `<state dir>/notifications/bad/` (at most [`KEEP`] files; older ones are
//! deleted) and logged once, at the move. A file younger than
//! [`MIN_AGE`] is left alone: a hook may still be writing it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime};

/// How many quarantined files to keep.
pub(crate) const KEEP: usize = 50;
/// A malformed file younger than this may still be being written.
pub(crate) const MIN_AGE: Duration = Duration::from_secs(2);

/// `<state dir>/notifications/bad`.
pub(crate) fn bad_dir() -> PathBuf {
    crate::ipc::state_dir().join("notifications").join("bad")
}

/// Move `path` into [`bad_dir`] (under its own name, suffixed when taken),
/// then trim the quarantine to [`KEEP`] newest files. Returns the new
/// location, or `None` when the file was left where it was (too young,
/// gone already, or the move failed).
pub(crate) fn quarantine(path: &Path, reason: &str) -> Option<PathBuf> {
    quarantine_into(path, reason, &bad_dir())
}

pub(crate) fn quarantine_into(path: &Path, reason: &str, dir: &Path) -> Option<PathBuf> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    if unmovable()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(path)
        .is_some_and(|seen| *seen == modified)
    {
        return None;
    }
    if SystemTime::now()
        .duration_since(modified)
        .is_ok_and(|age| age < MIN_AGE)
    {
        return None;
    }
    std::fs::create_dir_all(dir).ok()?;
    let name = path.file_name()?.to_string_lossy().into_owned();
    let mut target = dir.join(&name);
    let mut n = 1;
    while target.exists() {
        target = dir.join(format!("{name}.{n}"));
        n += 1;
    }
    if std::fs::rename(path, &target).is_err() {
        // Across filesystems (a feature's `.claude/notifications` on another
        // mount): copy, then remove.
        std::fs::copy(path, &target).ok()?;
        if let Err(err) = std::fs::remove_file(path) {
            // The original cannot be deleted (a read-only or foreign-owned
            // folder): undo the copy and stop trying until the file
            // changes, or every sweep would quarantine another copy.
            let _ = std::fs::remove_file(&target);
            unmovable()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(path.to_path_buf(), modified);
            crate::debug::log_to_file(
                crate::debug::LogLevel::Warn,
                "notify",
                &format!(
                    "notification file {} could not be parsed ({reason}) and cannot be removed ({err}); ignoring it until it changes",
                    path.display()
                ),
            );
            return None;
        }
    }
    crate::debug::log_to_file(
        crate::debug::LogLevel::Warn,
        "notify",
        &format!(
            "notification file {} could not be parsed ({reason}); moved to {}",
            path.display(),
            target.display()
        ),
    );
    trim(dir);
    Some(target)
}

/// Malformed files that could be copied but not removed, with the
/// modification time they had then. Skipped until that time changes.
fn unmovable() -> &'static Mutex<HashMap<PathBuf, SystemTime>> {
    static UNMOVABLE: OnceLock<Mutex<HashMap<PathBuf, SystemTime>>> = OnceLock::new();
    UNMOVABLE.get_or_init(Default::default)
}

/// Keep the [`KEEP`] newest files in `dir`, delete the rest.
fn trim(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| {
            let modified = e
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            (modified, e.path())
        })
        .collect();
    if files.len() <= KEEP {
        return;
    }
    // Oldest first; ties broken by name so the order is stable.
    files.sort();
    for (_, path) in files.iter().take(files.len() - KEEP) {
        let _ = std::fs::remove_file(path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn old_file(dir: &Path, name: &str, content: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, content).unwrap();
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(60))
            .unwrap();
        path
    }

    #[test]
    fn a_malformed_file_is_moved_and_an_in_flight_one_is_left() {
        let tmp = tempfile::tempdir().unwrap();
        let bad = tmp.path().join("bad");
        let junk = old_file(tmp.path(), "junk.json", "{not json");
        let fresh = tmp.path().join("fresh.json");
        std::fs::write(&fresh, "{still being writ").unwrap();

        let moved = quarantine_into(&junk, "expected value", &bad).unwrap();
        assert_eq!(moved, bad.join("junk.json"));
        assert!(!junk.exists());
        assert_eq!(std::fs::read_to_string(&moved).unwrap(), "{not json");

        assert!(quarantine_into(&fresh, "eof", &bad).is_none());
        assert!(fresh.exists());

        // Same name again: kept apart, not overwritten.
        let again = old_file(tmp.path(), "junk.json", "second");
        let moved2 = quarantine_into(&again, "x", &bad).unwrap();
        assert_eq!(moved2, bad.join("junk.json.1"));
        assert_eq!(
            std::fs::read_to_string(bad.join("junk.json")).unwrap(),
            "{not json"
        );
    }

    #[test]
    fn a_file_that_cannot_be_removed_is_not_copied_again_every_sweep() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let bad = tmp.path().join("bad");
        let locked = tmp.path().join("locked");
        std::fs::create_dir_all(&locked).unwrap();
        let junk = old_file(&locked, "junk.json", "{not json");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        // Root ignores the read-only bit; nothing to check then.
        if std::fs::File::create(locked.join("probe")).is_ok() {
            std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
            return;
        }

        for _ in 0..3 {
            assert!(quarantine_into(&junk, "x", &bad).is_none());
        }
        assert!(junk.exists());
        let copies = std::fs::read_dir(&bad).map(|d| d.count()).unwrap_or(0);
        assert_eq!(copies, 0, "no copy is left behind");

        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn the_quarantine_keeps_only_the_newest_fifty() {
        let tmp = tempfile::tempdir().unwrap();
        let bad = tmp.path().join("bad");
        std::fs::create_dir_all(&bad).unwrap();
        for i in 0..60 {
            let path = bad.join(format!("old-{i:03}.json"));
            std::fs::write(&path, "x").unwrap();
            let file = std::fs::File::options().write(true).open(&path).unwrap();
            file.set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(1_000 + i))
                .unwrap();
        }
        let newest = old_file(tmp.path(), "newest.json", "}");
        quarantine_into(&newest, "x", &bad).unwrap();

        let mut names: Vec<String> = std::fs::read_dir(&bad)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names.len(), KEEP);
        assert!(names.contains(&"newest.json".to_string()));
        // The eleven oldest went.
        assert!(!names.contains(&"old-000.json".to_string()));
        assert!(!names.contains(&"old-010.json".to_string()));
        assert!(names.contains(&"old-011.json".to_string()));
    }
}
