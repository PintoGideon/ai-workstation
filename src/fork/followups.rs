//! FORK: queued follow-ups (P7, DESIGN.md §16) — the harness contracts.
//!
//! A follow-up is an instruction the user typed for a busy agent, kept in a
//! per-session FIFO (persisted in `aiw.db`, `src/db/followups.rs`). It is
//! delivered only when the harness itself asks at the end of a turn: the
//! Stop hook (`scripts/stop-hook.sh` → `aiw stop-hook`) sends
//! `followup_take` and waits ≤ 2 s on the same connection for
//! `{"followup": "<text>" | null}`; with a text it prints the harness's
//! continuation JSON, which becomes the agent's next turn. The task prompt is
//! never sent on its own.
//!
//! Loop caps (docs/development/harness-hooks-reference.md): Claude stops
//! after 8 consecutive continuations (`CLAUDE_CODE_STOP_HOOK_BLOCK_CAP`);
//! Cursor's `loop_limit` is 5 by default, so AIW's `stop` entry sets 20. The
//! queue draining itself bounds the loop either way; `stop_hook_active` is
//! logged, not acted on.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The IPC message type the Stop hook sends.
pub const TAKE_TYPE: &str = "followup_take";

/// The automation action that queues one (`aiw automation queue`).
pub const QUEUE_ACTION: &str = "queue_followup";

/// How long the Stop hook waits for the dashboard's answer.
pub const TAKE_WAIT_MILLIS: u64 = 2_000;

/// The harnesses whose Stop hook can continue with a new instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StopHarness {
    Claude,
    Codex,
    Gemini,
    Cursor,
}

impl StopHarness {
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            "gemini" => Some(Self::Gemini),
            "cursor" | "cursor-agent" | "agent" => Some(Self::Cursor),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::Cursor => "cursor",
        }
    }
}

/// One queued instruction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Followup {
    pub id: i64,
    pub session_id: String,
    pub position: i64,
    pub text: String,
    pub created_at: String,
}

impl Followup {
    /// The first line, cut to `width` chars, for a list row.
    pub fn preview(&self, width: usize) -> String {
        let first = self.text.lines().next().unwrap_or("").trim();
        let more = self.text.lines().count() > 1;
        let mut shown: String = first.chars().take(width).collect();
        if shown.chars().count() < first.chars().count() || more {
            shown.push('…');
        }
        shown
    }
}

/// What the harness is told to do next.
pub fn continuation_json(harness: StopHarness, text: &str) -> String {
    match harness {
        StopHarness::Claude | StopHarness::Codex | StopHarness::Gemini => {
            json!({ "decision": "block", "reason": text }).to_string()
        }
        StopHarness::Cursor => json!({ "followup_message": text }).to_string(),
    }
}

/// What the hook prints when nothing is queued: Codex requires JSON from a
/// Stop hook that exits 0, the others accept silence.
pub fn no_followup_output(harness: StopHarness) -> Option<String> {
    match harness {
        StopHarness::Codex => Some("{}".to_string()),
        _ => None,
    }
}

/// The hook's output for the dashboard's answer.
pub fn stop_hook_output(harness: StopHarness, answer: Option<&Value>) -> Option<String> {
    let text = answer
        .and_then(|a| a.get("followup"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty());
    match text {
        Some(text) => Some(continuation_json(harness, text)),
        None => no_followup_output(harness),
    }
}

/// The dashboard's answer.
pub fn take_reply(text: Option<&str>) -> Value {
    json!({ "followup": text })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuation_json_matches_each_harness() {
        for harness in [StopHarness::Claude, StopHarness::Codex, StopHarness::Gemini] {
            let value: Value =
                serde_json::from_str(&continuation_json(harness, "run the tests")).unwrap();
            assert_eq!(value["decision"], "block", "{harness:?}");
            assert_eq!(value["reason"], "run the tests");
        }
        let cursor: Value =
            serde_json::from_str(&continuation_json(StopHarness::Cursor, "next")).unwrap();
        assert_eq!(cursor["followup_message"], "next");
    }

    #[test]
    fn nothing_queued_prints_json_only_for_codex() {
        assert_eq!(
            no_followup_output(StopHarness::Codex).as_deref(),
            Some("{}")
        );
        assert!(no_followup_output(StopHarness::Claude).is_none());
        assert!(stop_hook_output(StopHarness::Claude, None).is_none());
        assert!(stop_hook_output(StopHarness::Claude, Some(&json!({"followup": null}))).is_none());
        assert!(stop_hook_output(StopHarness::Gemini, Some(&json!({"followup": "  "}))).is_none());
        let out = stop_hook_output(StopHarness::Cursor, Some(&json!({"followup": "go"}))).unwrap();
        assert!(out.contains("followup_message"));
        assert_eq!(take_reply(None)["followup"], Value::Null);
        assert_eq!(take_reply(Some("x"))["followup"], "x");
    }

    #[test]
    fn previews_are_one_line() {
        let followup = Followup {
            id: 1,
            session_id: "s".into(),
            position: 0,
            text: "first line is long enough\nsecond".into(),
            created_at: String::new(),
        };
        assert_eq!(followup.preview(10), "first line…");
        assert_eq!(followup.preview(80), "first line is long enough…");
        assert_eq!(
            StopHarness::parse("cursor-agent"),
            Some(StopHarness::Cursor)
        );
        assert_eq!(StopHarness::Gemini.as_str(), "gemini");
    }
}
