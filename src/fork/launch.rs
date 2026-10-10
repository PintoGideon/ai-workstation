//! FORK: what AIW adds to an agent's command line at launch (DESIGN.md §16).
//!
//! Every place that starts Claude or Codex (a new feature, a restart, adding
//! a session, resuming a saved conversation) goes through
//! [`claude_launch_args`] / [`codex_launch_args`], so the launch-time pieces
//! live in one file:
//!
//! - **Claude `--settings <file>`** (P4, item 7): a per-session settings file
//!   under the state dir (`claude-settings/<feature session id>.json`),
//!   rewritten at each launch, carrying AIW's `statusLine` command and — when
//!   `claude_hooks_via_settings_flag` is on — its hooks, so nothing has to be
//!   written into the repo's `.claude/settings.local.json`. Claude merges
//!   hook entries across settings levels (hooks.md: "Hook entries merge
//!   across settings levels rather than replacing each other"), so the
//!   user's own project hooks keep running.
//!
//! The session identity is also written to `agent-context/<id>.txt` for the
//! Gemini and Cursor `SessionStart` hooks (item 6), which cannot take a
//! command-line flag.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::config::AiwConfig;
use crate::project::{Feature, VibeMode};

/// Everything the launch-time pieces need to know about the session.
#[derive(Debug, Clone, PartialEq)]
pub struct LaunchContext {
    pub project_name: String,
    pub feature_name: String,
    pub branch: String,
    pub workdir: PathBuf,
    pub repo: PathBuf,
    pub feature_session_id: String,
    pub mode: VibeMode,
    pub is_worktree: bool,
}

impl LaunchContext {
    pub fn for_feature(
        project_name: &str,
        repo: &Path,
        feature: &Feature,
        session_id: &str,
    ) -> Self {
        Self {
            project_name: project_name.to_string(),
            feature_name: feature.name.clone(),
            branch: feature.branch.clone(),
            workdir: feature.workdir.clone(),
            repo: repo.to_path_buf(),
            feature_session_id: session_id.to_string(),
            mode: feature.mode.clone(),
            is_worktree: feature.is_worktree,
        }
    }
}

/// Where the per-session Claude settings files live.
pub fn claude_settings_dir() -> PathBuf {
    crate::ipc::state_dir().join("claude-settings")
}

/// The settings file for one feature session.
pub fn claude_settings_path(feature_session_id: &str) -> PathBuf {
    claude_settings_dir().join(format!("{feature_session_id}.json"))
}

/// The `statusLine` entry: AIW's script, which prints one line for Claude's
/// footer and sends the numbers to the dashboard.
pub fn status_line_entry(hooks_dir: &Path) -> Value {
    json!({
        "type": "command",
        "command": hooks_dir.join("status-line.sh").to_string_lossy(),
        "padding": 0
    })
}

/// The settings passed with `--settings`.
pub fn claude_settings_json(hooks_dir: &Path) -> Value {
    json!({ "statusLine": status_line_entry(hooks_dir) })
}

fn write_atomic(path: &Path, content: &str) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.tmp.{}",
        path.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("settings"),
        std::process::id()
    ));
    let result = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// Write the session's settings file and return the `--settings` argument.
fn write_claude_settings(ctx: &LaunchContext) -> Option<PathBuf> {
    let hooks_dir = crate::project::aiw_claude_hooks_dir();
    let settings = claude_settings_json(&hooks_dir);
    let path = claude_settings_path(&ctx.feature_session_id);
    let rendered = serde_json::to_string_pretty(&settings).ok()? + "\n";
    write_atomic(&path, &rendered).ok()?;
    Some(path)
}

/// Claude's extra arguments for `ctx`: `args` plus `--settings <file>`.
pub fn claude_launch_args(
    ctx: &LaunchContext,
    _config: &AiwConfig,
    mut args: Vec<String>,
) -> Vec<String> {
    if let Some(path) = write_claude_settings(ctx) {
        args.push("--settings".to_string());
        args.push(path.to_string_lossy().into_owned());
    }
    args
}

/// Codex's extra arguments for `ctx` (nothing yet beyond `args`).
pub fn codex_launch_args(
    _ctx: &LaunchContext,
    _config: &AiwConfig,
    args: Vec<String>,
) -> Vec<String> {
    args
}

/// Whether `args` is exactly AIW's `--settings <file>` pair and nothing else
/// (what a launch with no other flags produces).
pub fn only_settings_flag(args: &[String]) -> bool {
    args.len() == 2 && args[0] == "--settings" && args[1].ends_with(".json")
}

/// Remove the settings file of a session that is gone.
pub fn remove_claude_settings(feature_session_id: &str) {
    let _ = std::fs::remove_file(claude_settings_path(feature_session_id));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::AgentKind;

    fn ctx() -> LaunchContext {
        let feature = Feature::new(
            "hx-12".into(),
            "feature/hx-12".into(),
            PathBuf::from("/tmp/wt"),
            true,
            VibeMode::Vibe,
            false,
            false,
            AgentKind::Claude,
            false,
            false,
        );
        LaunchContext::for_feature("RS", Path::new("/tmp/repo"), &feature, "fs-1")
    }

    #[test]
    fn claude_gets_a_settings_file_with_the_status_line() {
        let args = claude_launch_args(&ctx(), &AiwConfig::default(), vec!["--chrome".into()]);
        assert_eq!(args[0], "--chrome");
        assert_eq!(args[1], "--settings");
        let path = PathBuf::from(&args[2]);
        assert_eq!(path, claude_settings_path("fs-1"));
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written["statusLine"]["type"], "command");
        assert!(
            written["statusLine"]["command"]
                .as_str()
                .unwrap()
                .ends_with("/hooks/status-line.sh")
        );
        // Rewritten at each launch, removed with the session.
        claude_launch_args(&ctx(), &AiwConfig::default(), Vec::new());
        assert!(path.exists());
        remove_claude_settings("fs-1");
        assert!(!path.exists());
    }

    #[test]
    fn the_context_carries_the_features_identity() {
        let ctx = ctx();
        assert_eq!(ctx.project_name, "RS");
        assert_eq!(ctx.feature_name, "hx-12");
        assert_eq!(ctx.branch, "feature/hx-12");
        assert_eq!(ctx.workdir, PathBuf::from("/tmp/wt"));
        assert!(ctx.is_worktree);
        assert_eq!(
            codex_launch_args(&ctx, &AiwConfig::default(), vec!["-C".into()]),
            vec!["-C"]
        );
    }
}
