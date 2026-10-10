//! FORK: permission approvals from the dashboard (P6, DESIGN.md §16).
//!
//! A harness about to show a permission prompt runs
//! `scripts/permission-request.sh`, which calls `aiw permission-request`.
//! That command reads the hook's stdin, sends a `permission_request` to the
//! dashboard and *waits* on the same connection ([`super::ipc_reply`]) for
//! the user's decision. The pure parts — reading each harness's payload,
//! the preview shown on the row, and the decision JSON each harness expects
//! — live here so they are unit-tested without a socket.
//!
//! Harness contracts (docs/development/harness-hooks-reference.md):
//! - Claude `PermissionRequest`: stdin `tool_name`, `tool_input`; stdout
//!   `hookSpecificOutput.decision.behavior` allow/deny (deny takes a
//!   `message`). Exit 0 with no output → the normal prompt.
//! - Codex `PermissionRequest`: the same shape (no `updatedInput`).
//! - Cursor `beforeShellExecution` (`command`) and `beforeMCPExecution`
//!   (`tool_name`, `tool_input` string): stdout `{"permission": "allow" |
//!   "deny" | "ask"}`; `ask` is the interactive prompt. Invalid JSON blocks
//!   the call, so passing prints `ask` rather than nothing.
//! - Gemini: no hook can answer a prompt; its `Notification(ToolPermission)`
//!   stays a `⛔` the user answers in the pane.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// Default `permission_wait_seconds`: how long the hook waits for the
/// dashboard before letting the harness prompt itself.
pub const DEFAULT_WAIT_SECONDS: u64 = 300;

/// The IPC message type the hook sends.
pub const REQUEST_TYPE: &str = "permission_request";

/// The harnesses whose prompts a hook can answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Harness {
    Claude,
    Codex,
    Cursor,
}

impl Harness {
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "cursor" | "cursor-agent" | "agent" => Some(Self::Cursor),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Cursor => "cursor",
        }
    }
}

/// What the user decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Behavior {
    Allow,
    Deny,
    /// Let the harness show its own prompt.
    Pass,
}

impl Behavior {
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "allow" => Some(Self::Allow),
            "deny" => Some(Self::Deny),
            "pass" => Some(Self::Pass),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
            Self::Pass => "pass",
        }
    }
}

/// The request as the dashboard sees it (the IPC message body, minus the
/// session identity `aiw notify` adds from the environment).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub harness: Harness,
    pub tool_name: String,
    pub tool_input: Value,
    pub provider_session_id: Option<String>,
    pub cwd: Option<String>,
}

/// Read a harness's hook payload. `event` is the Cursor hook name (the two
/// Cursor events carry different fields); ignored for the others.
pub fn parse_request(harness: Harness, payload: &Value, event: Option<&str>) -> Request {
    let text = |key: &str| {
        payload
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|value| !value.is_empty())
    };
    let provider_session_id = text("session_id").or_else(|| text("conversation_id"));
    let cwd = text("cwd");
    let (tool_name, tool_input) = match harness {
        Harness::Claude | Harness::Codex => (
            text("tool_name").unwrap_or_else(|| "tool".to_string()),
            payload.get("tool_input").cloned().unwrap_or(Value::Null),
        ),
        Harness::Cursor => {
            let is_mcp = event.is_some_and(|e| e.eq_ignore_ascii_case("beforeMCPExecution"))
                || (payload.get("command").is_none() && payload.get("tool_name").is_some());
            if is_mcp {
                let name = text("tool_name").unwrap_or_else(|| "MCP".to_string());
                let name = match text("mcp_server_name") {
                    Some(server) => format!("{server}/{name}"),
                    None => name,
                };
                // Cursor passes the MCP input as a string; keep it readable.
                let input = match payload.get("tool_input") {
                    Some(Value::String(s)) => {
                        serde_json::from_str(s).unwrap_or_else(|_| Value::String(s.clone()))
                    }
                    Some(other) => other.clone(),
                    None => Value::Null,
                };
                (name, input)
            } else {
                let mut input = serde_json::Map::new();
                if let Some(command) = text("command") {
                    input.insert("command".to_string(), Value::String(command));
                }
                if let Some(cwd) = text("cwd") {
                    input.insert("cwd".to_string(), Value::String(cwd));
                }
                ("Shell".to_string(), Value::Object(input))
            }
        }
    };
    Request {
        harness,
        tool_name,
        tool_input,
        provider_session_id,
        cwd,
    }
}

/// One line that says what the tool wants: the command for a shell tool,
/// the path for a file tool, the first string field otherwise.
pub fn input_preview(tool_input: &Value, max_chars: usize) -> String {
    const KEYS: &[&str] = &[
        "command",
        "file_path",
        "path",
        "pattern",
        "url",
        "query",
        "description",
        "prompt",
    ];
    let text = match tool_input {
        Value::Object(map) => KEYS
            .iter()
            .find_map(|key| map.get(*key).and_then(Value::as_str))
            .map(str::to_string)
            .or_else(|| {
                map.values()
                    .find_map(|value| value.as_str().map(str::to_string))
            })
            .unwrap_or_else(|| {
                if map.is_empty() {
                    String::new()
                } else {
                    serde_json::to_string(tool_input).unwrap_or_default()
                }
            }),
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => serde_json::to_string(other).unwrap_or_default(),
    };
    let one_line: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut shown: String = one_line.chars().take(max_chars).collect();
    if shown.chars().count() < one_line.chars().count() {
        shown.push('…');
    }
    shown
}

/// The badge reason: `Bash — cargo test` (the badge model prefixes
/// `permission: `).
pub fn detail(tool_name: &str, tool_input: &Value) -> String {
    let preview = input_preview(tool_input, 60);
    if preview.is_empty() {
        tool_name.to_string()
    } else {
        format!("{tool_name} — {preview}")
    }
}

/// The JSON the hook prints for `behavior`, or `None` when the harness
/// should show its own prompt (nothing is printed).
pub fn decision_json(
    harness: Harness,
    behavior: Behavior,
    message: Option<&str>,
) -> Option<String> {
    let message = message
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .unwrap_or("Denied from the AIW dashboard");
    let value = match (harness, behavior) {
        (Harness::Claude | Harness::Codex, Behavior::Allow) => json!({
            "hookSpecificOutput": {
                "hookEventName": "PermissionRequest",
                "decision": { "behavior": "allow" }
            }
        }),
        (Harness::Claude, Behavior::Deny) => json!({
            "hookSpecificOutput": {
                "hookEventName": "PermissionRequest",
                "decision": { "behavior": "deny", "message": message, "interrupt": false }
            }
        }),
        (Harness::Codex, Behavior::Deny) => json!({
            "hookSpecificOutput": {
                "hookEventName": "PermissionRequest",
                "decision": { "behavior": "deny", "message": message }
            }
        }),
        (Harness::Claude | Harness::Codex, Behavior::Pass) => return None,
        (Harness::Cursor, Behavior::Allow) => json!({ "continue": true, "permission": "allow" }),
        (Harness::Cursor, Behavior::Deny) => json!({
            "continue": true,
            "permission": "deny",
            "user_message": message,
            "agent_message": message
        }),
        // An empty stdout is invalid JSON to Cursor's permission hooks, which
        // blocks the call; `ask` is its documented "prompt me" answer.
        (Harness::Cursor, Behavior::Pass) => json!({ "continue": true, "permission": "ask" }),
    };
    Some(value.to_string())
}

/// What the app answers on the open connection.
pub fn reply_json(behavior: Behavior, message: Option<&str>) -> Value {
    json!({ "behavior": behavior.as_str(), "message": message })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_and_codex_payloads_keep_tool_name_and_input() {
        let payload = json!({
            "session_id": "abc", "cwd": "/w", "hook_event_name": "PermissionRequest",
            "tool_name": "Bash", "tool_input": {"command": "cargo test", "description": "run"}
        });
        let request = parse_request(Harness::Claude, &payload, None);
        assert_eq!(request.tool_name, "Bash");
        assert_eq!(request.tool_input["command"], "cargo test");
        assert_eq!(request.provider_session_id.as_deref(), Some("abc"));
        assert_eq!(
            detail(&request.tool_name, &request.tool_input),
            "Bash — cargo test"
        );
        let codex = parse_request(Harness::Codex, &payload, None);
        assert_eq!(codex.tool_name, "Bash");
    }

    #[test]
    fn cursor_shell_and_mcp_payloads_become_tool_name_and_input() {
        let shell = json!({"conversation_id": "c1", "command": "rm -rf build", "cwd": "/w"});
        let request = parse_request(Harness::Cursor, &shell, Some("beforeShellExecution"));
        assert_eq!(request.tool_name, "Shell");
        assert_eq!(request.tool_input["command"], "rm -rf build");
        assert_eq!(request.provider_session_id.as_deref(), Some("c1"));

        let mcp = json!({
            "conversation_id": "c1", "tool_name": "query", "mcp_server_name": "db",
            "tool_input": "{\"sql\": \"select 1\"}"
        });
        let request = parse_request(Harness::Cursor, &mcp, Some("beforeMCPExecution"));
        assert_eq!(request.tool_name, "db/query");
        assert_eq!(request.tool_input["sql"], "select 1");
        assert_eq!(
            detail(&request.tool_name, &request.tool_input),
            "db/query — select 1"
        );
    }

    #[test]
    fn previews_are_one_short_line() {
        let long = json!({"command": format!("echo {}", "x".repeat(200))});
        let preview = input_preview(&long, 20);
        assert_eq!(preview.chars().count(), 21);
        assert!(preview.ends_with('…'));
        assert_eq!(input_preview(&json!({"file_path": "a.rs"}), 60), "a.rs");
        assert_eq!(input_preview(&json!({"other": "x\ny"}), 60), "x y");
        assert_eq!(input_preview(&json!({}), 60), "");
        assert_eq!(input_preview(&Value::Null, 60), "");
        assert_eq!(detail("Bash", &Value::Null), "Bash");
    }

    #[test]
    fn decision_json_matches_each_harness_contract() {
        let allow: Value =
            serde_json::from_str(&decision_json(Harness::Claude, Behavior::Allow, None).unwrap())
                .unwrap();
        assert_eq!(
            allow["hookSpecificOutput"]["hookEventName"],
            "PermissionRequest"
        );
        assert_eq!(allow["hookSpecificOutput"]["decision"]["behavior"], "allow");

        let deny: Value = serde_json::from_str(
            &decision_json(Harness::Claude, Behavior::Deny, Some("no")).unwrap(),
        )
        .unwrap();
        assert_eq!(deny["hookSpecificOutput"]["decision"]["behavior"], "deny");
        assert_eq!(deny["hookSpecificOutput"]["decision"]["message"], "no");
        assert_eq!(deny["hookSpecificOutput"]["decision"]["interrupt"], false);

        let codex_deny: Value =
            serde_json::from_str(&decision_json(Harness::Codex, Behavior::Deny, None).unwrap())
                .unwrap();
        assert_eq!(
            codex_deny["hookSpecificOutput"]["decision"]["behavior"],
            "deny"
        );
        assert!(
            codex_deny["hookSpecificOutput"]["decision"]
                .get("interrupt")
                .is_none()
        );

        assert!(decision_json(Harness::Claude, Behavior::Pass, None).is_none());
        assert!(decision_json(Harness::Codex, Behavior::Pass, None).is_none());

        let cursor_allow: Value =
            serde_json::from_str(&decision_json(Harness::Cursor, Behavior::Allow, None).unwrap())
                .unwrap();
        assert_eq!(cursor_allow["permission"], "allow");
        let cursor_deny: Value =
            serde_json::from_str(&decision_json(Harness::Cursor, Behavior::Deny, None).unwrap())
                .unwrap();
        assert_eq!(cursor_deny["permission"], "deny");
        let cursor_pass: Value =
            serde_json::from_str(&decision_json(Harness::Cursor, Behavior::Pass, None).unwrap())
                .unwrap();
        assert_eq!(cursor_pass["permission"], "ask");
    }

    #[test]
    fn names_round_trip() {
        for harness in [Harness::Claude, Harness::Codex, Harness::Cursor] {
            assert_eq!(Harness::parse(harness.as_str()), Some(harness));
        }
        assert_eq!(Harness::parse("cursor-agent"), Some(Harness::Cursor));
        assert_eq!(Harness::parse("gemini"), None);
        for behavior in [Behavior::Allow, Behavior::Deny, Behavior::Pass] {
            assert_eq!(Behavior::parse(behavior.as_str()), Some(behavior));
        }
        assert_eq!(reply_json(Behavior::Allow, None)["behavior"], "allow");
    }
}
