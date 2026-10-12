//! Gemini CLI and Cursor agent as first-class AIW agents (DESIGN.md §5, G2).
//!
//! Everything agent-specific for the two lives in [`super::gemini`] and
//! [`super::cursor`]; this module is the single entry point AIW code calls
//! through its `// FORK:` hooks, so the match sites in AIW stay one line each.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result};

use crate::project::{AgentKind, VibeMode};
use crate::traits::TmuxOps;

/// The command line for an interactive agent session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSpec {
    pub program: String,
    pub args: Vec<String>,
}

/// `true` for the agents this module owns.
pub fn is_extra(agent: &AgentKind) -> bool {
    matches!(agent, AgentKind::Gemini | AgentKind::Cursor)
}

/// The command line for `agent` in `mode`, resuming `resume_id` when given.
/// `None` for agents this module does not own.
pub fn launch_spec(
    agent: &AgentKind,
    mode: &VibeMode,
    resume_id: Option<&str>,
    model: Option<&str>,
) -> Option<LaunchSpec> {
    let resume_id = resume_id.map(str::trim).filter(|id| !id.is_empty());
    let model = model.map(str::trim).filter(|m| !m.is_empty());
    match agent {
        AgentKind::Gemini => Some(LaunchSpec {
            program: super::gemini::BINARY.to_string(),
            args: super::gemini::launch_args(mode, resume_id, model),
        }),
        AgentKind::Cursor => Some(LaunchSpec {
            program: super::cursor::resolve_binary(&path_var()),
            args: super::cursor::launch_args(mode, resume_id, model),
        }),
        _ => None,
    }
}

/// Start `agent` in an already-created tmux window.
#[allow(clippy::too_many_arguments)]
pub fn launch(
    tmux: &dyn TmuxOps,
    agent: &AgentKind,
    session: &str,
    window: &str,
    feature_session_id: &str,
    mode: &VibeMode,
    resume_id: Option<&str>,
) -> Result<()> {
    let spec = launch_spec(agent, mode, resume_id, None)
        .with_context(|| format!("{} is not a Gemini/Cursor agent", agent.display_name()))?;
    tmux.launch_program(
        session,
        window,
        feature_session_id,
        &spec.program,
        spec.args,
    )
}

fn path_var() -> OsString {
    std::env::var_os("PATH").unwrap_or_default()
}

/// The first executable called `name` on `path_var`.
pub(crate) fn find_on_path(name: &str, path_var: &OsStr) -> Option<PathBuf> {
    std::env::split_paths(path_var)
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

/// A regular file with an execute bit (any file off unix).
pub(crate) fn is_executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Binary names to probe for `agent`, in preference order.
pub fn binary_candidates(agent: &AgentKind) -> &'static [&'static str] {
    match agent {
        AgentKind::Gemini => &[super::gemini::BINARY],
        AgentKind::Cursor => super::cursor::BINARIES,
        _ => &[],
    }
}

/// Whether `agent`'s CLI is installed: found on `PATH` and answers
/// `--version`. Drives the harness setup screen's Installed / not found.
pub fn check_available(agent: &AgentKind) -> Result<()> {
    check_available_in(agent, &path_var())
}

pub(crate) fn check_available_in(agent: &AgentKind, path_var: &OsStr) -> Result<()> {
    let candidates = binary_candidates(agent);
    let binary = candidates
        .iter()
        .find_map(|name| find_on_path(name, path_var))
        .with_context(|| {
            format!(
                "{} CLI not found (looked for {}) - is {} installed?",
                agent.display_name(),
                candidates.join(", "),
                agent.display_name()
            )
        })?;
    // A binary being written (an update in progress, or a file still open in
    // a just-forked child) fails with ETXTBSY for a moment; retry briefly.
    let mut attempts = 0;
    let output = loop {
        match Command::new(&binary).arg("--version").output() {
            Err(err) if err.kind() == std::io::ErrorKind::ExecutableFileBusy && attempts < 20 => {
                attempts += 1;
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            result => {
                break result.with_context(|| format!("could not run {}", binary.display()))?;
            }
        }
    };
    if !output.status.success() {
        anyhow::bail!("{} returned an error", binary.display());
    }
    Ok(())
}

pub fn install_hint(agent: &AgentKind) -> &'static str {
    match agent {
        AgentKind::Gemini => {
            "Install Gemini CLI: npm install -g @google/gemini-cli (needs an API key or paid plan)"
        }
        AgentKind::Cursor => "Install Cursor agent: curl https://cursor.com/install -fsS | bash",
        _ => "",
    }
}

/// Mode wording for the agents whose permission modes differ from Claude's.
/// AIW's Vibeless per-edit diff review is Claude-only, so for these
/// two Vibeless is the agent's own ask-before-acting mode.
pub fn mode_description(agent: &AgentKind, mode: &VibeMode) -> Option<&'static str> {
    Some(match (agent, mode) {
        (AgentKind::Gemini, VibeMode::Vibeless) => {
            "Gemini asks before edits and commands (no AIW diff review)"
        }
        (AgentKind::Gemini, VibeMode::Vibe) => "auto-accepts edits (--approval-mode auto_edit)",
        (AgentKind::Gemini, VibeMode::SuperVibe) => {
            "skips all permission prompts (--approval-mode yolo)"
        }
        (AgentKind::Cursor, VibeMode::Vibeless) => {
            "Cursor asks before commands (no AIW diff review)"
        }
        (AgentKind::Cursor, VibeMode::Vibe) => {
            "same as Vibeless: Cursor has no edits-only auto-approve"
        }
        (AgentKind::Cursor, VibeMode::SuperVibe) => "runs commands without asking (--force)",
        _ => return None,
    })
}

/// Write `agent`'s AIW status hooks into `workdir` (merged into whatever the
/// user already has) and keep the file out of git.
pub fn ensure_hooks(workdir: &Path, agent: &AgentKind) {
    let hooks_dir = crate::project::aiw_claude_hooks_dir();
    match agent {
        AgentKind::Gemini => super::gemini::ensure_settings(workdir, &hooks_dir),
        AgentKind::Cursor => super::cursor::ensure_hooks_file(workdir, &hooks_dir),
        _ => {}
    }
}

/// Remove the hooks [`ensure_hooks`] added, leaving the user's own entries.
pub fn cleanup_hooks(workdir: &Path, agent: &AgentKind) {
    let hooks_dir = crate::project::aiw_claude_hooks_dir();
    match agent {
        AgentKind::Gemini => super::gemini::cleanup_settings(workdir, &hooks_dir),
        AgentKind::Cursor => super::cursor::cleanup_hooks_file(workdir, &hooks_dir),
        _ => {}
    }
}

// ---- shared helpers for the two hook writers ---------------------------------

pub(crate) use crate::fork::shell::quote as shell_quote;

/// The shell command running AIW hook script `script` with `args`.
pub(crate) fn script_command(hooks_dir: &Path, script: &str, args: &[&str]) -> String {
    let mut command = shell_quote(&hooks_dir.join(script).to_string_lossy());
    for arg in args {
        command.push(' ');
        command.push_str(arg);
    }
    command
}

/// Whether `command` runs one of AIW's scripts from `hooks_dir`.
pub(crate) fn is_aiw_command(command: &str, hooks_dir: &Path) -> bool {
    let dir = hooks_dir.to_string_lossy();
    !dir.is_empty() && command.contains(dir.as_ref())
}

pub(crate) fn read_json_object(path: &Path) -> Option<serde_json::Value> {
    match std::fs::read_to_string(path) {
        Err(_) => Some(serde_json::json!({})),
        Ok(text) if text.trim().is_empty() => Some(serde_json::json!({})),
        // A file we cannot parse belongs to the user; never overwrite it.
        Ok(text) => serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .filter(serde_json::Value::is_object),
    }
}

pub(crate) fn write_json_if_changed(path: &Path, value: &serde_json::Value) {
    let rendered = serde_json::to_string_pretty(value).unwrap_or_default() + "\n";
    if std::fs::read_to_string(path).ok().as_deref() == Some(rendered.as_str()) {
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, rendered);
}

/// Whether `relative` is tracked by git in `checkout`. AIW must not put
/// machine-specific hook paths into a file the repository shares.
pub(crate) fn is_tracked(checkout: &Path, relative: &str) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(checkout)
        .args(["ls-files", "--error-unmatch", "--", relative])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Write a merged config file at `workdir/relative` unless git tracks it,
/// then add it to the repo's `info/exclude`.
pub(crate) fn write_managed_file(
    workdir: &Path,
    relative: &str,
    merge: impl FnOnce(serde_json::Value) -> serde_json::Value,
) {
    let path = workdir.join(relative);
    if is_tracked(workdir, relative) {
        return;
    }
    let Some(existing) = read_json_object(&path) else {
        return;
    };
    write_json_if_changed(&path, &merge(existing));
    let _ = super::git_exclude::ensure_excluded(workdir, &[relative]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_gemini_and_cursor_are_extra() {
        assert!(is_extra(&AgentKind::Gemini));
        assert!(is_extra(&AgentKind::Cursor));
        for agent in [AgentKind::Claude, AgentKind::Codex, AgentKind::Pi] {
            assert!(!is_extra(&agent));
            assert!(launch_spec(&agent, &VibeMode::Vibe, None, None).is_none());
        }
    }

    #[test]
    fn launch_spec_for_gemini_uses_gemini_binary() {
        let spec = launch_spec(
            &AgentKind::Gemini,
            &VibeMode::SuperVibe,
            Some(" abc "),
            None,
        )
        .unwrap();
        assert_eq!(spec.program, "gemini");
        assert_eq!(spec.args, ["--approval-mode", "yolo", "--resume", "abc"]);
    }

    #[test]
    fn blank_resume_and_model_are_ignored() {
        let spec = launch_spec(&AgentKind::Gemini, &VibeMode::Vibe, Some("  "), Some("")).unwrap();
        assert_eq!(spec.args, ["--approval-mode", "auto_edit"]);
    }

    fn fake_bin(dir: &Path, name: &str, exit: i32) {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\necho 1.0.0\nexit {exit}\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[test]
    fn missing_binaries_are_not_found() {
        let empty = tempfile::tempdir().unwrap();
        for agent in [AgentKind::Gemini, AgentKind::Cursor] {
            let err = check_available_in(&agent, empty.path().as_os_str()).unwrap_err();
            assert!(err.to_string().contains("not found"), "{err}");
            assert!(!install_hint(&agent).is_empty());
        }
    }

    #[cfg(unix)]
    #[test]
    fn detects_installed_binaries() {
        let dir = tempfile::tempdir().unwrap();
        fake_bin(dir.path(), "gemini", 0);
        fake_bin(dir.path(), "cursor-agent", 0);
        assert!(check_available_in(&AgentKind::Gemini, dir.path().as_os_str()).is_ok());
        // Only the legacy name is installed: still detected.
        assert!(check_available_in(&AgentKind::Cursor, dir.path().as_os_str()).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn a_failing_version_probe_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        fake_bin(dir.path(), "gemini", 1);
        assert!(check_available_in(&AgentKind::Gemini, dir.path().as_os_str()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn non_executable_files_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("gemini"), "x").unwrap();
        assert!(find_on_path("gemini", dir.path().as_os_str()).is_none());
    }

    #[test]
    fn vibeless_wording_says_there_is_no_aiw_diff_review() {
        for agent in [AgentKind::Gemini, AgentKind::Cursor] {
            let text = mode_description(&agent, &VibeMode::Vibeless).unwrap();
            assert!(text.contains("no AIW diff review"), "{text}");
        }
        assert!(mode_description(&AgentKind::Claude, &VibeMode::Vibeless).is_none());
    }

    #[test]
    fn script_commands_quote_the_path_and_are_recognised() {
        let hooks = Path::new("/home/u/.aiw/hooks");
        let cmd = script_command(hooks, "attention.sh", &["completed"]);
        assert_eq!(cmd, "'/home/u/.aiw/hooks/attention.sh' completed");
        assert!(is_aiw_command(&cmd, hooks));
        assert!(!is_aiw_command("./my-hook.sh", hooks));
    }

    #[test]
    fn unparseable_config_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert!(read_json_object(&path).is_none());
        assert_eq!(
            read_json_object(&dir.path().join("missing.json")),
            Some(serde_json::json!({}))
        );
    }
}
