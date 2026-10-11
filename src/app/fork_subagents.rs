//! FORK: subagent activity on the dashboard (P5, DESIGN.md §16).
//!
//! Claude's `SubagentStart` / `SubagentStop`, Codex's `SubagentStart` /
//! `SubagentStop` and Cursor's `subagentStart` / `subagentStop` run
//! `scripts/subagent.sh` → `aiw subagent start|stop`, which sends a
//! `subagent` message. The app keeps a count per feature session; the
//! badge's working label says `working 6m · 2 subagents`. The count is
//! cleared when the turn ends (`thinking-stop`) or the session ends, so a
//! missed stop can never leave a stale number. Gemini has no subagent hooks.

use serde_json::Value;

use super::App;

/// The IPC message type `aiw subagent` sends.
pub const SUBAGENT_TYPE: &str = "subagent";

fn text(raw: &Value, key: &str) -> Option<String> {
    raw.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

impl App {
    /// Subagents running under `session_id` right now.
    pub fn subagent_count(&self, session_id: &str) -> usize {
        self.subagent_counts.get(session_id).copied().unwrap_or(0)
    }

    /// The feature session a hook message is about: its id when the hook
    /// ran with `AIW_FEATURE_SESSION_ID`, else the feature's window of the
    /// named harness.
    fn subagent_session_id(&self, raw: &Value) -> Option<String> {
        if let Some(id) = text(raw, "aiw_feature_session_id") {
            return Some(id);
        }
        let aiw_session = text(raw, "aiw_tmux_session").or_else(|| text(raw, "aiw_session"));
        let cwd = text(raw, "cwd").unwrap_or_default();
        let (pi, fi) = self.fork_feature_for(aiw_session.as_deref(), &cwd)?;
        let harness = text(raw, "harness").unwrap_or_default();
        let sessions = &self.store.projects[pi].features[fi].sessions;
        sessions
            .iter()
            .find(|s| {
                matches!(
                    (&s.kind, harness.as_str()),
                    (crate::project::SessionKind::Claude, "claude")
                        | (crate::project::SessionKind::Codex, "codex")
                        | (crate::project::SessionKind::Cursor, "cursor")
                )
            })
            .or_else(|| sessions.iter().find(|s| s.kind.is_agent_harness()))
            .map(|s| s.id.clone())
    }

    /// A `subagent` message. Returns `false` when `raw` is not one.
    pub(crate) fn handle_subagent_message(&mut self, raw: &Value) -> bool {
        if raw.get("type").and_then(Value::as_str) != Some(SUBAGENT_TYPE) {
            return false;
        }
        let Some(session_id) = self.subagent_session_id(raw) else {
            self.log_debug("subagent", "subagent event matched no session".into());
            return true;
        };
        let agent_type = text(raw, "agent_type").unwrap_or_else(|| "subagent".into());
        match text(raw, "event").as_deref() {
            Some("start") => {
                let count = self.subagent_counts.entry(session_id.clone()).or_insert(0);
                *count += 1;
                let count = *count;
                self.log_debug(
                    "subagent",
                    format!("{session_id}: {agent_type} started ({count} running)"),
                );
            }
            Some("stop") => {
                let left = match self.subagent_counts.get_mut(&session_id) {
                    Some(count) => {
                        *count = count.saturating_sub(1);
                        *count
                    }
                    None => 0,
                };
                if left == 0 {
                    self.subagent_counts.remove(&session_id);
                }
                self.log_debug(
                    "subagent",
                    format!("{session_id}: {agent_type} stopped ({left} running)"),
                );
            }
            other => self.log_debug("subagent", format!("ignored subagent event {other:?}")),
        }
        true
    }

    /// The turn ended (or the session did): nothing is running under it.
    pub(crate) fn clear_subagents(&mut self, session_id: &str) {
        if self.subagent_counts.remove(session_id).is_some() {
            self.log_debug(
                "subagent",
                format!("{session_id}: count cleared at the turn's end"),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::badges::BadgeKind;
    use crate::project::SessionKind;
    use serde_json::json;

    fn event(session_id: &str, event: &str) -> Value {
        json!({
            "type": "subagent", "event": event, "harness": "claude", "agent_type": "Explore",
            "aiw_session": "aiw-hx-12", "aiw_feature_session_id": session_id, "cwd": "/tmp"
        })
    }

    #[test]
    fn starts_and_stops_count_per_session_and_show_on_the_working_badge() {
        let mut app =
            crate::app::fork_workspace::workspace_app(&[SessionKind::Claude, SessionKind::Codex]);
        let claude = app.store.projects[0].features[0].sessions[0].id.clone();
        app.thinking_features.insert("aiw-hx-12".into());
        assert!(app.handle_subagent_message(&event(&claude, "start")));
        assert!(app.handle_subagent_message(&event(&claude, "start")));
        assert_eq!(app.subagent_count(&claude), 2);
        let badge = app.session_badge_at(0, 0, 0).unwrap();
        assert_eq!(badge.kind, BadgeKind::Working);
        assert_eq!(
            badge.label(chrono::Utc::now()),
            "working 0:00 · 2 subagents"
        );
        assert_eq!(app.session_badge_at(0, 0, 1).unwrap().subagents, 0);
        assert!(app.handle_subagent_message(&event(&claude, "stop")));
        assert_eq!(
            app.session_badge_at(0, 0, 0)
                .unwrap()
                .label(chrono::Utc::now()),
            "working 0:00 · 1 subagent"
        );
        assert!(app.handle_subagent_message(&event(&claude, "stop")));
        assert_eq!(app.subagent_count(&claude), 0);
        assert!(
            app.handle_subagent_message(&event(&claude, "stop")),
            "never negative"
        );
        assert_eq!(app.subagent_count(&claude), 0);
        assert!(!app.handle_subagent_message(&json!({"type": "stop"})));
    }

    #[test]
    fn a_subagent_event_is_a_new_frame() {
        let mut app = crate::app::fork_workspace::workspace_app(&[SessionKind::Claude]);
        let claude = app.store.projects[0].features[0].sessions[0].id.clone();
        let before = app.redraw_signature();
        app.handle_subagent_message(&event(&claude, "start"));
        assert_ne!(before, app.redraw_signature());
        let one = app.redraw_signature();
        app.queue_followup(&claude, "next").unwrap();
        assert_ne!(one, app.redraw_signature());
    }

    #[test]
    fn the_turn_end_clears_a_count_a_missed_stop_left_behind() {
        let mut app = crate::app::fork_workspace::workspace_app(&[SessionKind::Claude]);
        let claude = app.store.projects[0].features[0].sessions[0].id.clone();
        app.handle_subagent_message(&event(&claude, "start"));
        assert_eq!(app.subagent_count(&claude), 1);
        app.handle_ipc_message_value(json!({
            "type": "thinking-stop", "session_id": "aiw-hx-12", "aiw_feature_session_id": claude
        }));
        assert_eq!(app.subagent_count(&claude), 0);
    }

    #[test]
    fn without_the_session_id_the_count_lands_on_the_harness_window() {
        let mut app =
            crate::app::fork_workspace::workspace_app(&[SessionKind::Codex, SessionKind::Claude]);
        let claude = app.store.projects[0].features[0].sessions[1].id.clone();
        let mut raw = event("x", "start");
        raw.as_object_mut()
            .unwrap()
            .remove("aiw_feature_session_id");
        app.handle_subagent_message(&raw);
        assert_eq!(app.subagent_count(&claude), 1);
    }
}
