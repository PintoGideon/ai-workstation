//! FORK: the `aiw` subcommands the hook scripts call (DESIGN.md §16).
//!
//! Each reads the harness's hook JSON from stdin, adds AIW's session identity
//! from the environment it inherited (`AIW_SESSION`, `AIW_FEATURE_SESSION_ID`,
//! …) and talks to the dashboard's socket. Shell scripts never parse JSON;
//! everything that reads or prints JSON is here, and the pure parts live in
//! their own modules so they are unit-tested without a socket:
//!
//! - `aiw permission-request` → [`super::permissions`]: sends the request and
//!   waits on the same connection for the user's answer.
//!
//! Nothing here fails the agent's turn: every error path prints nothing and
//! exits 0 (the scripts also discard stderr).

use std::time::Duration;

use serde_json::{Value, json};

use super::permissions::{self, Behavior, Harness};
use crate::hook_payload::HookEnv;

/// The hook's stdin as JSON, or an empty object.
fn stdin_json() -> Value {
    use std::io::Read;
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    serde_json::from_str(input.trim()).unwrap_or_else(|_| json!({}))
}

/// AIW's session identity for a hook message.
fn identity_fields(env: &HookEnv, payload: &mut serde_json::Map<String, Value>) {
    if let Some(session) = env.aiw_tmux_session.clone().or(env.aiw_session.clone()) {
        payload.insert("aiw_session".into(), Value::String(session.clone()));
        payload.insert("aiw_tmux_session".into(), Value::String(session));
    }
    if let Some(id) = &env.aiw_feature_session_id {
        payload.insert("aiw_feature_session_id".into(), Value::String(id.clone()));
    }
    if let Some(window) = &env.aiw_tmux_window {
        payload.insert("aiw_tmux_window".into(), Value::String(window.clone()));
    }
    if !payload.contains_key("cwd")
        && let Ok(cwd) = std::env::current_dir()
    {
        payload.insert(
            "cwd".into(),
            Value::String(cwd.to_string_lossy().into_owned()),
        );
    }
}

/// The message `aiw permission-request` sends.
pub fn permission_request_message(
    env: &HookEnv,
    request: &permissions::Request,
    request_id: &str,
) -> Value {
    let mut payload = serde_json::Map::new();
    payload.insert(
        "type".into(),
        Value::String(permissions::REQUEST_TYPE.into()),
    );
    payload.insert("request_id".into(), Value::String(request_id.into()));
    payload.insert(
        "harness".into(),
        Value::String(request.harness.as_str().into()),
    );
    if let Some(id) = &request.provider_session_id {
        payload.insert("session_id".into(), Value::String(id.clone()));
    }
    if let Some(cwd) = &request.cwd {
        payload.insert("cwd".into(), Value::String(cwd.clone()));
    }
    payload.insert("tool_name".into(), Value::String(request.tool_name.clone()));
    payload.insert("tool_input".into(), request.tool_input.clone());
    identity_fields(env, &mut payload);
    Value::Object(payload)
}

/// What the hook prints for the dashboard's answer (or its absence).
pub fn permission_output(harness: Harness, answer: Option<&Value>) -> Option<String> {
    let behavior = answer
        .and_then(|a| a.get("behavior"))
        .and_then(Value::as_str)
        .and_then(Behavior::parse)
        .unwrap_or(Behavior::Pass);
    let message = answer
        .and_then(|a| a.get("message"))
        .and_then(Value::as_str);
    permissions::decision_json(harness, behavior, message)
}

/// `aiw status-line`: Claude's status JSON on stdin → the dashboard, and one
/// line for Claude's footer (or the user's own status line, run as is).
pub fn run_status_line() {
    let status = stdin_json();
    let telemetry = super::telemetry::parse_claude_status(&status);
    let env = HookEnv::from_process_env();
    let message = super::telemetry::status_line_message(&env, &status, &telemetry);
    if let Ok(line) = serde_json::to_string(&message) {
        let _ = crate::ipc::send(&crate::ipc::socket_path(), &line);
    }
    if let Some(output) = run_user_status_line(&status) {
        print!("{output}");
        return;
    }
    println!("{}", super::telemetry::footer_line(&telemetry));
}

/// The user's own `statusLine` command from `$CLAUDE_CONFIG_DIR/settings.json`
/// (default `~/.claude/settings.json`), run with the same JSON; its output
/// is what Claude's footer shows. Read-only; nothing is written there.
fn run_user_status_line(status: &Value) -> Option<String> {
    use std::io::Write;
    let config_dir = std::env::var_os("CLAUDE_CONFIG_DIR")
        .map(std::path::PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| dirs::home_dir().map(|h| h.join(".claude")))?;
    let command = super::telemetry::user_status_line_command(&config_dir.join("settings.json"))?;
    let mut child = std::process::Command::new("sh")
        .arg("-c")
        .arg(&command)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(status.to_string().as_bytes());
    }
    let output = child.wait_with_output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

/// `aiw permission-request --harness <h> [--event <e>]`.
pub fn run_permission_request(harness: &str, event: Option<&str>) {
    let Some(harness) = Harness::parse(harness) else {
        return;
    };
    let payload = stdin_json();
    let request = permissions::parse_request(harness, &payload, event);
    let env = HookEnv::from_process_env();
    let request_id = uuid::Uuid::new_v4().to_string();
    let message = permission_request_message(&env, &request, &request_id);
    let wait = Duration::from_secs(super::config::current().permission_wait_seconds.max(1));
    let answer = super::ipc_reply::request(&crate::ipc::socket_path(), message, wait)
        .ok()
        .flatten();
    if let Some(output) = permission_output(harness, answer.as_ref()) {
        println!("{output}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> HookEnv {
        HookEnv {
            aiw_session: Some("aiw-hx-12".into()),
            aiw_feature_session_id: Some("fs-1".into()),
            aiw_tmux_session: Some("aiw-hx-12".into()),
            aiw_tmux_window: Some("claude".into()),
        }
    }

    #[test]
    fn the_request_message_carries_the_tool_and_the_session_identity() {
        let request = permissions::parse_request(
            Harness::Claude,
            &json!({"session_id": "p1", "cwd": "/w", "tool_name": "Bash", "tool_input": {"command": "ls"}}),
            None,
        );
        let message = permission_request_message(&env(), &request, "req-1");
        assert_eq!(message["type"], "permission_request");
        assert_eq!(message["request_id"], "req-1");
        assert_eq!(message["harness"], "claude");
        assert_eq!(message["session_id"], "p1");
        assert_eq!(message["aiw_session"], "aiw-hx-12");
        assert_eq!(message["aiw_feature_session_id"], "fs-1");
        assert_eq!(message["tool_name"], "Bash");
        assert_eq!(message["tool_input"]["command"], "ls");
        assert_eq!(message["cwd"], "/w");
    }

    #[test]
    fn no_answer_means_the_harness_prompts() {
        assert!(permission_output(Harness::Claude, None).is_none());
        assert!(permission_output(Harness::Claude, Some(&json!({"behavior": "pass"}))).is_none());
        assert!(permission_output(Harness::Claude, Some(&json!({"nonsense": 1}))).is_none());
        let allow = permission_output(Harness::Codex, Some(&json!({"behavior": "allow"}))).unwrap();
        assert!(allow.contains("\"allow\""));
        let ask = permission_output(Harness::Cursor, None).unwrap();
        assert!(ask.contains("\"ask\""));
    }
}
