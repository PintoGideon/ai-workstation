//! FORK: Claude Code's own session roster, `claude agents --json`
//! (DESIGN.md §19.2).
//!
//! The command prints every Claude session on the machine that Claude's
//! agent view knows about: interactive ones in any terminal, and background
//! ones under Claude's supervisor. It is the documented, stable interface
//! (`~/.claude/jobs/<id>/state.json` is explicitly not), it does not start
//! the supervisor, and it answers in about 0.1 s. AIW reads it on the status
//! sweep, off the UI thread, to:
//!
//! - show Claude sessions started outside AIW in a project's repo or one of
//!   its worktrees as dimmed, read-only lines (`E` lists them with the
//!   command to attach), and
//! - use `waitingFor` as a second needs-you signal for AIW's own Claude
//!   sessions when their hooks have not reported one.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Deserialize;

/// How long the command may run before it is killed.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(4);

/// One session from `claude agents --json`. Every field but `cwd` and `kind`
/// is optional: background sessions add `id` and `state`, live processes add
/// `pid` and `status`, waiting ones add `waitingFor`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RosterEntry {
    pub cwd: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub started_at: Option<i64>,
    /// Short id of a background session (`claude attach <id>`).
    #[serde(default)]
    pub id: Option<String>,
    /// Background state: working, blocked, done, failed, stopped.
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub pid: Option<u32>,
    /// Live process status: busy, waiting, idle.
    #[serde(default)]
    pub status: Option<String>,
    /// What a waiting session waits for: permission prompt, input needed, …
    #[serde(default)]
    pub waiting_for: Option<String>,
    /// The full conversation id, usable with `claude --resume`.
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

impl RosterEntry {
    pub fn is_background(&self) -> bool {
        self.kind == "background"
    }

    /// Whether the session waits on a person.
    pub fn needs_input(&self) -> bool {
        self.state.as_deref() == Some("blocked") || self.status.as_deref() == Some("waiting")
    }

    /// What the session waits for, when it waits.
    pub fn waiting_detail(&self) -> Option<String> {
        if !self.needs_input() {
            return None;
        }
        Some(
            self.waiting_for
                .clone()
                .filter(|w| !w.is_empty())
                .unwrap_or_else(|| "input needed".to_string()),
        )
    }

    /// A short state word for the dimmed line: the waiting reason, else the
    /// background state, else the process status.
    pub fn state_word(&self) -> String {
        if let Some(waiting) = self.waiting_detail() {
            return format!("waiting: {waiting}");
        }
        match (self.state.as_deref(), self.status.as_deref()) {
            (Some(state), _) if !state.is_empty() => state.to_string(),
            (_, Some("busy")) => "working".to_string(),
            (_, Some(status)) if !status.is_empty() => status.to_string(),
            _ => "running".to_string(),
        }
    }

    /// The handle a person would use: the name, else the short or full id.
    pub fn display_name(&self) -> String {
        self.name
            .clone()
            .filter(|n| !n.is_empty())
            .or_else(|| self.id.clone())
            .or_else(|| {
                self.session_id
                    .as_ref()
                    .map(|id| id.chars().take(8).collect())
            })
            .unwrap_or_else(|| "claude".to_string())
    }

    /// The command that opens the session here, when there is one: only a
    /// background session can be attached to from another terminal.
    pub fn attach_command(&self) -> Option<String> {
        if !self.is_background() {
            return None;
        }
        self.id
            .as_ref()
            .or(self.session_id.as_ref())
            .map(|id| format!("claude attach {id}"))
    }
}

/// Parse the command's output. A malformed entry is skipped rather than
/// losing the whole roster; output that is not a JSON array is empty.
pub fn parse(json: &str) -> Vec<RosterEntry> {
    let Ok(serde_json::Value::Array(items)) = serde_json::from_str::<serde_json::Value>(json)
    else {
        return Vec::new();
    };
    items
        .into_iter()
        .filter_map(|item| serde_json::from_value::<RosterEntry>(item).ok())
        .filter(|entry| !entry.cwd.is_empty())
        .collect()
}

/// Run `<claude> agents --json` with a timeout. `Err` when the binary is
/// missing, fails, or overruns; the caller keeps its previous roster.
pub fn fetch(claude_bin: &str, timeout: Duration) -> Result<Vec<RosterEntry>, String> {
    // A binary being replaced (an update in progress, or a file still open
    // in a just-forked child) fails with ETXTBSY for a moment; retry briefly.
    let mut attempts = 0;
    let mut child = loop {
        let spawned = Command::new(claude_bin)
            .args(["agents", "--json"])
            // Never inherit the dashboard's tmux: the command is not interactive.
            .env_remove("TMUX")
            .current_dir(std::env::temp_dir())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        match spawned {
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && attempts < 20 => {
                attempts += 1;
                std::thread::sleep(Duration::from_millis(10));
            }
            result => {
                break result
                    .map_err(|e| format!("could not run {claude_bin} agents --json: {e}"))?;
            }
        }
    };
    // Drain stdout while the command runs: a roster larger than the pipe
    // buffer would otherwise block `claude` until the timeout.
    let reader = child.stdout.take().map(|mut stdout| {
        std::thread::spawn(move || {
            let mut out = String::new();
            let _ = stdout.read_to_string(&mut out);
            out
        })
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("claude agents --json timed out".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("claude agents --json: {e}"));
            }
        }
    };
    let out = reader
        .and_then(|handle| handle.join().ok())
        .unwrap_or_default();
    if !status.success() {
        return Err(format!("claude agents --json exited with {status}"));
    }
    Ok(parse(&out))
}

/// Whether `cwd` is `root` or inside it, comparing canonical paths when they
/// resolve (worktrees are often reached through symlinks).
pub fn path_within(cwd: &Path, root: &Path) -> bool {
    if cwd.starts_with(root) {
        return true;
    }
    match (cwd.canonicalize(), root.canonicalize()) {
        (Ok(cwd), Ok(root)) => cwd.starts_with(root),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"[
      {"pid": 11, "cwd": "/repo", "kind": "interactive", "startedAt": 1,
       "sessionId": "aaaa-1", "name": "repo-9a", "status": "idle"},
      {"pid": 12, "cwd": "/repo/wt", "kind": "background", "startedAt": 2,
       "id": "7c5dcf5d", "state": "blocked", "status": "waiting",
       "waitingFor": "permission prompt", "sessionId": "bbbb-2", "name": "fix-tests"},
      {"cwd": "/other", "kind": "background", "id": "1234", "state": "done"},
      {"kind": "interactive"},
      "not an object"
    ]"#;

    #[test]
    fn parses_entries_and_skips_malformed_ones() {
        let roster = parse(SAMPLE);
        assert_eq!(roster.len(), 3);
        assert_eq!(roster[0].session_id.as_deref(), Some("aaaa-1"));
        assert_eq!(roster[1].waiting_for.as_deref(), Some("permission prompt"));
        assert!(roster[1].is_background());
        assert!(parse("garbage").is_empty());
        assert!(parse("{}").is_empty());
        assert!(parse("[]").is_empty());
    }

    #[test]
    fn state_words_and_waiting() {
        let roster = parse(SAMPLE);
        assert_eq!(roster[0].state_word(), "idle");
        assert!(!roster[0].needs_input());
        assert_eq!(roster[1].state_word(), "waiting: permission prompt");
        assert_eq!(
            roster[1].waiting_detail().as_deref(),
            Some("permission prompt")
        );
        assert_eq!(roster[2].state_word(), "done");
        let busy = RosterEntry {
            cwd: "/x".into(),
            status: Some("busy".into()),
            ..Default::default()
        };
        assert_eq!(busy.state_word(), "working");
        let blocked = RosterEntry {
            cwd: "/x".into(),
            state: Some("blocked".into()),
            ..Default::default()
        };
        assert_eq!(blocked.waiting_detail().as_deref(), Some("input needed"));
    }

    #[test]
    fn names_and_attach_commands() {
        let roster = parse(SAMPLE);
        assert_eq!(roster[0].display_name(), "repo-9a");
        assert_eq!(roster[0].attach_command(), None);
        assert_eq!(
            roster[1].attach_command().as_deref(),
            Some("claude attach 7c5dcf5d")
        );
        assert_eq!(roster[2].display_name(), "1234");
        let bare = RosterEntry {
            cwd: "/x".into(),
            session_id: Some("0123456789abcdef".into()),
            ..Default::default()
        };
        assert_eq!(bare.display_name(), "01234567");
    }

    #[test]
    fn paths_within_roots() {
        assert!(path_within(Path::new("/repo/wt/src"), Path::new("/repo")));
        assert!(path_within(Path::new("/repo"), Path::new("/repo")));
        assert!(!path_within(Path::new("/repo2"), Path::new("/repo")));
        let dir = tempfile::TempDir::new().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir_all(real.join("sub")).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        assert!(path_within(&link.join("sub"), &real));
    }

    #[test]
    fn fetch_reports_a_missing_binary_and_reads_a_fake_one() {
        assert!(fetch("/definitely/not/claude", Duration::from_millis(200)).is_err());
        let dir = tempfile::TempDir::new().unwrap();
        let fake = dir.path().join("claude");
        std::fs::write(
            &fake,
            "#!/bin/sh\n[ \"$1 $2\" = \"agents --json\" ] && echo '[{\"cwd\":\"/r\",\"kind\":\"interactive\"}]'\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let roster = fetch(fake.to_str().unwrap(), Duration::from_secs(5)).unwrap();
        assert_eq!(roster.len(), 1);
        let slow = dir.path().join("slow");
        std::fs::write(&slow, "#!/bin/sh\nsleep 5\n").unwrap();
        std::fs::set_permissions(&slow, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(fetch(slow.to_str().unwrap(), Duration::from_millis(150)).is_err());
        // A roster bigger than the pipe buffer still arrives whole.
        let big = dir.path().join("big");
        std::fs::write(
            &big,
            "#!/bin/sh\nprintf '['\ni=0\nwhile [ $i -lt 2000 ]; do printf '{\"cwd\":\"/r/%05d\",\"kind\":\"interactive\",\"name\":\"%0100d\"},' $i $i; i=$((i+1)); done\nprintf '{\"cwd\":\"/last\",\"kind\":\"interactive\"}]'\n",
        )
        .unwrap();
        std::fs::set_permissions(&big, std::fs::Permissions::from_mode(0o755)).unwrap();
        let roster = fetch(big.to_str().unwrap(), Duration::from_secs(20)).unwrap();
        assert_eq!(roster.len(), 2001);
    }
}
