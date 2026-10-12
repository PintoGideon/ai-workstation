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
    /// The `--name` Claude is launched with (DESIGN.md §19.2); `None` for
    /// harnesses without a launch-time name and for callers that have none.
    pub session_name: Option<String>,
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
            session_name: None,
        }
    }

    /// The same context with the session's harness name.
    pub fn with_session_name(mut self, name: Option<String>) -> Self {
        self.session_name = name;
        self
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

/// The settings passed with `--settings`: the status line, and — with
/// `claude_hooks_via_settings_flag` — AIW's hook entries (DESIGN.md §16.6).
pub fn claude_settings_json(hooks_dir: &Path, ctx: &LaunchContext) -> Value {
    let mut settings = json!({ "statusLine": status_line_entry(hooks_dir) });
    if super::claude_settings::via_settings_flag() {
        let diff_review_cmd = if matches!(ctx.mode, VibeMode::Vibeless) {
            crate::app::setup::resolve_diff_review_command(&ctx.workdir, &ctx.repo)
        } else {
            None
        };
        let entries = super::claude_settings::hook_entries(&super::claude_settings::HookInputs {
            hooks_dir,
            workdir: &ctx.workdir,
            mode: &ctx.mode,
            diff_review_cmd: diff_review_cmd.as_deref(),
        });
        settings["hooks"] = super::claude_settings::hooks_object(entries);
    }
    // §19.6: Claude's own worktrees (`--worktree`, subagent isolation,
    // workflows) are created in AIW's layout. Only ever through this
    // per-launch file — never the repo's settings — so a `claude` you start
    // yourself keeps Claude's default.
    if super::claude_worktrees::enabled() {
        if !settings["hooks"].is_object() {
            settings["hooks"] = json!({});
        }
        for (event, entry) in super::claude_worktrees::hook_entries(hooks_dir) {
            settings["hooks"][event] = Value::Array(vec![entry]);
        }
    }
    // §19.5: whether messages from your other sessions (and AIW's `@`) are
    // delivered, held for approval, or refused.
    if let Some(value) =
        super::messaging::settings_value(super::messaging::inbound_policy(), &ctx.mode)
    {
        settings["crossSessionInbound"] = Value::String(value.into());
    }
    settings
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
    if super::claude_worktrees::enabled() {
        super::claude_worktrees::ensure_script(&hooks_dir);
    }
    let settings = claude_settings_json(&hooks_dir, ctx);
    let path = claude_settings_path(&ctx.feature_session_id);
    let rendered = serde_json::to_string_pretty(&settings).ok()? + "\n";
    write_atomic(&path, &rendered).ok()?;
    Some(path)
}

/// Claude's extra arguments for `ctx`: `args` plus `--settings <file>`.
pub fn claude_launch_args(
    ctx: &LaunchContext,
    config: &AiwConfig,
    mut args: Vec<String>,
) -> Vec<String> {
    if let Some(path) = write_claude_settings(ctx) {
        args.push("--settings".to_string());
        args.push(path.to_string_lossy().into_owned());
    }
    // FORK: §19.2 — the session's stable name, so `claude --resume <name>`,
    // agent view and other sessions see AIW's handle.
    if let Some(name) = ctx.session_name.as_deref() {
        args = super::session_identity::with_claude_name(args, name);
    }
    // Unless `agent_context_prompt` is off, the context paragraph goes into
    // `--append-system-prompt` (after any earlier appended text).
    if config.agent_context_prompt {
        args = super::agent_context::claude_args(ctx, args);
    }
    args
}

/// Codex's extra arguments for `ctx`: the context paragraph in
/// `developer_instructions`, unless `agent_context_prompt` is off.
pub fn codex_launch_args(
    ctx: &LaunchContext,
    config: &AiwConfig,
    args: Vec<String>,
) -> Vec<String> {
    if config.agent_context_prompt {
        super::agent_context::codex_args(ctx, args)
    } else {
        args
    }
}

/// Whether `args` is exactly what AIW adds to a launch with no other flags:
/// `--settings <file>`, optionally `--name <name>` (§19.2), and the context
/// paragraph.
pub fn only_settings_flag(args: &[String]) -> bool {
    let settings = args.len() >= 2 && args[0] == "--settings" && args[1].ends_with(".json");
    if !settings {
        return false;
    }
    let mut rest = &args[2..];
    if rest.len() >= 2 && rest[0] == "--name" && !rest[1].is_empty() {
        rest = &rest[2..];
    }
    rest.is_empty()
        || (rest.len() == 2
            && rest[0] == "--append-system-prompt"
            && rest[1].starts_with(super::agent_context::MARKER))
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
            AgentKind::Claude,
            false,
        );
        LaunchContext::for_feature("RS", Path::new("/tmp/repo"), &feature, "fs-1")
    }

    #[test]
    fn claude_gets_a_settings_file_with_the_status_line() {
        let args = claude_launch_args(&ctx(), &AiwConfig::default(), vec!["--chrome".into()]);
        assert_eq!(args[0], "--chrome");
        assert_eq!(args[1], "--settings");
        assert_eq!(args[3], "--append-system-prompt");
        assert!(args[4].starts_with("AIW context:"));
        let quiet = AiwConfig {
            agent_context_prompt: false,
            ..AiwConfig::default()
        };
        assert_eq!(claude_launch_args(&ctx(), &quiet, Vec::new()).len(), 2);
        assert!(only_settings_flag(&args[1..]));
        assert!(only_settings_flag(&claude_launch_args(
            &ctx(),
            &quiet,
            Vec::new()
        )));
        assert!(!only_settings_flag(&args));
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
        // The legacy writer keeps AIW's hooks in the repo file; only the
        // §19.6 worktree hooks (never written there) ride along.
        let mut events: Vec<&str> = written["hooks"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        events.sort_unstable();
        assert_eq!(events, ["WorktreeCreate", "WorktreeRemove"]);
        // With the flag on, the hooks ride along.
        super::super::claude_settings::force_for_test(Some(true));
        let with_hooks = claude_settings_json(Path::new("/h"), &ctx());
        super::super::claude_settings::force_for_test(None);
        assert!(with_hooks["hooks"]["Stop"].is_array());
        assert!(with_hooks["hooks"]["PermissionRequest"].is_array());
        assert_eq!(with_hooks["statusLine"]["type"], "command");
        // Rewritten at each launch, removed with the session.
        claude_launch_args(&ctx(), &AiwConfig::default(), Vec::new());
        assert!(path.exists());
        remove_claude_settings("fs-1");
        assert!(!path.exists());
    }

    #[test]
    fn the_inbound_message_rule_follows_the_policy_and_the_mode() {
        use crate::fork::messaging::{InboundPolicy, force_policy_for_test};
        force_policy_for_test(Some(InboundPolicy::Accept));
        assert_eq!(
            claude_settings_json(Path::new("/h"), &ctx())["crossSessionInbound"],
            "accept"
        );
        let mut bypass = ctx();
        bypass.mode = VibeMode::SuperVibe;
        assert!(
            claude_settings_json(Path::new("/h"), &bypass)
                .get("crossSessionInbound")
                .is_none(),
            "a bypass session keeps Claude's own rule"
        );
        force_policy_for_test(Some(InboundPolicy::Refuse));
        assert_eq!(
            claude_settings_json(Path::new("/h"), &bypass)["crossSessionInbound"],
            "refuse"
        );
        force_policy_for_test(Some(InboundPolicy::Default));
        assert!(
            claude_settings_json(Path::new("/h"), &ctx())
                .get("crossSessionInbound")
                .is_none()
        );
        force_policy_for_test(None);
    }

    #[test]
    fn a_named_session_gets_one_name_flag_and_still_counts_as_aiws_own_args() {
        let quiet = AiwConfig {
            agent_context_prompt: false,
            ..AiwConfig::default()
        };
        let mut ctx = ctx().with_session_name(Some("hx-12-claude-1".into()));
        // Its own settings file: the other test removes `fs-1`'s.
        ctx.feature_session_id = "fs-named".into();
        let args = claude_launch_args(&ctx, &quiet, Vec::new());
        assert_eq!(&args[2..], ["--name", "hx-12-claude-1"]);
        assert!(only_settings_flag(&args));
        let full = claude_launch_args(&ctx, &AiwConfig::default(), Vec::new());
        assert!(only_settings_flag(&full));
        assert_eq!(full.iter().filter(|a| *a == "--name").count(), 1);
        // A user's own flag is not AIW's.
        let mut extra = args.clone();
        extra.push("--chrome".into());
        assert!(!only_settings_flag(&extra));
        // An explicit name in the args wins.
        let mine = claude_launch_args(&ctx, &quiet, vec!["-n".into(), "mine".into()]);
        assert_eq!(
            mine.iter().filter(|a| *a == "--name" || *a == "-n").count(),
            1
        );
        remove_claude_settings("fs-named");
    }

    #[test]
    fn claudes_worktree_hooks_ride_along_unless_turned_off() {
        // §19.6: with AIW's hooks in the file or in the repo, the worktree
        // hooks always come through `--settings` (never the repo file).
        for via_flag in [true, false] {
            super::super::claude_settings::force_for_test(Some(via_flag));
            super::super::claude_worktrees::force_for_test(Some(true));
            let settings = claude_settings_json(Path::new("/h"), &ctx());
            let create = &settings["hooks"]["WorktreeCreate"][0]["hooks"][0];
            assert_eq!(create["command"], "/h/worktree-hook.sh");
            assert_eq!(create["args"][0], "create");
            assert_eq!(
                settings["hooks"]["WorktreeRemove"][0]["hooks"][0]["args"][0],
                "remove"
            );
            if via_flag {
                assert!(settings["hooks"]["Stop"].is_array());
            }
            super::super::claude_worktrees::force_for_test(Some(false));
            let off = claude_settings_json(Path::new("/h"), &ctx());
            assert!(off["hooks"].get("WorktreeCreate").is_none());
            assert!(off["hooks"].get("WorktreeRemove").is_none());
        }
        super::super::claude_settings::force_for_test(None);
        super::super::claude_worktrees::force_for_test(None);
        // A launch installs the script next to AIW's other hook scripts. Its
        // own session id: `claude_gets_a_settings_file_with_the_status_line`
        // creates and removes `fs-1`'s settings file in parallel.
        let wt_ctx = LaunchContext {
            feature_session_id: "fs-wt-hooks".into(),
            ..ctx()
        };
        claude_launch_args(&wt_ctx, &AiwConfig::default(), Vec::new());
        let script = crate::project::aiw_claude_hooks_dir()
            .join(super::super::claude_worktrees::SCRIPT_NAME);
        assert_eq!(
            std::fs::read_to_string(&script).unwrap(),
            super::super::claude_worktrees::SCRIPT
        );
        remove_claude_settings("fs-wt-hooks");
    }

    #[test]
    fn the_context_carries_the_features_identity() {
        let ctx = ctx();
        assert_eq!(ctx.project_name, "RS");
        assert_eq!(ctx.feature_name, "hx-12");
        assert_eq!(ctx.branch, "feature/hx-12");
        assert_eq!(ctx.workdir, PathBuf::from("/tmp/wt"));
        assert!(ctx.is_worktree);
        let quiet = AiwConfig {
            agent_context_prompt: false,
            ..AiwConfig::default()
        };
        assert_eq!(
            codex_launch_args(&ctx, &quiet, vec!["-C".into()]),
            vec!["-C"]
        );
        assert!(
            codex_launch_args(&ctx, &AiwConfig::default(), vec!["-C".into()])
                .iter()
                .any(|a| a.contains("AIW context:"))
        );
    }
}
