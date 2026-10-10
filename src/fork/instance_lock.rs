//! FORK: one TUI per config directory.
//!
//! Two `aiw` TUIs on the same config dir share `aiw.db` and the second one
//! re-binds the IPC socket out from under the first, so hooks reach the
//! wrong window. The TUI holds an advisory `flock` on `<config dir>/aiw.lock`
//! for as long as it runs; a second TUI sees the lock, prints who holds it
//! and exits. Subcommands (`doctor`, `grid`, `fonts`, `automation`, `notify`)
//! never take the lock, so they keep working alongside a running TUI.
//!
//! `flock` is released by the kernel when the holder exits, however it
//! exits, so a crash leaves nothing to clean up: the file stays behind but
//! locks nothing.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

pub(crate) const LOCK_FILE: &str = "aiw.lock";

/// Holds the lock for as long as it lives.
#[derive(Debug)]
pub struct InstanceLock {
    _file: File,
    path: PathBuf,
}

impl InstanceLock {
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Why the lock could not be taken.
#[derive(Debug)]
pub enum LockError {
    /// Another process holds it; `pid` is what that process wrote into the
    /// file (missing when it could not be read).
    Held { pid: Option<u32> },
    /// The file could not be opened or locked for another reason.
    Io(std::io::Error),
}

impl LockError {
    /// The message a second TUI prints before exiting.
    pub fn user_message(&self) -> String {
        match self {
            LockError::Held { pid: Some(pid) } => format!(
                "AIW is already running (pid {pid}). Use that window, or `aiw grid`/`aiw doctor` which work alongside it."
            ),
            LockError::Held { pid: None } => "AIW is already running. Use that window, or `aiw grid`/`aiw doctor` which work alongside it.".to_string(),
            LockError::Io(err) => format!("Could not take the AIW instance lock: {err}"),
        }
    }
}

/// Take the TUI lock under `config_dir`, writing this process's pid into
/// the file so a second instance can say who holds it.
pub fn acquire(config_dir: &Path) -> Result<InstanceLock, LockError> {
    std::fs::create_dir_all(config_dir).map_err(LockError::Io)?;
    let path = config_dir.join(LOCK_FILE);
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(LockError::Io)?;

    // SAFETY: `flock` only reads the descriptor, which `file` keeps open for
    // as long as the returned guard lives.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        if err.kind() == std::io::ErrorKind::WouldBlock {
            return Err(LockError::Held {
                pid: read_pid(&path),
            });
        }
        return Err(LockError::Io(err));
    }

    // Best effort: the lock is what matters, the pid is a courtesy.
    let _ = file.set_len(0);
    let _ = writeln!(file, "{}", std::process::id());
    let _ = file.flush();
    Ok(InstanceLock { _file: file, path })
}

fn read_pid(path: &Path) -> Option<u32> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_acquire_is_refused_while_the_first_lives() {
        let dir = tempfile::tempdir().unwrap();
        let first = acquire(dir.path()).unwrap();
        assert!(first.path().exists());

        match acquire(dir.path()) {
            Err(LockError::Held { pid }) => {
                assert_eq!(pid, Some(std::process::id()));
            }
            other => panic!("expected the lock to be held, got {other:?}"),
        }
        let message = LockError::Held {
            pid: Some(std::process::id()),
        }
        .user_message();
        assert!(message.contains("already running"), "{message}");
        assert!(message.contains(&std::process::id().to_string()));
        assert!(message.contains("aiw doctor"));
    }

    #[test]
    fn a_dropped_lock_is_free_again_even_though_the_file_remains() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOCK_FILE);
        {
            let _held = acquire(dir.path()).unwrap();
        }
        // The file left behind by a previous (possibly crashed) run locks
        // nothing on its own.
        assert!(path.exists());
        std::fs::write(&path, "424242\n").unwrap();
        let again = acquire(dir.path()).unwrap();
        assert_eq!(read_pid(again.path()), Some(std::process::id()));
    }

    #[test]
    fn creates_the_config_dir_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("fresh").join("ai_workstation");
        let lock = acquire(&nested).unwrap();
        assert_eq!(lock.path(), nested.join(LOCK_FILE));
    }

    #[test]
    fn held_without_a_readable_pid_still_explains_itself() {
        let message = LockError::Held { pid: None }.user_message();
        assert!(message.starts_with("AIW is already running."));
        assert!(message.contains("aiw grid"));
    }
}
