//! FORK: background Claude sessions (DESIGN.md §19.7).
//!
//! A background session is a Claude conversation that runs under Claude
//! Code's own supervisor (`claude --bg`) instead of in a tmux pane. It keeps
//! working when the terminal, the cockpit or AIW itself closes; AIW only
//! starts it, shows its state from `claude agents --json` (the roster AIW
//! already reads, §19.2) and the session's own hooks, and attaches to it on
//! demand with `claude attach <id>` in a window of the feature's tmux
//! session that closes again when you detach.
//!
//! In the store it is an ordinary Claude session whose window name starts
//! with [`WINDOW_PREFIX`]: that window exists only while you are attached, a
//! feature start never launches anything into it, and the exited sweep
//! leaves it alone. The short id `claude --bg` prints (what `attach`, `logs`,
//! `stop`, `respawn` and `rm` take) is kept in `aiw_background_sessions`.
//!
//! Everything here is pure or a bounded child process: the app module
//! (`app::fork_background`) runs these off the UI thread.

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Map, Value};

/// Window names of background sessions start with this. The window exists
/// only while the session is attached.
pub const WINDOW_PREFIX: &str = "aiw-bg-";

/// How long `claude --bg` may take (it may start the supervisor first).
pub const LAUNCH_TIMEOUT: Duration = Duration::from_secs(30);
/// How long `stop`, `respawn`, `rm` and `logs` may take.
pub const OP_TIMEOUT: Duration = Duration::from_secs(15);
/// Lines of `claude logs` kept for the info block.
pub const LOG_LINES: usize = 3;

/// Whether `window` names a background session's (attach) window.
pub fn is_background_window(window: &str) -> bool {
    window.starts_with(WINDOW_PREFIX)
}

/// A free window name for a new background session: `aiw-bg-claude`,
/// `aiw-bg-claude-2`, … not in `taken`.
pub fn window_name<'a>(taken: impl IntoIterator<Item = &'a str>) -> String {
    let taken: Vec<&str> = taken.into_iter().collect();
    let base = format!("{WINDOW_PREFIX}claude");
    if !taken.contains(&base.as_str()) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|name| !taken.contains(&name.as_str()))
        .expect("an unbounded range always yields a free name")
}

/// What a background session is doing, from the roster entry (when Claude
/// lists it) and what AIW knows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BgState {
    /// `claude --bg` has not answered yet.
    Starting,
    /// A turn is running.
    Working,
    /// Waits on you: a question, a permission, an unstarted first prompt.
    Blocked(String),
    /// The last turn finished.
    Done,
    /// The session errored.
    Failed(String),
    /// Stopped (by you, or the supervisor after an idle hour); attaching
    /// resumes it.
    Stopped,
    /// Claude no longer lists it (removed, or its supervisor state is gone):
    /// `c` resumes the conversation in a new background session.
    Gone,
}

impl BgState {
    /// The state word for the row and the info block.
    pub fn word(&self) -> String {
        match self {
            BgState::Starting => "starting".into(),
            BgState::Working => "working".into(),
            BgState::Blocked(why) => format!("waiting: {why}"),
            BgState::Done => "done".into(),
            BgState::Failed(why) if why.is_empty() => "failed".into(),
            BgState::Failed(why) => format!("failed: {why}"),
            BgState::Stopped => "stopped".into(),
            BgState::Gone => "not running".into(),
        }
    }
}

/// The state of a session Claude lists as `entry`.
pub fn state_of(entry: &super::claude_roster::RosterEntry) -> BgState {
    if let Some(waiting) = entry.waiting_detail() {
        return BgState::Blocked(waiting);
    }
    match entry.state.as_deref() {
        Some("working") => BgState::Working,
        Some("blocked") => BgState::Blocked("input needed".into()),
        Some("done") => BgState::Done,
        Some("failed") => BgState::Failed(String::new()),
        Some("stopped") => BgState::Stopped,
        _ => match entry.status.as_deref() {
            Some("busy") => BgState::Working,
            Some("waiting") => BgState::Blocked("input needed".into()),
            _ => BgState::Done,
        },
    }
}

/// The short id in `claude --bg`'s output. Claude prints
/// `backgrounded <id>` and a block of `claude attach <id>` / `logs` / `stop`
/// commands; when it started a copy instead (the id was already running) the
/// note says `started a copy as <id>`, which wins.
pub fn parse_bg_output(out: &str) -> Option<String> {
    let token_after = |marker: &str| -> Option<String> {
        out.lines().rev().find_map(|line| {
            let start = line.find(marker)? + marker.len();
            let token: String = line[start..]
                .trim_start()
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            (!token.is_empty()).then_some(token)
        })
    };
    token_after("started a copy as ")
        .or_else(|| token_after("backgrounded "))
        .or_else(|| token_after("claude attach "))
}

/// The argument list of a new background session: `--bg`, AIW's usual
/// launch flags (`--settings`, `--name`, the mode, the context paragraph),
/// then the first prompt, if any. A prompt that starts with `-` follows `--`
/// so Claude does not read it as a flag.
pub fn launch_args(extra: Vec<String>, prompt: Option<&str>) -> Vec<String> {
    let mut args = vec!["--bg".to_string()];
    args.extend(extra);
    if let Some(prompt) = prompt.map(str::trim).filter(|p| !p.is_empty()) {
        if prompt.starts_with('-') {
            args.push("--".into());
        }
        args.push(prompt.to_string());
    }
    args
}

/// Continue a conversation Claude no longer lists, in place:
/// `--resume <conversation> --bg`. No other flags: with flags, Claude starts
/// a copy under a new id instead of continuing the session.
pub fn resume_args(conversation_id: &str) -> Vec<String> {
    vec![
        "--resume".into(),
        conversation_id.to_string(),
        "--bg".into(),
    ]
}

/// `claude <verb> <id>` for `stop`, `respawn`, `logs`.
pub fn verb_args(verb: &str, id: &str) -> Vec<String> {
    vec![verb.to_string(), id.to_string()]
}

/// `claude rm <id>` plus the flags a refusal asked for.
pub fn rm_args(id: &str, flags: &[String]) -> Vec<String> {
    let mut args = vec!["rm".to_string(), id.to_string()];
    args.extend(flags.iter().cloned());
    args
}

/// A `claude rm` that refused: Claude's own words and the flags it printed
/// for a second, discarding `rm`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RmRefusal {
    pub text: String,
    pub flags: Vec<String>,
}

/// Read a refusal from `claude rm <id>`'s output. Claude prints the exact
/// command to run again: `claude rm <id> --discard-unpushed <commit>@<wt>`
/// or `--force-remove-worktree <wt>`. `None` when the output names no such
/// flag (a plain failure).
pub fn parse_rm_refusal(out: &str) -> Option<RmRefusal> {
    let mut flags = Vec::new();
    let words: Vec<&str> = out
        .split(|c: char| c.is_whitespace() || c == '\'' || c == '"' || c == '`')
        .filter(|w| !w.is_empty())
        .collect();
    for (i, word) in words.iter().enumerate() {
        for flag in ["--discard-unpushed", "--force-remove-worktree"] {
            if let Some(value) = word.strip_prefix(&format!("{flag}=")) {
                push_flag(&mut flags, flag, value);
            } else if *word == flag
                && let Some(value) = words.get(i + 1)
                && !value.starts_with("--")
            {
                push_flag(&mut flags, flag, value.trim_end_matches(['.', ',', ')']));
            }
        }
    }
    if flags.is_empty() {
        return None;
    }
    let text = out
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    Some(RmRefusal { text, flags })
}

fn push_flag(flags: &mut Vec<String>, flag: &str, value: &str) {
    if value.is_empty() {
        return;
    }
    let pair = [flag.to_string(), value.to_string()];
    if !flags.windows(2).any(|w| w == pair) {
        flags.extend(pair);
    }
}

/// The last `n` non-empty lines of `claude logs`, without terminal escape
/// sequences.
pub fn log_tail(out: &str, n: usize) -> Vec<String> {
    let clean = strip_ansi(out);
    let lines: Vec<String> = clean
        .lines()
        .map(|l| l.trim_end().to_string())
        .filter(|l| !l.trim().is_empty())
        .collect();
    let skip = lines.len().saturating_sub(n);
    lines.into_iter().skip(skip).collect()
}

/// Remove CSI / OSC escape sequences and other control characters.
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            match chars.peek() {
                Some('[') => {
                    chars.next();
                    // CSI: parameters, then one final byte in @..~.
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    // OSC: up to BEL or ST.
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' {
                            break;
                        }
                        if c == '\u{1b}' && chars.peek() == Some(&'\\') {
                            chars.next();
                            break;
                        }
                    }
                }
                _ => {
                    chars.next();
                }
            }
            continue;
        }
        if c == '\r' {
            continue;
        }
        if c.is_control() && c != '\n' && c != '\t' {
            continue;
        }
        out.push(c);
    }
    out
}

use crate::fork::shell::quote as shell_quote;

/// What the attach window runs: clear the pane, `claude attach <id>`, then —
/// once you detach — close the window, so the cockpit's agent pane falls
/// back to its placeholder and the row returns to "in the background".
pub fn attach_shell_command(
    claude_bin: &str,
    id: &str,
    tmux_bin: &str,
    tmux_socket: Option<&Path>,
    tmux_session: &str,
    window: &str,
) -> String {
    let socket = tmux_socket
        .map(|s| format!(" -S {}", shell_quote(&s.to_string_lossy())))
        .unwrap_or_default();
    format!(
        "printf '\\033[2J\\033[H'; env -u TMUX {} attach {}; {}{} kill-window -t {}",
        shell_quote(claude_bin),
        shell_quote(id),
        shell_quote(tmux_bin),
        socket,
        shell_quote(&format!("={tmux_session}:{window}"))
    )
}

/// The environment variables AIW's hook scripts read, as the `env` block of
/// a background session's `--settings` file. A background session is
/// started by Claude's supervisor, not by the shell that ran `claude --bg`,
/// so these must not depend on environment inheritance.
pub fn settings_env(vars: &[(String, String)]) -> Map<String, Value> {
    vars.iter()
        .filter(|(k, v)| !k.is_empty() && !v.is_empty())
        .map(|(k, v)| (k.clone(), Value::String(v.clone())))
        .collect()
}

/// Add `env` and `"worktree": {"bgIsolation": "none"}` to the settings
/// file AIW wrote for the launch. The second keeps the session in the
/// checkout AIW gave it: Claude otherwise moves a background session into a
/// fresh worktree before its first edit unless it already runs in a linked
/// worktree (a feature in the repo dir is not).
pub fn amend_settings_file(path: &Path, env: &Map<String, Value>) -> std::io::Result<()> {
    let text = std::fs::read_to_string(path)?;
    let mut settings: Value = serde_json::from_str(&text)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let Some(obj) = settings.as_object_mut() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "settings file is not a JSON object",
        ));
    };
    let mut merged = obj
        .get("env")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for (k, v) in env {
        merged.insert(k.clone(), v.clone());
    }
    obj.insert("env".into(), Value::Object(merged));
    let mut worktree = obj
        .get("worktree")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    worktree.insert("bgIsolation".into(), Value::String("none".into()));
    obj.insert("worktree".into(), Value::Object(worktree));
    let rendered = serde_json::to_string_pretty(&settings)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?
        + "\n";
    let tmp = path.with_extension(format!("json.bg.{}", std::process::id()));
    std::fs::write(&tmp, rendered)?;
    std::fs::rename(&tmp, path)
}

/// The `--settings <path>` value in `args`, if any.
pub fn settings_path(args: &[String]) -> Option<&str> {
    args.windows(2)
        .find(|w| w[0] == "--settings")
        .map(|w| w[1].as_str())
}

/// What one `claude` run printed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutput {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

impl RunOutput {
    /// Both streams, stdout first.
    pub fn combined(&self) -> String {
        match (self.stdout.trim().is_empty(), self.stderr.trim().is_empty()) {
            (false, false) => format!("{}\n{}", self.stdout.trim_end(), self.stderr.trim_end()),
            (false, true) => self.stdout.clone(),
            _ => self.stderr.clone(),
        }
    }

    /// The last non-empty line, for a toast.
    pub fn last_line(&self) -> String {
        strip_ansi(&self.combined())
            .lines()
            .map(str::trim)
            .rfind(|l| !l.is_empty())
            .unwrap_or("")
            .to_string()
    }
}

/// Run `<claude> <args>` in `cwd` with `env` added, without a terminal,
/// for at most `timeout`. The command's own children (Claude's supervisor
/// when `--bg` starts it) may keep the pipes open after it exits, so once
/// the process itself has exited its output is read for a short grace and
/// then taken as is.
pub fn run(
    claude_bin: &str,
    args: &[String],
    cwd: &Path,
    env: &[(String, String)],
    timeout: Duration,
) -> Result<RunOutput, String> {
    let mut attempts = 0;
    let mut child = loop {
        let mut command = Command::new(claude_bin);
        command
            .args(args)
            .current_dir(cwd)
            .env_remove("TMUX")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in env {
            command.env(k, v);
        }
        match command.spawn() {
            Err(e) if e.kind() == std::io::ErrorKind::ExecutableFileBusy && attempts < 20 => {
                attempts += 1;
                std::thread::sleep(Duration::from_millis(10));
            }
            result => {
                break result.map_err(|e| format!("could not run {claude_bin}: {e}"))?;
            }
        }
    };
    let collect = |stream: Option<Box<dyn Read + Send>>| {
        let buf = Arc::new(Mutex::new(Vec::new()));
        if let Some(mut stream) = stream {
            let sink = Arc::clone(&buf);
            std::thread::spawn(move || {
                let mut chunk = [0u8; 4096];
                loop {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => sink
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .extend_from_slice(&chunk[..n]),
                    }
                }
            });
        }
        buf
    };
    let stdout = collect(
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    );
    let stderr = collect(
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    );
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{} timed out", args.join(" ")));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("{}: {e}", args.join(" ")));
            }
        }
    };
    // The readers finish at EOF; a daemon that inherited a pipe holds it
    // open, so give them a moment, then take what arrived.
    std::thread::sleep(Duration::from_millis(150));
    let take = |buf: &Arc<Mutex<Vec<u8>>>| {
        String::from_utf8_lossy(&buf.lock().unwrap_or_else(|e| e.into_inner())).into_owned()
    };
    Ok(RunOutput {
        success: status.success(),
        stdout: take(&stdout),
        stderr: take(&stderr),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::claude_roster::RosterEntry;

    #[test]
    fn window_names_are_prefixed_and_unique() {
        assert_eq!(window_name([]), "aiw-bg-claude");
        assert_eq!(
            window_name(["claude", "aiw-bg-claude", "aiw-bg-claude-2"]),
            "aiw-bg-claude-3"
        );
        assert!(is_background_window("aiw-bg-claude-2"));
        assert!(!is_background_window("claude"));
        assert!(!is_background_window("bg-claude"));
    }

    #[test]
    fn the_short_id_is_read_from_bg_output() {
        let out = "backgrounded 7c5dcf5d\n  claude agents         list sessions\n  claude attach 7c5dcf5d  open in this terminal\n  claude logs 7c5dcf5d    show recent output\n";
        assert_eq!(parse_bg_output(out).as_deref(), Some("7c5dcf5d"));
        // Only the command block (an older or reworded first line).
        assert_eq!(
            parse_bg_output("Started.\nRun `claude attach ab12cd34` to open it").as_deref(),
            Some("ab12cd34")
        );
        // A copy wins over the original it names.
        let copy = "note: session 11111111 is already running in the background, so this started a copy as 22222222. `claude attach 11111111` opens the original.\nbackgrounded 22222222";
        assert_eq!(parse_bg_output(copy).as_deref(), Some("22222222"));
        assert_eq!(parse_bg_output("error: not trusted"), None);
    }

    #[test]
    fn launch_and_resume_arguments() {
        let extra = vec!["--settings".into(), "/s.json".into()];
        assert_eq!(
            launch_args(extra.clone(), Some("  fix the test ")),
            ["--bg", "--settings", "/s.json", "fix the test"]
        );
        assert_eq!(
            launch_args(extra.clone(), Some("   ")),
            ["--bg", "--settings", "/s.json"]
        );
        assert_eq!(
            launch_args(Vec::new(), Some("-v is broken")),
            ["--bg", "--", "-v is broken"]
        );
        assert_eq!(resume_args("abc-123"), ["--resume", "abc-123", "--bg"]);
        assert_eq!(verb_args("stop", "7c5d"), ["stop", "7c5d"]);
        assert_eq!(
            rm_args("7c5d", &["--force-remove-worktree".into(), "wt1".into()]),
            ["rm", "7c5d", "--force-remove-worktree", "wt1"]
        );
        assert_eq!(
            settings_path(&[
                "--name".into(),
                "n".into(),
                "--settings".into(),
                "/p.json".into()
            ]),
            Some("/p.json")
        );
    }

    #[test]
    fn rm_refusals_name_their_flags() {
        let out = "Not removing 7c5d: the worktree has uncommitted changes, including src/a.rs.\nDeleting it would lose them. To discard them, run 'claude rm 7c5d --force-remove-worktree wt-9f2a'.";
        let refusal = parse_rm_refusal(out).unwrap();
        assert_eq!(refusal.flags, ["--force-remove-worktree", "wt-9f2a"]);
        assert!(refusal.text.contains("uncommitted changes"));

        let both =
            "run 'claude rm 7c5d --discard-unpushed 1a2b3c@wt-1 --force-remove-worktree=wt-1'";
        assert_eq!(
            parse_rm_refusal(both).unwrap().flags,
            [
                "--discard-unpushed",
                "1a2b3c@wt-1",
                "--force-remove-worktree",
                "wt-1"
            ]
        );
        assert_eq!(parse_rm_refusal("error: no such session 7c5d"), None);
        // The usage line names the flags in brackets, with placeholders: not a
        // refusal to act on.
        assert_eq!(
            parse_rm_refusal(
                "Usage: claude rm <id> [--discard-unpushed <commit>@<worktree-id>] [--force-remove-worktree <worktree-id>]"
            ),
            None
        );
    }

    #[test]
    fn roster_states_map_to_bg_states() {
        let entry = |state: &str, status: Option<&str>, waiting: Option<&str>| RosterEntry {
            cwd: "/w".into(),
            kind: "background".into(),
            state: Some(state.into()),
            status: status.map(str::to_string),
            waiting_for: waiting.map(str::to_string),
            ..Default::default()
        };
        assert_eq!(
            state_of(&entry("working", Some("busy"), None)),
            BgState::Working
        );
        assert_eq!(
            state_of(&entry(
                "blocked",
                Some("waiting"),
                Some("permission prompt")
            )),
            BgState::Blocked("permission prompt".into())
        );
        assert_eq!(state_of(&entry("done", Some("idle"), None)), BgState::Done);
        assert_eq!(
            state_of(&entry("failed", None, None)),
            BgState::Failed(String::new())
        );
        assert_eq!(state_of(&entry("stopped", None, None)), BgState::Stopped);
        assert_eq!(
            BgState::Blocked("input needed".into()).word(),
            "waiting: input needed"
        );
        assert_eq!(BgState::Gone.word(), "not running");
    }

    #[test]
    fn log_tails_drop_escapes_and_blank_lines() {
        let out = "\u{1b}[2J\u{1b}[H\u{1b}]0;title\u{7}line one\r\n\n\u{1b}[1;32mline two\u{1b}[0m\nline three\n   \n";
        assert_eq!(log_tail(out, 2), ["line two", "line three"]);
        assert_eq!(log_tail(out, 10), ["line one", "line two", "line three"]);
        assert!(log_tail("", 3).is_empty());
    }

    #[test]
    fn the_attach_command_closes_its_window_on_detach() {
        let cmd = attach_shell_command(
            "/bin/claude",
            "7c5d",
            "/usr/bin/tmux",
            Some(Path::new("/s/tmux.sock")),
            "aiw-rs-feat",
            "aiw-bg-claude",
        );
        assert!(
            cmd.contains("env -u TMUX '/bin/claude' attach '7c5d';"),
            "{cmd}"
        );
        assert!(
            cmd.ends_with(
                "'/usr/bin/tmux' -S '/s/tmux.sock' kill-window -t '=aiw-rs-feat:aiw-bg-claude'"
            ),
            "{cmd}"
        );
        let no_socket = attach_shell_command("c", "id", "tmux", None, "s", "w");
        assert!(
            no_socket.ends_with("'tmux' kill-window -t '=s:w'"),
            "{no_socket}"
        );
    }

    #[test]
    fn settings_files_gain_env_and_stay_in_their_checkout() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("s.json");
        std::fs::write(
            &path,
            r#"{"statusLine":{"type":"command"},"env":{"KEEP":"1"}}"#,
        )
        .unwrap();
        let env = settings_env(&[
            ("AIW_ACTIVE".into(), "1".into()),
            ("AIW_FEATURE_SESSION_ID".into(), "s-1".into()),
            ("EMPTY".into(), String::new()),
        ]);
        amend_settings_file(&path, &env).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["env"]["KEEP"], "1");
        assert_eq!(v["env"]["AIW_ACTIVE"], "1");
        assert_eq!(v["env"]["AIW_FEATURE_SESSION_ID"], "s-1");
        assert!(v["env"].get("EMPTY").is_none());
        assert_eq!(v["worktree"]["bgIsolation"], "none");
        assert_eq!(v["statusLine"]["type"], "command");
        assert!(amend_settings_file(&dir.path().join("missing.json"), &env).is_err());
    }

    #[test]
    fn run_reads_both_streams_and_reports_status() {
        let dir = tempfile::TempDir::new().unwrap();
        let bin = dir.path().join("fake");
        std::fs::write(
            &bin,
            "#!/bin/sh\necho \"out $1 $AIW_X\"\necho err >&2\n[ \"$1\" = ok ]\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let bin = bin.to_string_lossy().into_owned();
        let env = [("AIW_X".to_string(), "y".to_string())];
        let ok = run(&bin, &["ok".into()], dir.path(), &env, OP_TIMEOUT).unwrap();
        assert!(ok.success);
        assert_eq!(ok.stdout.trim(), "out ok y");
        assert_eq!(ok.stderr.trim(), "err");
        assert_eq!(ok.last_line(), "err");
        let bad = run(&bin, &["no".into()], dir.path(), &env, OP_TIMEOUT).unwrap();
        assert!(!bad.success);
        assert!(run("/no/such/claude", &[], dir.path(), &[], OP_TIMEOUT).is_err());
    }

    #[test]
    fn run_does_not_wait_for_a_daemon_holding_the_pipe() {
        let dir = tempfile::TempDir::new().unwrap();
        let bin = dir.path().join("fake");
        // A child that keeps stdout open for 20 s, like a supervisor started
        // by the first `claude --bg`.
        std::fs::write(
            &bin,
            "#!/bin/sh\necho backgrounded abc\n(sleep 20) &\nexit 0\n",
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let started = Instant::now();
        let out = run(&bin.to_string_lossy(), &[], dir.path(), &[], OP_TIMEOUT).unwrap();
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(parse_bg_output(&out.stdout).as_deref(), Some("abc"));
    }
}
