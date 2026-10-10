//! FORK: Codex CLI hooks in the worktree's `.codex/hooks.json` (DESIGN.md §16).
//!
//! Codex's older `notify` argv (`scripts/codex-notify.sh`, passed per launch
//! with `-c notify=[…]`) only fires at the end of a turn. Its hooks (GA
//! 2026-05, docs/development/harness-hooks-reference.md) add the events the
//! dashboard answers or logs: `PermissionRequest`, `Stop`, `PreCompact`,
//! `SessionEnd`, `SubagentStart` / `SubagentStop`. They live in the project
//! layer, `<worktree>/.codex/hooks.json`, merged with whatever the user has
//! there and kept out of git.
//!
//! Codex trusts a project hook by the hash of its definition: the first run
//! in a worktree asks the user to trust AIW's hooks once (`/hooks` in the
//! Codex TUI). AIW never passes `--dangerously-bypass-hook-trust`; `aiw
//! doctor` and the README say so.

use std::path::Path;

use serde_json::{Value, json};

use super::extra_agents::{is_aiw_command, script_command, write_managed_file};

/// The hooks file AIW merges into, relative to the checkout.
pub const HOOKS_FILE: &str = ".codex/hooks.json";

/// Per-hook timeout in seconds (Codex's unit): answering a permission or a
/// Stop hook waits on the user; the rest are quick.
const WAIT_TIMEOUT_SECS: u64 = 600;
const QUICK_TIMEOUT_SECS: u64 = 10;

/// `(event, [(script, args)])` — what AIW wires. Every script reads the
/// hook JSON on stdin and never fails the turn.
type Step = (&'static str, &'static [&'static str]);

pub const EVENTS: &[(&str, &[Step])] = &[
    (
        "PermissionRequest",
        &[("permission-request.sh", &["codex"])],
    ),
    // A queued follow-up becomes the next turn (P7).
    ("Stop", &[("stop-hook.sh", &["codex"])]),
];

fn timeout_for(event: &str) -> u64 {
    match event {
        "PermissionRequest" | "Stop" => WAIT_TIMEOUT_SECS,
        _ => QUICK_TIMEOUT_SECS,
    }
}

fn is_aiw_hook(hook: &Value, hooks_dir: &Path) -> bool {
    hook.get("command")
        .and_then(Value::as_str)
        .is_some_and(|command| is_aiw_command(command, hooks_dir))
}

/// Drop AIW's hooks from `config.hooks`, keeping the user's; groups and
/// events left empty go too.
pub fn remove_aiw_hooks(config: &mut Value, hooks_dir: &Path) {
    let Some(hooks) = config.get_mut("hooks").and_then(Value::as_object_mut) else {
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
        && let Some(map) = config.as_object_mut()
    {
        map.remove("hooks");
    }
}

/// `config` with AIW's hooks (re)installed; everything else preserved.
pub fn merge_hooks(mut config: Value, hooks_dir: &Path) -> Value {
    if !config.is_object() {
        config = json!({});
    }
    remove_aiw_hooks(&mut config, hooks_dir);
    let root = config.as_object_mut().expect("object");
    let hooks = root.entry("hooks").or_insert_with(|| json!({}));
    if !hooks.is_object() {
        *hooks = json!({});
    }
    let hooks = hooks.as_object_mut().expect("object");
    for (event, steps) in EVENTS {
        let entries: Vec<Value> = steps
            .iter()
            .map(|(script, args)| {
                json!({
                    "type": "command",
                    "command": script_command(hooks_dir, script, args),
                    "timeout": timeout_for(event),
                    "statusMessage": "AIW",
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
    config
}

pub fn ensure_hooks_file(workdir: &Path, hooks_dir: &Path) {
    write_managed_file(workdir, HOOKS_FILE, |existing| {
        merge_hooks(existing, hooks_dir)
    });
}

pub fn cleanup_hooks_file(workdir: &Path, hooks_dir: &Path) {
    let path = workdir.join(HOOKS_FILE);
    if !path.exists() {
        return;
    }
    let Some(mut config) = super::extra_agents::read_json_object(&path) else {
        return;
    };
    remove_aiw_hooks(&mut config, hooks_dir);
    if config.as_object().is_some_and(|root| root.is_empty()) {
        let _ = std::fs::remove_file(&path);
    } else {
        super::extra_agents::write_json_if_changed(&path, &config);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hooks_dir() -> &'static Path {
        Path::new("/home/u/.config/ai_workstation/hooks")
    }

    #[test]
    fn a_fresh_file_wires_every_event_with_the_right_timeouts() {
        let config = merge_hooks(json!({}), hooks_dir());
        for (event, _) in EVENTS {
            let groups = config["hooks"][event].as_array().unwrap();
            assert_eq!(groups.len(), 1, "{event}");
            for hook in groups[0]["hooks"].as_array().unwrap() {
                assert_eq!(hook["type"], "command");
                assert!(
                    hook["command"]
                        .as_str()
                        .unwrap()
                        .starts_with("'/home/u/.config/ai_workstation/hooks/")
                );
            }
        }
        let permission = &config["hooks"]["PermissionRequest"][0]["hooks"][0];
        assert_eq!(
            permission["command"],
            "'/home/u/.config/ai_workstation/hooks/permission-request.sh' codex"
        );
        assert_eq!(permission["timeout"], 600);
    }

    #[test]
    fn merging_keeps_user_hooks_and_is_idempotent() {
        let existing = json!({
            "hooks": {
                "PermissionRequest": [{"hooks": [{"type": "command", "command": "./mine.sh"}]}],
                "PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "./guard.sh"}]}]
            }
        });
        let merged = merge_hooks(existing, hooks_dir());
        let merged = merge_hooks(merged, hooks_dir());
        let permission = merged["hooks"]["PermissionRequest"].as_array().unwrap();
        assert_eq!(permission.len(), 2);
        assert_eq!(permission[0]["hooks"][0]["command"], "./mine.sh");
        assert_eq!(merged["hooks"]["PreToolUse"][0]["matcher"], "Bash");

        let mut cleaned = merged.clone();
        remove_aiw_hooks(&mut cleaned, hooks_dir());
        assert_eq!(
            cleaned["hooks"]["PermissionRequest"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            cleaned["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
            "./guard.sh"
        );
    }

    #[test]
    fn files_are_written_and_cleaned_up_in_the_worktree() {
        let dir = tempfile::tempdir().unwrap();
        ensure_hooks_file(dir.path(), hooks_dir());
        let path = dir.path().join(HOOKS_FILE);
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(written["hooks"]["PermissionRequest"].is_array());
        cleanup_hooks_file(dir.path(), hooks_dir());
        assert!(!path.exists(), "nothing of the user's: the file goes");

        std::fs::write(
            &path,
            r#"{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"./mine.sh"}]}]}}"#,
        )
        .unwrap();
        ensure_hooks_file(dir.path(), hooks_dir());
        cleanup_hooks_file(dir.path(), hooks_dir());
        let left: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(left["hooks"]["Stop"][0]["hooks"][0]["command"], "./mine.sh");
        assert!(left["hooks"].get("PermissionRequest").is_none());
    }
}
