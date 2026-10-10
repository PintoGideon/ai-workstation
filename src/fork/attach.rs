//! FORK: native attach (DESIGN.md §15.4) — hand the terminal to the tmux
//! window an agent runs in, so typing is the terminal's own, with no
//! `send-keys` proxy and no capture-and-render loop in between.
//!
//! The dashboard records a `PendingAttach`; the main loop drains it, tears
//! the TUI down exactly as it does for `$EDITOR`, runs `tmux attach-session`
//! on AIW's socket with the terminal inherited, and takes the screen back
//! when the client detaches. `Ctrl+Q` detaches (bound in the root key table
//! of AIW's private server, which only AIW attaches to); tmux's own
//! `prefix d` keeps working.
//!
//! Nothing here captures or polls: while attached, AIW is idle.

use crate::tmux::TmuxManager;

/// tmux's default `status-right` for AIW sessions (see `TmuxManager`), put
/// back after an attach.
pub const DEFAULT_STATUS_RIGHT: &str = " #[fg=cyan]prefix+s#[default]: sessions ";

/// The key that returns to the dashboard: the same one that leaves the
/// in-app pane, so there is one "back" key.
pub const DETACH_KEY: &str = "C-q";

/// What the main loop attaches to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingAttach {
    pub session: String,
    pub window: String,
    /// `project › feature › session`, for the tmux status line and the
    /// message shown on return.
    pub title: String,
    /// Feature-session id, so the badge's unread `✓` is cleared on return.
    pub session_id: String,
}

/// Arguments after `tmux -S <socket>` for one attach, as a single tmux
/// invocation (`;`-separated commands):
///
/// 1. the window follows the attached client's size (the dashboard's
///    `resize-window` had pinned it to the pane's size, which would leave
///    the rest of a bigger terminal blank);
/// 2. a status line that says where you are and how to get back;
/// 3. `Ctrl+Q` → `detach-client` (private server only: a user-owned server
///    keeps its own key tables);
/// 4. the attach itself, on the window.
pub fn attach_args(req: &PendingAttach, private_server: bool) -> Vec<String> {
    let target = format!("{}:{}", req.session, req.window);
    let back = if private_server {
        " Ctrl+Q: back to AIW "
    } else {
        " prefix d: back to AIW "
    };
    let mut args: Vec<String> = Vec::new();
    let mut push = |cmd: &[&str]| {
        if !args.is_empty() {
            args.push(";".into());
        }
        args.extend(cmd.iter().map(|s| s.to_string()));
    };
    push(&["set-option", "-w", "-t", &target, "window-size", "latest"]);
    push(&["set-option", "-t", &req.session, "status", "on"]);
    push(&[
        "set-option",
        "-t",
        &req.session,
        "status-left",
        &format!(" #[bold]AIW#[default] {} ", req.title),
    ]);
    push(&["set-option", "-t", &req.session, "status-left-length", "80"]);
    push(&["set-option", "-t", &req.session, "status-right", back]);
    if private_server {
        push(&["bind-key", "-T", "root", DETACH_KEY, "detach-client"]);
    }
    push(&["attach-session", "-t", &target]);
    args
}

/// Arguments that undo the status-line changes once the client has
/// detached. The window size is restored by the dashboard's next
/// `resize-window`; the key binding is harmless and stays.
pub fn restore_args(req: &PendingAttach) -> Vec<Vec<String>> {
    vec![
        vec![
            "set-option".into(),
            "-u".into(),
            "-t".into(),
            req.session.clone(),
            "status-left".into(),
        ],
        vec![
            "set-option".into(),
            "-u".into(),
            "-t".into(),
            req.session.clone(),
            "status-left-length".into(),
        ],
        vec![
            "set-option".into(),
            "-t".into(),
            req.session.clone(),
            "status-right".into(),
            DEFAULT_STATUS_RIGHT.into(),
        ],
    ]
}

/// Run the attach on the inherited terminal and wait for the detach. The
/// caller has already left the alternate screen and raw mode.
pub fn run_attach(req: &PendingAttach) -> std::io::Result<std::process::ExitStatus> {
    let private_server = TmuxManager::manages_private_socket();
    let mut command = TmuxManager::command();
    // A nested tmux refuses to attach while `$TMUX` is set; AIW's server is
    // a different one, so the guard does not apply.
    command.env_remove("TMUX").env_remove("TMUX_PANE");
    command.args(attach_args(req, private_server));
    let status = command.status();
    for args in restore_args(req) {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let _ = TmuxManager::command()
            .args(&args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> PendingAttach {
        PendingAttach {
            session: "aiw-rs-hx-12".into(),
            window: "claude".into(),
            title: "RS › hx-12 › claude".into(),
            session_id: "sess-1".into(),
        }
    }

    fn commands(args: &[String]) -> Vec<Vec<&str>> {
        args.split(|a| a == ";")
            .map(|cmd| cmd.iter().map(String::as_str).collect())
            .collect()
    }

    #[test]
    fn attach_targets_the_window_and_binds_ctrl_q_on_the_private_server() {
        let args = attach_args(&req(), true);
        let cmds = commands(&args);
        assert_eq!(
            cmds.last().unwrap(),
            &vec!["attach-session", "-t", "aiw-rs-hx-12:claude"]
        );
        assert!(cmds.contains(&vec!["bind-key", "-T", "root", "C-q", "detach-client"]));
        assert!(cmds.contains(&vec![
            "set-option",
            "-w",
            "-t",
            "aiw-rs-hx-12:claude",
            "window-size",
            "latest"
        ]));
        let right = cmds
            .iter()
            .find(|c| c.get(3) == Some(&"status-right"))
            .unwrap();
        assert!(right[4].contains("Ctrl+Q"));
        let left = cmds
            .iter()
            .find(|c| c.get(3) == Some(&"status-left"))
            .unwrap();
        assert!(left[4].contains("RS › hx-12 › claude"));
    }

    #[test]
    fn a_user_owned_server_keeps_its_key_tables() {
        let args = attach_args(&req(), false);
        let cmds = commands(&args);
        assert!(!cmds.iter().any(|c| c[0] == "bind-key"));
        let right = cmds
            .iter()
            .find(|c| c.get(3) == Some(&"status-right"))
            .unwrap();
        assert!(right[4].contains("prefix d"));
    }

    #[test]
    fn restore_puts_the_default_status_hint_back() {
        let cmds = restore_args(&req());
        assert_eq!(cmds.len(), 3);
        assert_eq!(cmds[0][1], "-u");
        assert_eq!(cmds[2][4], DEFAULT_STATUS_RIGHT);
    }
}
