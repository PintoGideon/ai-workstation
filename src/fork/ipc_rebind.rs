//! FORK: re-bind the IPC socket when its file disappears while AIW runs.
//!
//! Hooks reach the dashboard through `~/.local/state/ai_workstation/aiw.sock`.
//! If that file is removed under a running AIW (a state-dir cleanup, a tidy
//! script, `rm -rf` by hand) the listener keeps the unlinked inode, nothing
//! can connect any more, and every hook silently falls back to its sentinel
//! files until AIW is restarted. The 5-second sweep calls [`rebind_if_vanished`],
//! which notices the missing file and binds a fresh listener at the same path.
//!
//! The old accept thread is parked in `accept()` on an inode nobody can reach;
//! it exits the next time it wakes (never, in practice) and is a one-off leak
//! per vanished socket, which is cheaper than the alternative of polling.

use crate::app::App;

/// Bind a new IPC listener if the socket file of the current one is gone.
/// Returns whether a re-bind happened.
pub(crate) fn rebind_if_vanished(app: &mut App) -> bool {
    let Some(path) = app.ipc.as_ref().map(|guard| guard.path().to_path_buf()) else {
        return false;
    };
    if path.exists() {
        return false;
    }
    // Drop first: the old guard's `Drop` removes the (already missing) file,
    // and must not run after the new one bound the same path.
    app.ipc = None;
    match crate::ipc::start_with_wakeup(&path, Some(app.view_wakeup_tx())) {
        Ok(guard) => {
            app.log_warn(
                "ipc",
                format!(
                    "socket file {} had vanished; listening on it again",
                    path.display()
                ),
            );
            app.ipc = Some(guard);
            true
        }
        Err(err) => {
            app.log_warn(
                "ipc",
                format!(
                    "socket file {} had vanished and could not be re-bound: {err}",
                    path.display()
                ),
            );
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::ProjectStore;
    use crate::traits::{MockTmuxOps, MockWorktreeOps};

    fn app() -> App {
        App::new_for_test(
            ProjectStore::empty(),
            Box::new(MockTmuxOps::new()),
            Box::new(MockWorktreeOps::new()),
        )
    }

    #[test]
    fn nothing_happens_without_a_listener_or_while_the_file_exists() {
        let mut app = app();
        assert!(!rebind_if_vanished(&mut app));

        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("aiw.sock");
        app.ipc = Some(crate::ipc::start(&path).unwrap());
        assert!(!rebind_if_vanished(&mut app));
        assert!(path.exists());
    }

    #[test]
    fn a_removed_socket_file_is_bound_again_at_the_same_path() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("aiw.sock");
        let mut app = app();
        app.ipc = Some(crate::ipc::start(&path).unwrap());

        std::fs::remove_file(&path).unwrap();
        assert!(rebind_if_vanished(&mut app));
        assert!(path.exists(), "a new listener owns the path");
        assert_eq!(app.ipc.as_ref().unwrap().path(), path);

        // The new listener really accepts: a message arrives on its channel.
        crate::ipc::send(&path, r#"{"type":"ping"}"#).unwrap();
        let got = app
            .ipc
            .as_ref()
            .unwrap()
            .rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert_eq!(got["type"], "ping");
    }
}
