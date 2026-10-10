//! Gemini CLI (`gemini`) — launch flags and status hooks.
//!
//! Verified against google-gemini/gemini-cli `docs/cli/cli-reference.md`,
//! `docs/cli/session-management.md`, `docs/hooks/reference.md` and
//! `docs/reference/configuration.md`:
//! - `--approval-mode default|auto_edit|yolo`, `-m/--model`,
//!   `-r/--resume <uuid|latest|index>`.
//! - Hooks live under top-level `"hooks"` in `.gemini/settings.json`:
//!   `{"<Event>": [{"matcher"?, "hooks": [{"name", "type": "command",
//!   "command", "timeout" (ms)}]}]}`; `matcher` is optional and omitting it
//!   matches everything. Hooks inherit Gemini's environment, so the `AIW_*`
//!   variables AIW exports reach the scripts.
//! - Hook stdin carries `session_id` (a UUID, the id `--resume` takes),
//!   `transcript_path`, `cwd`, `hook_event_name`; `Notification` adds
//!   `notification_type` (`ToolPermission`) and `message`.
//! - Empty stdout with exit 0 is a no-op, which is what AIW's scripts produce.
//! - `context.fileName` accepts a string or an array of strings.

use std::path::Path;

use serde_json::{Value, json};

use super::extra_agents::{is_aiw_command, script_command, write_managed_file};
use crate::project::VibeMode;

pub const BINARY: &str = "gemini";

/// The settings file AIW merges its hooks into, relative to the checkout.
pub const SETTINGS_FILE: &str = ".gemini/settings.json";

/// Instruction files Gemini should load, so it reads what the other agents do.
pub const CONTEXT_FILES: [&str; 3] = ["AGENTS.md", "GEMINI.md", "CLAUDE.md"];

/// Per-hook timeout, in milliseconds (Gemini's unit).
const HOOK_TIMEOUT_MS: u64 = 10_000;

/// Prefix on every hook name AIW writes, so they can be found again.
const NAME_PREFIX: &str = "aiw-";

pub fn approval_mode(mode: &VibeMode) -> &'static str {
    match mode {
        VibeMode::Vibeless => "default",
        VibeMode::Vibe => "auto_edit",
        VibeMode::SuperVibe => "yolo",
    }
}

pub fn launch_args(mode: &VibeMode, resume_id: Option<&str>, model: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "--approval-mode".to_string(),
        approval_mode(mode).to_string(),
    ];
    if let Some(model) = model {
        args.extend(["--model".to_string(), model.to_string()]);
    }
    if let Some(id) = resume_id {
        args.extend(["--resume".to_string(), id.to_string()]);
    }
    args
}

/// `(event, [(hook name, script, script args)])` — what AIW wires.
///
/// - `BeforeAgent` (prompt submitted) → working, prompt saved.
/// - `BeforeTool` / `AfterTool` → tool running / done.
/// - `AfterAgent` (turn finished) → idle + Completed.
/// - `Notification` (`ToolPermission`) → Question, via the same payload
///   classifier Claude's `Notification` uses.
/// - `SessionStart` clears a stale notification; `SessionEnd` stops the
///   working indicator.
type HookSpec = (&'static str, &'static str, &'static [&'static str]);

pub const EVENTS: &[(&str, &[HookSpec])] = &[
    (
        "SessionStart",
        &[
            ("aiw-clear-notify", "clear-notify.sh", &[]),
            // The launch context as additional context (item 6).
            ("aiw-session-context", "session-context.sh", &["gemini"]),
        ],
    ),
    (
        "BeforeAgent",
        &[
            ("aiw-thinking-start", "thinking-start.sh", &[]),
            ("aiw-save-prompt", "save-prompt.sh", &[]),
            ("aiw-clear-notify", "clear-notify.sh", &[]),
        ],
    ),
    (
        "BeforeTool",
        &[
            ("aiw-thinking-start", "thinking-start.sh", &[]),
            ("aiw-tool-start", "tool-start.sh", &[]),
        ],
    ),
    ("AfterTool", &[("aiw-tool-stop", "tool-stop.sh", &[])]),
    (
        "AfterAgent",
        &[
            ("aiw-thinking-stop", "thinking-stop.sh", &[]),
            ("aiw-notify", "notify.sh", &[]),
            ("aiw-attention", "attention.sh", &["completed"]),
            // A queued follow-up becomes the next turn (P7).
            ("aiw-stop-hook", "stop-hook.sh", &["gemini"]),
        ],
    ),
    (
        "Notification",
        &[("aiw-attention", "attention.sh", &["notification"])],
    ),
    (
        "SessionEnd",
        &[("aiw-thinking-stop", "thinking-stop.sh", &[])],
    ),
];

fn is_aiw_hook(hook: &Value, hooks_dir: &Path) -> bool {
    let named = hook
        .get("name")
        .and_then(Value::as_str)
        .is_some_and(|name| name.starts_with(NAME_PREFIX));
    let command = hook
        .get("command")
        .and_then(Value::as_str)
        .is_some_and(|command| is_aiw_command(command, hooks_dir));
    named && command
}

/// Drop every AIW hook from `settings.hooks`, keeping the user's. Groups and
/// events left empty by the removal are dropped too.
pub fn remove_aiw_hooks(settings: &mut Value, hooks_dir: &Path) {
    let Some(hooks) = settings.get_mut("hooks").and_then(Value::as_object_mut) else {
        return;
    };
    for groups in hooks.values_mut() {
        let Some(groups) = groups.as_array_mut() else {
            continue;
        };
        for group in groups.iter_mut() {
            if let Some(inner) = group.get_mut("hooks").and_then(Value::as_array_mut) {
                inner.retain(|hook| !is_aiw_hook(hook, hooks_dir));
            }
        }
        groups.retain(|group| {
            group
                .get("hooks")
                .and_then(Value::as_array)
                .is_none_or(|inner| !inner.is_empty())
        });
    }
    hooks.retain(|_, groups| groups.as_array().is_none_or(|groups| !groups.is_empty()));
    if hooks.is_empty()
        && let Some(map) = settings.as_object_mut()
    {
        map.remove("hooks");
    }
}

/// Make sure `context.fileName` names every file in [`CONTEXT_FILES`],
/// keeping whatever the user listed first.
fn merge_context_files(settings: &mut Value) {
    let Some(root) = settings.as_object_mut() else {
        return;
    };
    let context = root.entry("context").or_insert_with(|| json!({}));
    if !context.is_object() {
        *context = json!({});
    }
    let context = context.as_object_mut().expect("object");
    let mut names: Vec<Value> = match context.get("fileName") {
        Some(Value::String(name)) => vec![Value::String(name.clone())],
        Some(Value::Array(list)) => list.clone(),
        _ => Vec::new(),
    };
    for file in CONTEXT_FILES {
        if !names.iter().any(|name| name.as_str() == Some(file)) {
            names.push(Value::String(file.to_string()));
        }
    }
    context.insert("fileName".to_string(), Value::Array(names));
}

/// `settings` with AIW's hooks (re)installed and the context files set.
/// Everything else in the file is preserved.
pub fn merge_settings(mut settings: Value, hooks_dir: &Path) -> Value {
    if !settings.is_object() {
        settings = json!({});
    }
    remove_aiw_hooks(&mut settings, hooks_dir);
    merge_context_files(&mut settings);

    let root = settings.as_object_mut().expect("object");
    let hooks = root.entry("hooks").or_insert_with(|| json!({}));
    if !hooks.is_object() {
        *hooks = json!({});
    }
    let hooks = hooks.as_object_mut().expect("object");
    for (event, specs) in EVENTS {
        let entries: Vec<Value> = specs
            .iter()
            .map(|(name, script, args)| {
                json!({
                    "name": name,
                    "type": "command",
                    "command": script_command(hooks_dir, script, args),
                    "timeout": HOOK_TIMEOUT_MS,
                })
            })
            .collect();
        let groups = hooks
            .entry(event.to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
        if !groups.is_array() {
            *groups = Value::Array(Vec::new());
        }
        groups
            .as_array_mut()
            .expect("array")
            .push(json!({ "hooks": entries }));
    }
    settings
}

pub fn ensure_settings(workdir: &Path, hooks_dir: &Path) {
    write_managed_file(workdir, SETTINGS_FILE, |existing| {
        merge_settings(existing, hooks_dir)
    });
}

pub fn cleanup_settings(workdir: &Path, hooks_dir: &Path) {
    let path = workdir.join(SETTINGS_FILE);
    let Some(mut settings) = super::extra_agents::read_json_object(&path) else {
        return;
    };
    if !path.exists() {
        return;
    }
    remove_aiw_hooks(&mut settings, hooks_dir);
    super::extra_agents::write_json_if_changed(&path, &settings);
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOOKS: &str = "/home/u/.aiw/hooks";

    fn hooks_dir() -> &'static Path {
        Path::new(HOOKS)
    }

    #[test]
    fn approval_modes_follow_vibe_modes() {
        assert_eq!(
            launch_args(&VibeMode::Vibeless, None, None),
            ["--approval-mode", "default"]
        );
        assert_eq!(
            launch_args(&VibeMode::Vibe, None, None),
            ["--approval-mode", "auto_edit"]
        );
        assert_eq!(
            launch_args(&VibeMode::SuperVibe, None, None),
            ["--approval-mode", "yolo"]
        );
    }

    #[test]
    fn resume_and_model_flags() {
        assert_eq!(
            launch_args(&VibeMode::Vibe, Some("1234-uuid"), Some("pro")),
            [
                "--approval-mode",
                "auto_edit",
                "--model",
                "pro",
                "--resume",
                "1234-uuid"
            ]
        );
        assert_eq!(
            launch_args(&VibeMode::Vibeless, Some("latest"), None),
            ["--approval-mode", "default", "--resume", "latest"]
        );
    }

    #[test]
    fn fresh_settings_get_every_event_and_context_files() {
        let settings = merge_settings(json!({}), hooks_dir());
        for event in [
            "SessionStart",
            "BeforeAgent",
            "AfterAgent",
            "BeforeTool",
            "AfterTool",
            "Notification",
            "SessionEnd",
        ] {
            let groups = settings["hooks"][event].as_array().unwrap();
            assert_eq!(groups.len(), 1, "{event}");
            for hook in groups[0]["hooks"].as_array().unwrap() {
                assert_eq!(hook["type"], "command");
                assert_eq!(hook["timeout"], HOOK_TIMEOUT_MS);
                assert!(hook["name"].as_str().unwrap().starts_with("aiw-"));
                assert!(
                    hook["command"]
                        .as_str()
                        .unwrap()
                        .starts_with("'/home/u/.aiw/hooks/")
                );
            }
        }
        assert_eq!(
            settings["context"]["fileName"],
            json!(["AGENTS.md", "GEMINI.md", "CLAUDE.md"])
        );
        let after_agent = &settings["hooks"]["AfterAgent"][0]["hooks"];
        assert_eq!(
            after_agent[2]["command"],
            "'/home/u/.aiw/hooks/attention.sh' completed"
        );
        assert_eq!(
            settings["hooks"]["Notification"][0]["hooks"][0]["command"],
            "'/home/u/.aiw/hooks/attention.sh' notification"
        );
    }

    #[test]
    fn merging_keeps_user_settings_and_hooks() {
        let existing = json!({
            "model": {"name": "gemini-2.5-pro"},
            "context": {"fileName": "MY.md"},
            "hooks": {
                "BeforeTool": [
                    {"matcher": "write_file", "hooks": [
                        {"name": "security", "type": "command", "command": "./sec.sh"}
                    ]}
                ],
                "PreCompress": [{"hooks": [{"type": "command", "command": "./c.sh"}]}]
            }
        });
        let merged = merge_settings(existing, hooks_dir());

        assert_eq!(merged["model"]["name"], "gemini-2.5-pro");
        assert_eq!(
            merged["context"]["fileName"],
            json!(["MY.md", "AGENTS.md", "GEMINI.md", "CLAUDE.md"])
        );
        let before_tool = merged["hooks"]["BeforeTool"].as_array().unwrap();
        assert_eq!(before_tool.len(), 2);
        assert_eq!(before_tool[0]["hooks"][0]["name"], "security");
        assert_eq!(
            merged["hooks"]["PreCompress"][0]["hooks"][0]["command"],
            "./c.sh"
        );
    }

    #[test]
    fn merging_twice_is_idempotent() {
        let once = merge_settings(json!({"hooks": {}}), hooks_dir());
        let twice = merge_settings(once.clone(), hooks_dir());
        assert_eq!(once, twice);
    }

    #[test]
    fn removal_keeps_user_hooks_sharing_a_group() {
        let mut settings = merge_settings(json!({}), hooks_dir());
        settings["hooks"]["AfterTool"][0]["hooks"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name": "mine", "type": "command", "command": "./log.sh"}));
        remove_aiw_hooks(&mut settings, hooks_dir());
        assert_eq!(
            settings["hooks"],
            json!({"AfterTool": [{"hooks": [{"name": "mine", "type": "command", "command": "./log.sh"}]}]})
        );
    }

    #[test]
    fn a_user_hook_named_like_ours_but_elsewhere_is_kept() {
        let mut settings = json!({"hooks": {"AfterAgent": [{"hooks": [
            {"name": "aiw-custom", "type": "command", "command": "./mine.sh"}
        ]}]}});
        remove_aiw_hooks(&mut settings, hooks_dir());
        assert_eq!(
            settings["hooks"]["AfterAgent"][0]["hooks"][0]["name"],
            "aiw-custom"
        );
    }

    #[test]
    fn writes_merges_and_cleans_up_a_real_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"ui": {"theme": "Dracula"}}"#).unwrap();

        ensure_settings(dir.path(), hooks_dir());
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(written["ui"]["theme"], "Dracula");
        assert!(written["hooks"]["AfterAgent"].is_array());

        cleanup_settings(dir.path(), hooks_dir());
        let cleaned: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(cleaned["ui"]["theme"], "Dracula");
        assert!(cleaned.get("hooks").is_none());
    }

    #[test]
    fn invalid_user_file_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(SETTINGS_FILE);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "// comment\n{").unwrap();
        ensure_settings(dir.path(), hooks_dir());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "// comment\n{");
    }
}
