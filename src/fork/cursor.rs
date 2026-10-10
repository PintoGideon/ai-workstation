//! Cursor agent CLI (`agent`, legacy alias `cursor-agent`) — launch flags and
//! status hooks.
//!
//! Verified against cursor.com/docs/cli/overview, /docs/cli/reference/parameters
//! and /docs/hooks:
//! - `-f/--force` allows commands without asking; there is no edits-only
//!   auto-approve flag. `--model <model>`, `--resume [chatId]`.
//! - `.cursor/hooks.json` is `{"version": 1, "hooks": {"<event>": [{"command",
//!   "timeout" (seconds)}]}}`; project hooks run from the project root.
//! - Hook stdin carries `conversation_id` (the id `--resume` takes; also sent
//!   as `session_id` on `sessionStart`), `transcript_path`, `workspace_roots`,
//!   `hook_event_name`; `stop` adds `status`.
//! - `beforeSubmitPrompt` answers `{"continue": true}`. `preToolUse` is a
//!   *permission* hook (invalid output can block, `allow` may approve), so AIW
//!   deliberately does not wire it; `postToolUse` is observational.
//! - There is no permission/question hook, so Cursor reports Completed but
//!   never Question; a blocked prompt shows as AIW's generic Waiting.

use std::ffi::OsStr;
use std::path::Path;

use serde_json::{Value, json};

use super::extra_agents::{find_on_path, is_aiw_command, script_command, write_managed_file};
use crate::project::VibeMode;

/// Binary names in preference order: `agent` is current, `cursor-agent` is
/// the backward-compatible alias.
pub const BINARIES: &[&str] = &["agent", "cursor-agent"];

/// The hooks file AIW merges into, relative to the checkout.
pub const HOOKS_FILE: &str = ".cursor/hooks.json";

/// Per-hook timeout, in seconds (Cursor's unit).
const HOOK_TIMEOUT_SECS: u64 = 10;
/// Timeout for hooks that wait for an answer from the dashboard (P6).
const WAIT_TIMEOUT_SECS: u64 = 600;
/// How many queued follow-ups Cursor delivers back to back (P7).
pub const STOP_LOOP_LIMIT: u64 = 20;

/// The binary to launch: the first of [`BINARIES`] on `path_var`, else
/// `agent` (so a missing install fails visibly in the pane).
pub fn resolve_binary(path_var: &OsStr) -> String {
    BINARIES
        .iter()
        .find(|name| find_on_path(name, path_var).is_some())
        .unwrap_or(&BINARIES[0])
        .to_string()
}

pub fn launch_args(mode: &VibeMode, resume_id: Option<&str>, model: Option<&str>) -> Vec<String> {
    let mut args = Vec::new();
    if matches!(mode, VibeMode::SuperVibe) {
        args.push("--force".to_string());
    }
    if let Some(model) = model {
        args.extend(["--model".to_string(), model.to_string()]);
    }
    if let Some(id) = resume_id {
        args.extend(["--resume".to_string(), id.to_string()]);
    }
    args
}

type Step = (&'static str, &'static [&'static str]);

/// `(event, scripts to run in order, JSON to print afterwards)`.
pub const EVENTS: &[(&str, &[Step], Option<&str>)] = &[
    ("sessionStart", &[("clear-notify.sh", &[])], None),
    (
        "beforeSubmitPrompt",
        &[
            ("thinking-start.sh", &[]),
            ("save-prompt.sh", &[]),
            ("clear-notify.sh", &[]),
        ],
        Some(r#"{"continue":true}"#),
    ),
    (
        "postToolUse",
        &[("thinking-start.sh", &[]), ("tool-stop.sh", &[])],
        None,
    ),
    (
        "stop",
        &[
            ("thinking-stop.sh", &[]),
            ("notify.sh", &[]),
            ("attention.sh", &["completed"]),
            // Last, so its `followup_message` is the hook's answer (P7).
            ("stop-hook.sh", &["cursor"]),
        ],
        None,
    ),
    ("sessionEnd", &[("thinking-stop.sh", &[])], None),
    // FORK(P5): the badge's subagent count. `subagentStart` is a permission
    // hook (invalid output blocks), so the entry answers `allow` itself.
    (
        "subagentStart",
        &[("subagent.sh", &["start", "cursor"])],
        Some(r#"{"permission":"allow"}"#),
    ),
    (
        "subagentStop",
        &[("subagent.sh", &["stop", "cursor"])],
        None,
    ),
    // FORK(P6): the dashboard answers the prompt; the script prints
    // `permission` allow/deny, or `ask` when nobody answered.
    (
        "beforeShellExecution",
        &[("permission-request.sh", &["cursor", "beforeShellExecution"])],
        None,
    ),
    (
        "beforeMCPExecution",
        &[("permission-request.sh", &["cursor", "beforeMCPExecution"])],
        None,
    ),
];

/// Per-hook timeout in seconds (Cursor's unit): hooks that wait on the user
/// get the long one.
fn timeout_for(event: &str) -> u64 {
    if event.starts_with("before") && event != "beforeSubmitPrompt" {
        WAIT_TIMEOUT_SECS
    } else {
        HOOK_TIMEOUT_SECS
    }
}

/// One shell command per event: the scripts in order (each reads the same
/// stdin, so it is saved first), then the event's required JSON answer.
fn event_command(hooks_dir: &Path, steps: &[Step], answer: Option<&str>) -> String {
    let mut parts = Vec::new();
    if steps.len() > 1 {
        parts.push("p=$(cat)".to_string());
        for (script, args) in steps {
            parts.push(format!(
                "printf '%s' \"$p\" | {}",
                script_command(hooks_dir, script, args)
            ));
        }
    } else if let Some((script, args)) = steps.first() {
        parts.push(script_command(hooks_dir, script, args));
    }
    if let Some(answer) = answer {
        parts.push(format!("printf '%s\\n' '{answer}'"));
    }
    parts.join("; ")
}

fn is_aiw_entry(entry: &Value, hooks_dir: &Path) -> bool {
    entry
        .get("command")
        .and_then(Value::as_str)
        .is_some_and(|command| is_aiw_command(command, hooks_dir))
}

/// Drop AIW's entries from `config.hooks`, keeping the user's.
pub fn remove_aiw_hooks(config: &mut Value, hooks_dir: &Path) {
    let Some(hooks) = config.get_mut("hooks").and_then(Value::as_object_mut) else {
        return;
    };
    for entries in hooks.values_mut() {
        if let Some(entries) = entries.as_array_mut() {
            entries.retain(|entry| !is_aiw_entry(entry, hooks_dir));
        }
    }
    hooks.retain(|_, entries| entries.as_array().is_none_or(|list| !list.is_empty()));
}

/// `config` with AIW's hooks (re)installed; everything else preserved.
pub fn merge_hooks(mut config: Value, hooks_dir: &Path) -> Value {
    if !config.is_object() {
        config = json!({});
    }
    remove_aiw_hooks(&mut config, hooks_dir);
    let root = config.as_object_mut().expect("object");
    root.entry("version").or_insert(json!(1));
    let hooks = root.entry("hooks").or_insert_with(|| json!({}));
    if !hooks.is_object() {
        *hooks = json!({});
    }
    let hooks = hooks.as_object_mut().expect("object");
    for (event, steps, answer) in EVENTS {
        let entries = hooks
            .entry(event.to_string())
            .or_insert_with(|| Value::Array(Vec::new()));
        if !entries.is_array() {
            *entries = Value::Array(Vec::new());
        }
        let mut entry = json!({
            "command": event_command(hooks_dir, steps, *answer),
            "timeout": timeout_for(event),
        });
        if *event == "stop" {
            // Cursor stops honouring `followup_message` after `loop_limit`
            // consecutive continuations (default 5); a queue can be longer.
            entry["loop_limit"] = json!(STOP_LOOP_LIMIT);
        }
        entries.as_array_mut().expect("array").push(entry);
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
    super::extra_agents::write_json_if_changed(&path, &config);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hooks_dir() -> &'static Path {
        Path::new("/home/u/.aiw/hooks")
    }

    #[test]
    fn only_supervibe_forces() {
        assert!(launch_args(&VibeMode::Vibeless, None, None).is_empty());
        assert!(launch_args(&VibeMode::Vibe, None, None).is_empty());
        assert_eq!(launch_args(&VibeMode::SuperVibe, None, None), ["--force"]);
    }

    #[test]
    fn resume_and_model_flags() {
        assert_eq!(
            launch_args(&VibeMode::SuperVibe, Some("chat-1"), Some("gpt-5")),
            ["--force", "--model", "gpt-5", "--resume", "chat-1"]
        );
        assert_eq!(
            launch_args(&VibeMode::Vibe, Some("chat-1"), None),
            ["--resume", "chat-1"]
        );
    }

    #[cfg(unix)]
    #[test]
    fn prefers_agent_then_cursor_agent() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let make = |name: &str| {
            let path = dir.path().join(name);
            std::fs::write(&path, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        assert_eq!(resolve_binary(dir.path().as_os_str()), "agent");
        make("cursor-agent");
        assert_eq!(resolve_binary(dir.path().as_os_str()), "cursor-agent");
        make("agent");
        assert_eq!(resolve_binary(dir.path().as_os_str()), "agent");
    }

    #[test]
    fn fresh_config_wires_every_event() {
        let config = merge_hooks(json!({}), hooks_dir());
        assert_eq!(config["version"], 1);
        for event in [
            "sessionStart",
            "beforeSubmitPrompt",
            "postToolUse",
            "stop",
            "sessionEnd",
        ] {
            let entries = config["hooks"][event].as_array().unwrap();
            assert_eq!(entries.len(), 1, "{event}");
            assert_eq!(entries[0]["timeout"], HOOK_TIMEOUT_SECS);
        }
        // preToolUse is a permission hook; AIW must not answer it.
        assert!(config["hooks"].get("preToolUse").is_none());
        // FORK(P6): the permission hooks wait on the dashboard.
        for event in ["beforeShellExecution", "beforeMCPExecution"] {
            let entries = config["hooks"][event].as_array().unwrap();
            assert_eq!(entries.len(), 1, "{event}");
            assert_eq!(entries[0]["timeout"], WAIT_TIMEOUT_SECS);
            assert_eq!(
                entries[0]["command"],
                format!("'/home/u/.aiw/hooks/permission-request.sh' cursor {event}")
            );
        }

        let submit = config["hooks"]["beforeSubmitPrompt"][0]["command"]
            .as_str()
            .unwrap();
        assert!(submit.starts_with("p=$(cat); "), "{submit}");
        assert!(submit.contains("| '/home/u/.aiw/hooks/thinking-start.sh'"));
        assert!(
            submit.ends_with(r#"printf '%s\n' '{"continue":true}'"#),
            "{submit}"
        );

        let stop = config["hooks"]["stop"][0]["command"].as_str().unwrap();
        assert!(stop.contains("'/home/u/.aiw/hooks/attention.sh' completed"));
        assert_eq!(
            config["hooks"]["sessionEnd"][0]["command"],
            "'/home/u/.aiw/hooks/thinking-stop.sh'"
        );
        assert_eq!(config["hooks"]["stop"][0]["loop_limit"], STOP_LOOP_LIMIT);
    }

    #[cfg(unix)]
    #[test]
    fn combined_command_feeds_stdin_to_every_script_and_answers() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out.txt");
        for name in ["thinking-start.sh", "save-prompt.sh", "clear-notify.sh"] {
            let path = dir.path().join(name);
            std::fs::write(
                &path,
                format!(
                    "#!/bin/sh\nprintf '{name}:' >> '{}'\ncat >> '{}'\n",
                    out.display(),
                    out.display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let (_, steps, answer) = EVENTS
            .iter()
            .find(|(event, _, _)| *event == "beforeSubmitPrompt")
            .unwrap();
        let command = event_command(dir.path(), steps, *answer);
        let mut child = std::process::Command::new("sh")
            .args(["-c", &command])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(b"{\"prompt\":\"hi\"}")
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "{\"continue\":true}\n"
        );
        assert_eq!(
            std::fs::read_to_string(&out).unwrap(),
            "thinking-start.sh:{\"prompt\":\"hi\"}save-prompt.sh:{\"prompt\":\"hi\"}clear-notify.sh:{\"prompt\":\"hi\"}"
        );
    }

    #[test]
    fn merging_keeps_user_hooks_and_is_idempotent() {
        let existing = json!({
            "version": 1,
            "hooks": {
                "stop": [{"command": ".cursor/hooks/audit.sh"}],
                "preToolUse": [{"command": "./guard.sh", "matcher": "Shell"}]
            }
        });
        let merged = merge_hooks(existing, hooks_dir());
        let stop = merged["hooks"]["stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2);
        assert_eq!(stop[0]["command"], ".cursor/hooks/audit.sh");
        assert_eq!(merged["hooks"]["preToolUse"][0]["command"], "./guard.sh");

        assert_eq!(merge_hooks(merged.clone(), hooks_dir()), merged);

        let mut cleaned = merged;
        remove_aiw_hooks(&mut cleaned, hooks_dir());
        assert_eq!(
            cleaned,
            json!({"version": 1, "hooks": {
                "stop": [{"command": ".cursor/hooks/audit.sh"}],
                "preToolUse": [{"command": "./guard.sh", "matcher": "Shell"}]
            }})
        );
    }

    #[test]
    fn writes_and_cleans_up_a_real_file() {
        let dir = tempfile::tempdir().unwrap();
        ensure_hooks_file(dir.path(), hooks_dir());
        let path = dir.path().join(HOOKS_FILE);
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(written["hooks"]["stop"].is_array());

        cleanup_hooks_file(dir.path(), hooks_dir());
        let cleaned: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(cleaned, json!({"version": 1, "hooks": {}}));
    }
}
