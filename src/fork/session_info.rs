//! FORK: the session info block (DESIGN.md §18) — what one agent session is
//! about, read from the harness's own files, locally and read-only.
//!
//! The pure half: incremental folds over the harness transcripts (Claude's
//! session `.jsonl`, Codex's rollout, Gemini's chat file) that keep the
//! first user prompt, the harness's own summary when it wrote one, and the
//! model; the path of Claude's transcript for a known session id; and the
//! small text helpers the block draws with. Nothing here touches the app,
//! tmux or the clock; the sweep in `app/fork_session_info.rs` runs the folds
//! on a background thread and the block is drawn from its cache.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// What a transcript has said so far.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TranscriptScan {
    /// The first real user prompt (not a tool result, not a slash-command
    /// echo).
    pub first_prompt: Option<String>,
    /// The harness's own title for the session, where it writes one
    /// (Claude's `summary` lines; the last one wins).
    pub summary: Option<String>,
    pub model: Option<String>,
}

impl TranscriptScan {
    /// The title the block shows: the harness's summary, else the first
    /// prompt, on one line.
    pub fn title(&self) -> Option<String> {
        self.summary
            .as_deref()
            .or(self.first_prompt.as_deref())
            .map(one_line)
            .filter(|s| !s.is_empty())
    }
}

/// Text from a message `content`: a string, or the `text` of its text
/// blocks (Claude `text`, Codex `input_text`, Gemini parts).
fn content_text(content: &Value) -> Option<String> {
    if let Some(s) = content.as_str() {
        return Some(s.to_string());
    }
    let blocks = content.as_array()?;
    let texts: Vec<&str> = blocks
        .iter()
        .filter(|b| {
            matches!(
                b.get("type").and_then(Value::as_str),
                None | Some("text") | Some("input_text")
            )
        })
        .filter_map(|b| b.get("text").and_then(Value::as_str))
        .collect();
    if texts.is_empty() {
        None
    } else {
        Some(texts.join("\n"))
    }
}

/// A user message the harness wrote on the user's behalf (Claude's
/// `<command-name>` echoes, `<local-command-stdout>`, system reminders) is
/// not a prompt.
fn is_prompt(text: &str) -> bool {
    let trimmed = text.trim_start();
    !trimmed.is_empty() && !trimmed.starts_with('<')
}

/// One line of a Claude Code session transcript
/// (`~/.claude/projects/<encoded cwd>/<session id>.jsonl`).
pub fn claude_transcript_line(scan: &mut TranscriptScan, line: &str) {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return;
    };
    match value.get("type").and_then(Value::as_str) {
        Some("summary") => {
            if let Some(summary) = value.get("summary").and_then(Value::as_str)
                && !summary.trim().is_empty()
            {
                scan.summary = Some(summary.to_string());
            }
        }
        Some("user") => {
            if scan.first_prompt.is_some()
                || value.get("isMeta").and_then(Value::as_bool) == Some(true)
            {
                return;
            }
            if let Some(text) = content_text(&value["message"]["content"])
                && is_prompt(&text)
            {
                scan.first_prompt = Some(text);
            }
        }
        Some("assistant") => {
            if let Some(model) = value["message"]["model"].as_str()
                && !model.is_empty()
            {
                scan.model = Some(model.to_string());
            }
        }
        _ => {}
    }
}

/// One line of a Codex rollout (`~/.codex/sessions/…/rollout-*.jsonl`).
pub fn codex_rollout_line(scan: &mut TranscriptScan, line: &str) {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return;
    };
    let Some(payload) = value.get("payload") else {
        return;
    };
    match value.get("type").and_then(Value::as_str) {
        Some("response_item") => {
            if scan.first_prompt.is_none()
                && payload.get("type").and_then(Value::as_str) == Some("message")
                && payload.get("role").and_then(Value::as_str) == Some("user")
                && let Some(text) = payload.get("content").and_then(content_text)
                && is_prompt(&text)
            {
                scan.first_prompt = Some(text);
            }
        }
        Some("event_msg") => {
            if scan.first_prompt.is_none()
                && payload.get("type").and_then(Value::as_str) == Some("user_message")
                && let Some(text) = payload.get("message").and_then(Value::as_str)
                && is_prompt(text)
            {
                scan.first_prompt = Some(text.to_string());
            }
        }
        Some("turn_context") | Some("session_meta") => {
            if let Some(model) = payload.get("model").and_then(Value::as_str)
                && !model.is_empty()
            {
                scan.model = Some(model.to_string());
            }
        }
        _ => {}
    }
}

/// One line of a Gemini CLI chat file (`<gemini home>/tmp/<project
/// hash>/chats/*.jsonl`).
pub fn gemini_chat_line(scan: &mut TranscriptScan, line: &str) {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return;
    };
    match value.get("type").and_then(Value::as_str) {
        Some("user") => {
            if scan.first_prompt.is_none()
                && let Some(text) = value.get("content").and_then(content_text)
                && is_prompt(&text)
            {
                scan.first_prompt = Some(text);
            }
        }
        Some("gemini") => {
            if let Some(model) = value.get("model").and_then(Value::as_str)
                && !model.is_empty()
            {
                scan.model = Some(model.to_string());
            }
        }
        _ => {}
    }
}

/// Claude Code's project directory name for a checkout: every character
/// that is not ASCII alphanumeric becomes `-`.
pub fn claude_project_dir_name(workdir: &Path) -> String {
    workdir
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// `<home>/.claude/projects/<encoded workdir>/<session id>.jsonl`.
pub fn claude_transcript_path(home: &Path, workdir: &Path, session_id: &str) -> PathBuf {
    home.join(".claude")
        .join("projects")
        .join(claude_project_dir_name(workdir))
        .join(format!("{session_id}.jsonl"))
}

/// The first line of `text`, inner whitespace collapsed.
pub fn one_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// `…/RS/hx-12`: the last two components of a worktree path, so the row
/// says which checkout without the shared prefix. A path inside `home`
/// that is short enough is shown with `~`.
pub fn shorten_worktree(path: &Path, home: Option<&Path>, width: usize) -> String {
    let full = match home.and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    };
    if full.chars().count() <= width {
        return full;
    }
    let parts: Vec<&str> = path
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();
    let tail = match parts.len() {
        0 => return full,
        1 => parts[0].to_string(),
        n => format!("{}/{}", parts[n - 2], parts[n - 1]),
    };
    format!("…/{tail}")
}

/// `12m`, `3h`, `2d`: how long ago `secs` seconds is.
pub fn age_label(secs: i64) -> String {
    if secs < 60 {
        format!("{}s", secs.max(0))
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

/// `$0.84`, `$12`: a cost with the precision its size deserves.
pub fn cost_label(usd: f64) -> String {
    if usd >= 10.0 {
        format!("${usd:.0}")
    } else {
        format!("${usd:.2}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fold(apply: fn(&mut TranscriptScan, &str), text: &str) -> TranscriptScan {
        let mut scan = TranscriptScan::default();
        for line in text.lines() {
            apply(&mut scan, line);
        }
        scan
    }

    #[test]
    fn claude_transcript_keeps_the_first_real_prompt_the_last_summary_and_the_model() {
        let scan = fold(
            claude_transcript_line,
            r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"meta"}}
{"type":"user","message":{"role":"user","content":"<command-name>/clear</command-name>"}}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"x"}]}}
{"type":"user","message":{"role":"user","content":"Fix the login redirect loop\nin the SSO flow"}}
{"type":"assistant","message":{"role":"assistant","model":"claude-opus-4","content":[{"type":"text","text":"On it"}]}}
{"type":"user","message":{"role":"user","content":"second prompt"}}
{"type":"summary","summary":"Login redirect fix","leafUuid":"a"}
not json
{"type":"summary","summary":"SSO redirect loop fixed","leafUuid":"b"}"#,
        );
        assert_eq!(
            scan.first_prompt.as_deref(),
            Some("Fix the login redirect loop\nin the SSO flow")
        );
        assert_eq!(scan.summary.as_deref(), Some("SSO redirect loop fixed"));
        assert_eq!(scan.model.as_deref(), Some("claude-opus-4"));
        assert_eq!(scan.title().as_deref(), Some("SSO redirect loop fixed"));
        let mut no_summary = scan.clone();
        no_summary.summary = None;
        assert_eq!(
            no_summary.title().as_deref(),
            Some("Fix the login redirect loop")
        );
        assert_eq!(TranscriptScan::default().title(), None);
    }

    #[test]
    fn codex_rollout_keeps_the_first_user_message_and_the_model() {
        let scan = fold(
            codex_rollout_line,
            r#"{"timestamp":"t","type":"session_meta","payload":{"id":"s","cwd":"/w"}}
{"timestamp":"t","type":"turn_context","payload":{"model":"gpt-5-codex"}}
{"timestamp":"t","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hi"}]}}
{"timestamp":"t","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Add retries to the uploader"}]}}
{"timestamp":"t","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"later"}]}}"#,
        );
        assert_eq!(
            scan.first_prompt.as_deref(),
            Some("Add retries to the uploader")
        );
        assert_eq!(scan.model.as_deref(), Some("gpt-5-codex"));
        let events = fold(
            codex_rollout_line,
            r#"{"type":"event_msg","payload":{"type":"user_message","message":"From an event"}}"#,
        );
        assert_eq!(events.first_prompt.as_deref(), Some("From an event"));
    }

    #[test]
    fn gemini_chat_keeps_the_first_user_turn_and_the_model() {
        let scan = fold(
            gemini_chat_line,
            r#"{"type":"user","content":[{"text":"Rename the module"}]}
{"type":"gemini","model":"gemini-2.5-pro","content":"ok","tokens":{"total":12}}
{"type":"user","content":"again"}"#,
        );
        assert_eq!(scan.first_prompt.as_deref(), Some("Rename the module"));
        assert_eq!(scan.model.as_deref(), Some("gemini-2.5-pro"));
    }

    #[test]
    fn the_claude_transcript_path_uses_claude_codes_project_encoding() {
        assert_eq!(
            claude_project_dir_name(Path::new("/home/u/my-project")),
            "-home-u-my-project"
        );
        assert_eq!(
            claude_transcript_path(Path::new("/h"), Path::new("/w/x.y"), "abc-1"),
            PathBuf::from("/h/.claude/projects/-w-x-y/abc-1.jsonl")
        );
    }

    #[test]
    fn text_helpers() {
        assert_eq!(one_line("  \n  Fix   the\tbug \nmore"), "Fix the bug");
        assert_eq!(one_line("   "), "");
        let home = Path::new("/home/u");
        let wt = Path::new("/home/u/.local/share/ai_workstation/worktrees/RS/hx-12");
        assert_eq!(shorten_worktree(wt, Some(home), 30), "…/RS/hx-12");
        assert_eq!(
            shorten_worktree(wt, Some(home), 80),
            "~/.local/share/ai_workstation/worktrees/RS/hx-12"
        );
        assert_eq!(
            shorten_worktree(Path::new("/srv/repo"), Some(home), 30),
            "/srv/repo"
        );
        assert_eq!(shorten_worktree(Path::new("/x"), None, 1), "…/x");
        assert_eq!(age_label(5), "5s");
        assert_eq!(age_label(725), "12m");
        assert_eq!(age_label(7_300), "2h");
        assert_eq!(age_label(200_000), "2d");
        assert_eq!(cost_label(0.8412), "$0.84");
        assert_eq!(cost_label(12.4), "$12");
    }
}
