//! FORK: per-session context and cost, read locally (P4, DESIGN.md §16).
//!
//! - **Claude Code** runs `scripts/status-line.sh` as its `statusLine`
//!   command (passed at launch with `--settings`); `aiw status-line` reads
//!   the status JSON on stdin, sends a `status_line` message to the dashboard
//!   and prints one short line for Claude's own footer. The raw
//!   `context_window` object rides along so the existing context collector
//!   gets a direct sample instead of estimating from the transcript.
//! - **Codex** writes `token_count` and `turn_context` events to its rollout
//!   file; [`codex_rollout_line`] folds them (incrementally, through
//!   `line_fold`) into the model, the session's total tokens and the context
//!   use of the last request against `model_context_window`.
//! - **Gemini** keeps `~/.gemini/tmp/<sha256(project root)>/chats/*.jsonl`;
//!   the newest file's last `gemini` record gives `tokens.total` (no window
//!   is reported, so only the count is shown).
//! - **Cursor** reports nothing usable; nothing is shown.
//!
//! Everything here is pure or reads files; nothing touches tmux and nothing
//! runs while drawing.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::hook_payload::HookEnv;

/// The IPC message type `aiw status-line` sends.
pub const STATUS_LINE_TYPE: &str = "status_line";

/// Where a session's numbers came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TelemetrySource {
    StatusLine,
    CodexRollout,
    GeminiChat,
}

/// What one agent session has reported, all optional: a harness says what
/// it can and the UI shows what it knows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionTelemetry {
    pub source: TelemetrySource,
    pub model: Option<String>,
    /// Context used, as a whole percentage of the window.
    pub context_pct: Option<u8>,
    pub context_window: Option<u64>,
    pub context_used_tokens: Option<u64>,
    /// Session total (cumulative) tokens, where the harness reports one.
    pub total_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
    pub lines_added: Option<u64>,
    pub lines_removed: Option<u64>,
    /// Claude's `rate_limits` object, as sent (five_hour / seven_day).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limits: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<Value>,
    /// Claude's raw `context_window`, fed to the context collector.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window_raw: Option<Value>,
    pub updated_at: DateTime<Utc>,
}

impl SessionTelemetry {
    fn empty(source: TelemetrySource) -> Self {
        Self {
            source,
            model: None,
            context_pct: None,
            context_window: None,
            context_used_tokens: None,
            total_tokens: None,
            cost_usd: None,
            lines_added: None,
            lines_removed: None,
            rate_limits: None,
            pr: None,
            context_window_raw: None,
            updated_at: Utc::now(),
        }
    }

    /// `42% ctx`, `12.3k tokens` (no window known), or nothing.
    pub fn context_label(&self) -> Option<String> {
        if let Some(pct) = self.context_pct {
            return Some(format!("{pct}% ctx"));
        }
        self.context_used_tokens
            .or(self.total_tokens)
            .map(|tokens| {
                format!(
                    "{} tokens",
                    crate::token_tracking::format_token_count(tokens)
                )
            })
    }

    /// The payload the context collector reads as Claude's status line.
    pub fn claude_runtime_payload(&self, session_id: Option<&str>) -> Option<Value> {
        let context = self.context_window_raw.clone()?;
        let mut payload = json!({ "context_window": context });
        if let Some(id) = session_id {
            payload["session_id"] = Value::String(id.to_string());
        }
        Some(payload)
    }
}

fn whole_percent(used: u64, window: u64) -> Option<u8> {
    (window > 0).then(|| ((u128::from(used) * 100 / u128::from(window)).min(100)) as u8)
}

fn u64_of(value: Option<&Value>) -> Option<u64> {
    let value = value?;
    value.as_u64().or_else(|| {
        let number = value.as_f64()?;
        (number.is_finite() && number >= 0.0).then_some(number.round() as u64)
    })
}

fn f64_of(value: Option<&Value>) -> Option<f64> {
    value?.as_f64().filter(|n| n.is_finite())
}

fn text_of(value: Option<&Value>) -> Option<String> {
    value?
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// What Claude's status JSON says (see the hooks reference for the shape).
pub fn parse_claude_status(status: &Value) -> SessionTelemetry {
    let mut telemetry = SessionTelemetry::empty(TelemetrySource::StatusLine);
    telemetry.model = text_of(status.pointer("/model/display_name"))
        .or_else(|| text_of(status.pointer("/model/id")));
    if let Some(context) = status.get("context_window").filter(|c| c.is_object()) {
        telemetry.context_window = u64_of(context.get("context_window_size"));
        telemetry.context_used_tokens = u64_of(context.get("total_input_tokens"));
        telemetry.context_pct = u64_of(context.get("used_percentage"))
            .map(|p| p.min(100) as u8)
            .or_else(|| whole_percent(telemetry.context_used_tokens?, telemetry.context_window?));
        telemetry.total_tokens = match (
            u64_of(context.get("total_input_tokens")),
            u64_of(context.get("total_output_tokens")),
        ) {
            (Some(input), Some(output)) => Some(input + output),
            (Some(input), None) => Some(input),
            _ => None,
        };
        telemetry.context_window_raw = Some(context.clone());
    }
    if let Some(cost) = status.get("cost") {
        telemetry.cost_usd = f64_of(cost.get("total_cost_usd"));
        telemetry.lines_added = u64_of(cost.get("total_lines_added"));
        telemetry.lines_removed = u64_of(cost.get("total_lines_removed"));
    }
    telemetry.rate_limits = status.get("rate_limits").filter(|v| v.is_object()).cloned();
    telemetry.pr = status.get("pr").filter(|v| v.is_object()).cloned();
    telemetry
}

/// The one line printed into Claude's footer: `Opus 4 · ctx 42% · $0.84`.
pub fn footer_line(telemetry: &SessionTelemetry) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Some(model) = &telemetry.model {
        parts.push(model.clone());
    }
    if let Some(pct) = telemetry.context_pct {
        parts.push(format!("ctx {pct}%"));
    }
    if let Some(cost) = telemetry.cost_usd {
        parts.push(format!("${cost:.2}"));
    }
    if let (Some(added), Some(removed)) = (telemetry.lines_added, telemetry.lines_removed)
        && added + removed > 0
    {
        parts.push(format!("+{added} −{removed}"));
    }
    if parts.is_empty() {
        "AIW".to_string()
    } else {
        parts.join(" · ")
    }
}

/// The IPC message for a status line, with AIW's session identity.
pub fn status_line_message(env: &HookEnv, status: &Value, telemetry: &SessionTelemetry) -> Value {
    let mut message = json!({
        "type": STATUS_LINE_TYPE,
        "session_id": status.get("session_id").cloned().unwrap_or(Value::Null),
        "model": telemetry.model,
        "context_used_pct": telemetry.context_pct,
        "context_window": telemetry.context_window,
        "context_used_tokens": telemetry.context_used_tokens,
        "total_tokens": telemetry.total_tokens,
        "cost_usd": telemetry.cost_usd,
        "lines_added": telemetry.lines_added,
        "lines_removed": telemetry.lines_removed,
        "rate_limits": telemetry.rate_limits,
        "pr": telemetry.pr,
        "context_window_raw": telemetry.context_window_raw,
        "cwd": status.get("cwd").cloned().unwrap_or(Value::Null),
    });
    if let Some(session) = env.aiw_tmux_session.clone().or(env.aiw_session.clone()) {
        message["aiw_session"] = Value::String(session.clone());
        message["aiw_tmux_session"] = Value::String(session);
    }
    if let Some(id) = &env.aiw_feature_session_id {
        message["aiw_feature_session_id"] = Value::String(id.clone());
    }
    message
}

/// The telemetry a `status_line` message carries (the dashboard side).
pub fn telemetry_from_message(message: &Value) -> SessionTelemetry {
    let mut telemetry = SessionTelemetry::empty(TelemetrySource::StatusLine);
    telemetry.model = text_of(message.get("model"));
    telemetry.context_pct = u64_of(message.get("context_used_pct")).map(|p| p.min(100) as u8);
    telemetry.context_window = u64_of(message.get("context_window"));
    telemetry.context_used_tokens = u64_of(message.get("context_used_tokens"));
    telemetry.total_tokens = u64_of(message.get("total_tokens"));
    telemetry.cost_usd = f64_of(message.get("cost_usd"));
    telemetry.lines_added = u64_of(message.get("lines_added"));
    telemetry.lines_removed = u64_of(message.get("lines_removed"));
    telemetry.rate_limits = message
        .get("rate_limits")
        .filter(|v| v.is_object())
        .cloned();
    telemetry.pr = message.get("pr").filter(|v| v.is_object()).cloned();
    telemetry.context_window_raw = message
        .get("context_window_raw")
        .filter(|v| v.is_object())
        .cloned();
    telemetry
}

/// The user's own `statusLine.command` from Claude's user settings (read
/// only), so `aiw status-line` can run it and print its output instead of
/// AIW's line. `None` when there is none, or it is AIW's own script.
pub fn user_status_line_command(settings_path: &Path) -> Option<String> {
    let settings: Value =
        serde_json::from_str(&std::fs::read_to_string(settings_path).ok()?).ok()?;
    let command = text_of(settings.pointer("/statusLine/command"))?;
    if settings.pointer("/statusLine/type").and_then(Value::as_str) == Some("command")
        && !command.contains("status-line.sh")
    {
        Some(command)
    } else {
        None
    }
}

// ---- Codex ----------------------------------------------------------------

/// What a Codex rollout says so far (folded line by line, incrementally).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CodexRolloutScan {
    pub model: Option<String>,
    pub total_tokens: Option<u64>,
    pub last_request_tokens: Option<u64>,
    pub context_window: Option<u64>,
}

/// Fold one rollout line.
pub fn codex_rollout_line(scan: &mut CodexRolloutScan, line: &str) {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return;
    };
    let event_type = value.get("type").and_then(Value::as_str);
    let payload = value.get("payload");
    if event_type == Some("turn_context")
        && let Some(model) = text_of(payload.and_then(|p| p.get("model")))
    {
        scan.model = Some(model);
    }
    if event_type != Some("event_msg")
        || payload.and_then(|p| p.get("type")).and_then(Value::as_str) != Some("token_count")
    {
        return;
    }
    let Some(info) = payload
        .and_then(|p| p.get("info"))
        .filter(|i| i.is_object())
    else {
        return;
    };
    if let Some(total) = u64_of(info.pointer("/total_token_usage/total_tokens")) {
        scan.total_tokens = Some(total);
    }
    if let Some(last) = info.get("last_token_usage") {
        scan.last_request_tokens = u64_of(last.get("total_tokens")).or_else(|| {
            Some(u64_of(last.get("input_tokens"))? + u64_of(last.get("output_tokens")).unwrap_or(0))
        });
    }
    if let Some(window) = u64_of(info.get("model_context_window")) {
        scan.context_window = Some(window);
    }
}

impl CodexRolloutScan {
    pub fn telemetry(&self) -> Option<SessionTelemetry> {
        if self.model.is_none() && self.total_tokens.is_none() && self.last_request_tokens.is_none()
        {
            return None;
        }
        let mut telemetry = SessionTelemetry::empty(TelemetrySource::CodexRollout);
        telemetry.model = self.model.clone();
        telemetry.total_tokens = self.total_tokens;
        telemetry.context_window = self.context_window;
        telemetry.context_used_tokens = self.last_request_tokens;
        telemetry.context_pct = match (self.last_request_tokens, self.context_window) {
            (Some(used), Some(window)) => whole_percent(used, window),
            _ => None,
        };
        Some(telemetry)
    }
}

// ---- Gemini ---------------------------------------------------------------

/// Gemini's home: `$GEMINI_CLI_HOME`, else `~/.gemini` under `home`.
pub fn gemini_home(home: &Path) -> PathBuf {
    std::env::var_os("GEMINI_CLI_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| home.join(".gemini"))
}

/// `sha256(project root)` as hex: gemini-cli's `getProjectHash`.
pub fn gemini_project_hash(project_root: &Path) -> String {
    let mut hasher = Sha256::new();
    hasher.update(project_root.to_string_lossy().as_bytes());
    format!("{:x}", hasher.finalize())
}

/// `<gemini home>/tmp/<project hash>/chats`, when it exists.
pub fn gemini_chats_dir(gemini_home: &Path, project_root: &Path) -> Option<PathBuf> {
    let dir = gemini_home
        .join("tmp")
        .join(gemini_project_hash(project_root))
        .join("chats");
    dir.is_dir().then_some(dir)
}

/// The newest `*.jsonl` session file in `chats`.
pub fn gemini_newest_session(chats: &Path) -> Option<PathBuf> {
    std::fs::read_dir(chats)
        .ok()?
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            entry.path().extension().and_then(|e| e.to_str()) == Some("jsonl")
                && entry.file_type().is_ok_and(|t| t.is_file())
        })
        .filter_map(|entry| {
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, entry.path()))
        })
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
}

/// The last `gemini` record so far: `tokens.total` and `model`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct GeminiChatScan {
    pub model: Option<String>,
    pub total_tokens: Option<u64>,
}

pub fn gemini_chat_line(scan: &mut GeminiChatScan, line: &str) {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return;
    };
    if value.get("type").and_then(Value::as_str) != Some("gemini") {
        return;
    }
    if let Some(model) = text_of(value.get("model")) {
        scan.model = Some(model);
    }
    if let Some(total) = u64_of(value.pointer("/tokens/total")) {
        scan.total_tokens = Some(total);
    }
}

impl GeminiChatScan {
    pub fn telemetry(&self) -> Option<SessionTelemetry> {
        let total = self.total_tokens?;
        let mut telemetry = SessionTelemetry::empty(TelemetrySource::GeminiChat);
        telemetry.model = self.model.clone();
        telemetry.context_used_tokens = Some(total);
        Some(telemetry)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status() -> Value {
        json!({
            "session_id": "c-1", "cwd": "/w",
            "model": {"id": "claude-opus-4", "display_name": "Opus 4"},
            "context_window": {
                "total_input_tokens": 84_000, "total_output_tokens": 6_000,
                "context_window_size": 200_000, "used_percentage": 42, "remaining_percentage": 58,
                "current_usage": {"input_tokens": 4_000, "cache_read_input_tokens": 80_000, "cache_creation_input_tokens": 0}
            },
            "cost": {"total_cost_usd": 0.8412, "total_lines_added": 61, "total_lines_removed": 3},
            "rate_limits": {"five_hour": {"used_percentage": 12, "resets_at": 1}, "seven_day": {"used_percentage": 30}}
        })
    }

    #[test]
    fn claude_status_is_parsed_and_the_footer_is_one_short_line() {
        let telemetry = parse_claude_status(&status());
        assert_eq!(telemetry.model.as_deref(), Some("Opus 4"));
        assert_eq!(telemetry.context_pct, Some(42));
        assert_eq!(telemetry.context_window, Some(200_000));
        assert_eq!(telemetry.context_used_tokens, Some(84_000));
        assert_eq!(telemetry.total_tokens, Some(90_000));
        assert_eq!(telemetry.cost_usd, Some(0.8412));
        assert_eq!(telemetry.lines_added, Some(61));
        assert!(telemetry.rate_limits.is_some());
        assert_eq!(footer_line(&telemetry), "Opus 4 · ctx 42% · $0.84 · +61 −3");
        assert_eq!(telemetry.context_label().as_deref(), Some("42% ctx"));

        // Without `used_percentage` the percentage is computed.
        let mut status = status();
        status["context_window"]
            .as_object_mut()
            .unwrap()
            .remove("used_percentage");
        assert_eq!(parse_claude_status(&status).context_pct, Some(42));
        assert_eq!(footer_line(&parse_claude_status(&json!({}))), "AIW");
    }

    #[test]
    fn the_message_round_trips_and_feeds_the_context_collector() {
        let env = HookEnv {
            aiw_session: Some("aiw-hx-12".into()),
            aiw_feature_session_id: Some("fs-1".into()),
            aiw_tmux_session: None,
            aiw_tmux_window: None,
        };
        let telemetry = parse_claude_status(&status());
        let message = status_line_message(&env, &status(), &telemetry);
        assert_eq!(message["type"], "status_line");
        assert_eq!(message["aiw_feature_session_id"], "fs-1");
        assert_eq!(message["aiw_session"], "aiw-hx-12");
        assert_eq!(message["context_used_pct"], 42);
        let back = telemetry_from_message(&message);
        assert_eq!(back.context_pct, Some(42));
        assert_eq!(back.cost_usd, Some(0.8412));
        assert_eq!(back.model.as_deref(), Some("Opus 4"));
        let payload = back.claude_runtime_payload(Some("c-1")).unwrap();
        assert_eq!(payload["session_id"], "c-1");
        assert_eq!(payload["context_window"]["context_window_size"], 200_000);
    }

    #[test]
    fn the_users_own_status_line_is_found_but_not_aiws() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert!(user_status_line_command(&path).is_none());
        std::fs::write(
            &path,
            r#"{"statusLine":{"type":"command","command":"~/bin/mine.sh"}}"#,
        )
        .unwrap();
        assert_eq!(
            user_status_line_command(&path).as_deref(),
            Some("~/bin/mine.sh")
        );
        std::fs::write(
            &path,
            r#"{"statusLine":{"type":"command","command":"/x/hooks/status-line.sh"}}"#,
        )
        .unwrap();
        assert!(user_status_line_command(&path).is_none());
    }

    #[test]
    fn codex_rollout_lines_give_model_total_and_context_percent() {
        let mut scan = CodexRolloutScan::default();
        codex_rollout_line(&mut scan, "not json");
        codex_rollout_line(
            &mut scan,
            r#"{"type":"turn_context","payload":{"model":"gpt-5-codex","cwd":"/w"}}"#,
        );
        codex_rollout_line(
            &mut scan,
            r#"{"type":"event_msg","payload":{"type":"token_count","info":{"total_token_usage":{"total_tokens":900000},"last_token_usage":{"input_tokens":79000,"output_tokens":1000,"total_tokens":80000},"model_context_window":200000}}}"#,
        );
        let telemetry = scan.telemetry().unwrap();
        assert_eq!(telemetry.model.as_deref(), Some("gpt-5-codex"));
        assert_eq!(telemetry.total_tokens, Some(900_000));
        assert_eq!(telemetry.context_pct, Some(40));
        assert_eq!(telemetry.context_label().as_deref(), Some("40% ctx"));
        assert!(CodexRolloutScan::default().telemetry().is_none());
    }

    #[test]
    fn gemini_chats_live_under_the_sha256_of_the_project_root() {
        let home = tempfile::tempdir().unwrap();
        let gemini = home.path().join(".gemini");
        let project = Path::new("/home/u/repo");
        assert_eq!(gemini_project_hash(project), gemini_project_hash(project));
        assert_eq!(gemini_project_hash(project).len(), 64);
        assert!(gemini_chats_dir(&gemini, project).is_none());
        let chats = gemini
            .join("tmp")
            .join(gemini_project_hash(project))
            .join("chats");
        std::fs::create_dir_all(&chats).unwrap();
        assert_eq!(gemini_chats_dir(&gemini, project), Some(chats.clone()));
        assert!(gemini_newest_session(&chats).is_none());
        let older = chats.join("session-2026-10-10T09-00-aaaaaaaa.jsonl");
        std::fs::write(&older, "{}\n").unwrap();
        let newer = chats.join("session-2026-10-10T10-00-bbbbbbbb.jsonl");
        std::fs::write(&newer, "{}\n").unwrap();
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::open(&newer)
            .unwrap()
            .set_modified(later)
            .unwrap();
        assert_eq!(gemini_newest_session(&chats), Some(newer));

        let mut scan = GeminiChatScan::default();
        gemini_chat_line(
            &mut scan,
            r#"{"sessionId":"s","projectHash":"h","startTime":"t"}"#,
        );
        gemini_chat_line(&mut scan, r#"{"type":"user","content":"hi"}"#);
        gemini_chat_line(
            &mut scan,
            r#"{"type":"gemini","model":"gemini-2.5-pro","content":"x","tokens":{"input":100,"output":20,"total":12345}}"#,
        );
        let telemetry = scan.telemetry().unwrap();
        assert_eq!(telemetry.model.as_deref(), Some("gemini-2.5-pro"));
        assert_eq!(telemetry.context_label().as_deref(), Some("12.3k tokens"));
        assert!(telemetry.context_pct.is_none());
    }
}
