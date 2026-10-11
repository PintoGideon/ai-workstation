//! FORK: Claude's hooks and status line through `--settings` (item 7,
//! DESIGN.md §16).
//!
//! AIW used to write its hook entries into the repo's
//! `.claude/settings.local.json`. With `claude_hooks_via_settings_flag` on
//! (the default) they are built here instead and passed at launch as
//! `--settings <state dir>/claude-settings/<feature session id>.json`,
//! together with the `statusLine` command, so the repo file only carries
//! what the user put there (plus the permissions AIW's diff review needs).
//! The user's own project hooks keep running: Claude merges hook entries
//! across settings levels rather than replacing them (hooks.md: "Hook
//! entries merge across settings levels rather than replacing each other";
//! settings.md: "Lists merge instead of overriding"; a handler defined in
//! two files runs once).
//!
//! The one builder, [`hook_entries`], serves both the legacy writer in
//! `app::setup` (flag off) and the launch file, so the two can never
//! disagree. Which mode is active is decided once at start-up
//! ([`set_via_settings_flag`]) from the config; a process that never
//! decided (unit tests) keeps the legacy writer, which the inherited tests
//! exercise.

use std::path::Path;
use std::sync::atomic::{AtomicU8, Ordering};

use serde_json::{Value, json};

use crate::app::setup::{
    claude_exec_hook, claude_exec_hook_with_args, claude_exec_hook_with_timeout,
};
use crate::project::VibeMode;

/// 0 = not decided (legacy), 1 = legacy writer, 2 = `--settings`.
static MODE: AtomicU8 = AtomicU8::new(0);

#[cfg(test)]
thread_local! {
    static TEST_OVERRIDE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// Decide the mode for this process (from `claude_hooks_via_settings_flag`).
pub fn set_via_settings_flag(on: bool) {
    MODE.store(if on { 2 } else { 1 }, Ordering::Relaxed);
}

/// Whether hooks go through `--settings` rather than `settings.local.json`.
pub fn via_settings_flag() -> bool {
    #[cfg(test)]
    if let Some(forced) = TEST_OVERRIDE.with(|cell| cell.get()) {
        return forced;
    }
    MODE.load(Ordering::Relaxed) == 2
}

/// Force the mode for the current test thread.
#[cfg(test)]
pub fn force_for_test(on: Option<bool>) {
    TEST_OVERRIDE.with(|cell| cell.set(on));
}

/// What the hook entries depend on.
#[derive(Debug, Clone)]
pub struct HookInputs<'a> {
    pub hooks_dir: &'a Path,
    pub workdir: &'a Path,
    pub mode: &'a VibeMode,
    /// The diff-review command for Vibeless mode, when one resolves.
    pub diff_review_cmd: Option<&'a str>,
}

/// AIW's Claude hook entries, `(event, entry)`, in the order they are
/// written.
pub fn hook_entries(inputs: &HookInputs<'_>) -> Vec<(&'static str, Value)> {
    let hooks_dir = inputs.hooks_dir;
    let notify_path = hooks_dir.join("notify.sh");
    let clear_path = hooks_dir.join("clear-notify.sh");
    let save_prompt_path = hooks_dir.join("save-prompt.sh");
    let thinking_start_path = hooks_dir.join("thinking-start.sh");
    let thinking_stop_path = hooks_dir.join("thinking-stop.sh");
    let tool_start_path = hooks_dir.join("tool-start.sh");
    let tool_stop_path = hooks_dir.join("tool-stop.sh");
    let attention_path = hooks_dir.join("attention.sh");
    let permission_request_path = hooks_dir.join("permission-request.sh");
    let stop_hook_path = hooks_dir.join("stop-hook.sh");
    let subagent_path = hooks_dir.join("subagent.sh");
    let hook_event_path = hooks_dir.join(super::hook_events::SCRIPT_NAME);
    let hook_event =
        |kind: &str| claude_exec_hook_with_timeout(&hook_event_path, &[kind, "claude"], 10);

    let wants_diff_review = matches!(inputs.mode, VibeMode::Vibeless);
    let diff_review_cmd = if wants_diff_review {
        inputs.diff_review_cmd
    } else {
        None
    };

    let mut entries: Vec<(&'static str, Value)> = Vec::new();

    // Stop: clear active thinking + write stop notification. The turn ending
    // is what "completed, awaiting review" means for Claude. The stop hook
    // delivers a queued follow-up (P7).
    entries.push((
        "Stop",
        json!({
            "matcher": "",
            "hooks": [
                claude_exec_hook(&thinking_stop_path),
                claude_exec_hook(&notify_path),
                claude_exec_hook_with_args(&attention_path, &["completed"]),
                hook_event("stop"),
                claude_exec_hook_with_timeout(&stop_hook_path, &["claude"], 30)
            ]
        }),
    ));

    // Notification: only the blocking ones become a question;
    // `attention.sh notification` classifies the payload.
    entries.push((
        "Notification",
        json!({
            "matcher": "",
            "hooks": [
                claude_exec_hook_with_args(&attention_path, &["notification"]),
                hook_event("notification")
            ]
        }),
    ));

    // PreToolUse: set thinking + tool-running + clear notification, plus
    // diff-review for vibeless mode.
    let mut pre_tool_hooks: Vec<Value> = vec![
        claude_exec_hook(&thinking_start_path),
        claude_exec_hook(&tool_start_path),
        claude_exec_hook(&clear_path),
    ];
    if let Some(dr_cmd) = diff_review_cmd {
        pre_tool_hooks.push(json!({
            "type": "command",
            "command": dr_cmd,
            "args": [inputs.workdir.to_string_lossy()],
            "timeout": 600
        }));
    }
    entries.push((
        "PreToolUse",
        json!({
            "matcher": if diff_review_cmd.is_some() { "Edit|Write" } else { "" },
            "hooks": pre_tool_hooks
        }),
    ));

    entries.push((
        "PostToolUse",
        json!({
            "matcher": "",
            "hooks": [claude_exec_hook(&tool_stop_path)]
        }),
    ));

    // UserPromptSubmit: set thinking + persist latest prompt.
    entries.push((
        "UserPromptSubmit",
        json!({
            "matcher": "",
            "hooks": [
                claude_exec_hook(&thinking_start_path),
                claude_exec_hook(&save_prompt_path)
            ]
        }),
    ));

    // SubagentStart / SubagentStop — the badge's subagent count (P5).
    for (event, arg) in [("SubagentStart", "start"), ("SubagentStop", "stop")] {
        entries.push((
            event,
            json!({
                "matcher": "",
                "hooks": [claude_exec_hook_with_timeout(&subagent_path, &[arg, "claude"], 10)]
            }),
        ));
    }

    // PermissionRequest — the dashboard answers the prompt (P6).
    entries.push((
        "PermissionRequest",
        json!({
            "matcher": "",
            "hooks": [
                claude_exec_hook_with_timeout(&permission_request_path, &["claude"], 600)
            ]
        }),
    ));

    // FORK: DESIGN.md §19.1 — the events that decorate the row.
    for (event, kind) in [
        ("StopFailure", "stop-failure"),
        ("PostCompact", "post-compact"),
        ("PostModelSwitch", "post-model-switch"),
        ("TaskCreated", "task-created"),
        ("TaskCompleted", "task-completed"),
        ("CwdChanged", "cwd-changed"),
        ("PermissionDenied", "permission-denied"),
        ("SessionEnd", "session-end"),
    ] {
        entries.push((
            event,
            json!({
                "matcher": "",
                "hooks": [hook_event(kind)]
            }),
        ));
    }

    entries
}

/// The `hooks` object for a settings file: every entry under its event.
pub fn hooks_object(entries: Vec<(&'static str, Value)>) -> Value {
    let mut hooks = serde_json::Map::new();
    for (event, entry) in entries {
        hooks
            .entry(event.to_string())
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .expect("array")
            .push(entry);
    }
    Value::Object(hooks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs<'a>(mode: &'a VibeMode, dr: Option<&'a str>) -> HookInputs<'a> {
        HookInputs {
            hooks_dir: Path::new("/home/u/.config/ai_workstation/hooks"),
            workdir: Path::new("/wt"),
            mode,
            diff_review_cmd: dr,
        }
    }

    #[test]
    fn every_event_is_wired_once_with_aiws_scripts() {
        let entries = hook_entries(&inputs(&VibeMode::Vibe, None));
        let events: Vec<&str> = entries.iter().map(|(e, _)| *e).collect();
        assert_eq!(
            events,
            [
                "Stop",
                "Notification",
                "PreToolUse",
                "PostToolUse",
                "UserPromptSubmit",
                "SubagentStart",
                "SubagentStop",
                "PermissionRequest",
                "StopFailure",
                "PostCompact",
                "PostModelSwitch",
                "TaskCreated",
                "TaskCompleted",
                "CwdChanged",
                "PermissionDenied",
                "SessionEnd"
            ]
        );
        let hooks = hooks_object(entries);
        assert_eq!(hooks["Stop"][0]["hooks"].as_array().unwrap().len(), 5);
        assert_eq!(
            hooks["Notification"][0]["hooks"].as_array().unwrap().len(),
            2
        );
        assert_eq!(
            hooks["PostCompact"][0]["hooks"][0]["args"][0],
            "post-compact"
        );
        assert_eq!(hooks["StopFailure"][0]["hooks"][0]["timeout"], 10);
        assert_eq!(hooks["PreToolUse"][0]["matcher"], "");
        assert_eq!(hooks["PermissionRequest"][0]["hooks"][0]["timeout"], 600);
        for entry in hooks.as_object().unwrap().values() {
            for hook in entry[0]["hooks"].as_array().unwrap() {
                assert!(
                    hook["command"]
                        .as_str()
                        .unwrap()
                        .starts_with("/home/u/.config/ai_workstation/hooks/")
                );
            }
        }
    }

    #[test]
    fn vibeless_adds_the_diff_review_hook_on_edits() {
        let entries = hook_entries(&inputs(&VibeMode::Vibeless, Some("/x/diff-review.sh")));
        let hooks = hooks_object(entries);
        assert_eq!(hooks["PreToolUse"][0]["matcher"], "Edit|Write");
        let last = hooks["PreToolUse"][0]["hooks"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()
            .clone();
        assert_eq!(last["command"], "/x/diff-review.sh");
        assert_eq!(last["args"][0], "/wt");
        // Vibe mode ignores a resolved command.
        let vibe = hooks_object(hook_entries(&inputs(
            &VibeMode::Vibe,
            Some("/x/diff-review.sh"),
        )));
        assert_eq!(vibe["PreToolUse"][0]["matcher"], "");
    }

    #[test]
    fn with_the_flag_on_the_repo_file_keeps_only_the_users_entries() {
        use crate::project::AgentKind;
        let dir = tempfile::tempdir().unwrap();
        let workdir = dir.path();
        std::fs::create_dir_all(workdir.join(".claude")).unwrap();
        let settings_path = workdir.join(".claude/settings.local.json");
        // An older AIW wrote its hooks here, next to the user's own entry.
        let legacy = crate::project::aiw_claude_hooks_dir().join("notify.sh");
        std::fs::write(
            &settings_path,
            serde_json::to_string(&json!({
                "hooks": {
                    "Stop": [
                        {"matcher": "", "hooks": [{"type": "command", "command": legacy, "args": []}]},
                        {"matcher": "", "hooks": [{"type": "command", "command": "./mine.sh"}]}
                    ]
                }
            }))
            .unwrap(),
        )
        .unwrap();

        force_for_test(Some(true));
        crate::app::setup::ensure_notification_hooks(
            workdir,
            workdir,
            &VibeMode::Vibe,
            &AgentKind::Claude,
            false,
        );
        force_for_test(None);

        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&settings_path).unwrap()).unwrap();
        let stop = written["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(
            stop.len(),
            1,
            "AIW's old entry is gone, the user's stays: {written}"
        );
        assert_eq!(stop[0]["hooks"][0]["command"], "./mine.sh");
        assert!(written["hooks"].get("PermissionRequest").is_none());

        // The legacy writer puts them back.
        force_for_test(Some(false));
        crate::app::setup::ensure_notification_hooks(
            workdir,
            workdir,
            &VibeMode::Vibe,
            &AgentKind::Claude,
            false,
        );
        force_for_test(None);
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&settings_path).unwrap()).unwrap();
        assert_eq!(written["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert!(written["hooks"]["PermissionRequest"].is_array());
    }

    #[test]
    fn the_mode_is_decided_once_and_tests_can_force_it() {
        force_for_test(Some(true));
        assert!(via_settings_flag());
        force_for_test(Some(false));
        assert!(!via_settings_flag());
        force_for_test(None);
    }
}
