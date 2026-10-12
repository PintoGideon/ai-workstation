//! FORK: the app side of the newer harness hook events (DESIGN.md §19.1).
//!
//! `aiw hook-event` sends a `hook-event` message; this folds it into the
//! session's [`HookEventState`], which the badge and the info block read.
//! A turn that died on an API error (`StopFailure`) or a session that ended
//! clears the working markers the same way a normal turn end does, so the row
//! never keeps saying "working" for an agent that stopped.

use chrono::Utc;
use serde_json::Value;

use super::App;
use crate::fork::hook_events::{HOOK_EVENT_TYPE, HookEvent, HookEventState};

impl App {
    /// What the hooks have told AIW about `session_id`.
    pub fn hook_state(&self, session_id: &str) -> Option<&HookEventState> {
        self.hook_events.get(session_id)
    }

    /// A new turn started in `session_id` (`thinking-start`).
    pub(crate) fn hook_turn_started(&mut self, session_id: &str) {
        if let Some(state) = self.hook_events.get_mut(session_id) {
            state.turn_started();
        }
    }

    /// Whether any session of the feature at `(pi, fi)` still has background
    /// tasks or `/loop` schedules alive — such a feature is not dormant.
    pub fn feature_has_background_work(&self, pi: usize, fi: usize) -> bool {
        self.store
            .projects
            .get(pi)
            .and_then(|p| p.features.get(fi))
            .is_some_and(|feature| {
                feature.sessions.iter().any(|s| {
                    self.hook_state(&s.id)
                        .is_some_and(|state| state.background_work() > 0)
                })
            })
    }

    /// A `hook-event` message. Returns `false` when `raw` is not one.
    pub(crate) fn handle_hook_event_message(&mut self, raw: &Value) -> bool {
        if raw.get("type").and_then(Value::as_str) != Some(HOOK_EVENT_TYPE) {
            return false;
        }
        let Some(event) = HookEvent::parse(raw) else {
            self.log_debug(
                "hook-event",
                format!(
                    "ignored hook event {:?}",
                    raw.get("kind").and_then(Value::as_str)
                ),
            );
            return true;
        };
        let Some(session_id) = self.subagent_session_id(raw) else {
            self.log_debug("hook-event", format!("{event:?} matched no session"));
            return true;
        };
        // §19.8: a sign of life for the watchdog.
        self.watchdog_saw_activity(&session_id);
        // §19.3: the agent is up — a prompt waiting for it can go.
        if matches!(event, HookEvent::SessionStart) {
            self.fan_out_session_ready(&session_id);
            // §19.5: Claude reports its inbox socket here.
            self.messaging_session_started(&session_id, raw);
        }
        let session_end = matches!(event, HookEvent::SessionEnd);
        self.log_debug("hook-event", format!("{session_id}: {event:?}"));
        let turn_ended = self
            .hook_events
            .entry(session_id.clone())
            .or_default()
            .apply(event, Utc::now());
        if turn_ended {
            self.ipc_thinking_feature_sessions.remove(&session_id);
            self.ipc_tool_feature_sessions.remove(&session_id);
            if let Some(harness_session) = raw.get("session_id").and_then(Value::as_str) {
                self.ipc_thinking_sessions.remove(harness_session);
                self.ipc_tool_sessions.remove(harness_session);
            }
            self.clear_subagents(&session_id);
        }
        if session_end {
            self.hook_events.remove(&session_id);
            self.forget_inbox(&session_id); // §19.5
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::badges::BadgeKind;
    use crate::project::SessionKind;
    use serde_json::json;

    fn event(session_id: &str, kind: &str, extra: Value) -> Value {
        let mut raw = json!({
            "type": "hook-event", "kind": kind, "harness": "claude",
            "aiw_session": "aiw-hx-12", "aiw_feature_session_id": session_id, "cwd": "/tmp",
            "session_id": "claude-uuid"
        });
        for (k, v) in extra.as_object().unwrap() {
            raw[k] = v.clone();
        }
        raw
    }

    fn app() -> (App, String) {
        let app = crate::app::fork_workspace::workspace_app(&[SessionKind::Claude]);
        let id = app.store.projects[0].features[0].sessions[0].id.clone();
        (app, id)
    }

    #[test]
    fn a_compaction_shows_on_the_idle_row_until_the_next_turn() {
        let (mut app, id) = app();
        assert!(app.handle_ipc_message_value_returns(event(&id, "post-compact", json!({}))));
        let badge = app.session_badge_at(0, 0, 0).unwrap();
        assert_eq!(badge.kind, BadgeKind::Idle);
        assert_eq!(badge.label_compact(), "compacted");
        app.handle_ipc_message_value(json!({
            "type": "thinking-start", "session_id": "claude-uuid", "aiw_feature_session_id": id
        }));
        let badge = app.session_badge_at(0, 0, 0).unwrap();
        assert_eq!(badge.kind, BadgeKind::Working);
        assert!(!badge.label_compact().contains("compacted"));
    }

    #[test]
    fn a_failed_turn_needs_you_and_stops_the_working_marker() {
        let (mut app, id) = app();
        app.handle_ipc_message_value(json!({
            "type": "thinking-start", "session_id": "claude-uuid", "aiw_feature_session_id": id
        }));
        assert_eq!(
            app.session_badge_at(0, 0, 0).unwrap().kind,
            BadgeKind::Working
        );
        app.handle_ipc_message_value(event(
            &id,
            "stop-failure",
            json!({"error": "rate_limit_error"}),
        ));
        let badge = app.session_badge_at(0, 0, 0).unwrap();
        assert_eq!(badge.kind, BadgeKind::NeedsYou);
        assert_eq!(badge.label_compact(), "turn failed: rate_limit_error");
        assert!(!app.ipc_thinking_feature_sessions.contains(&id));
        app.handle_ipc_message_value(json!({
            "type": "thinking-start", "session_id": "claude-uuid", "aiw_feature_session_id": id
        }));
        assert_eq!(
            app.session_badge_at(0, 0, 0).unwrap().kind,
            BadgeKind::Working
        );
    }

    #[test]
    fn a_failure_outranks_the_previous_turns_done() {
        let (mut app, id) = app();
        app.handle_ipc_message_value(json!({
            "type": "attention", "aiw_event_kind": "completed", "aiw_session": "aiw-hx-12",
            "cwd": "/tmp"
        }));
        assert_eq!(
            app.session_badge_at(0, 0, 0).unwrap().kind,
            BadgeKind::FinishedUnread
        );
        app.handle_ipc_message_value(event(&id, "stop-failure", json!({})));
        assert_eq!(
            app.session_badge_at(0, 0, 0).unwrap().label_compact(),
            "turn failed: API error"
        );
    }

    #[test]
    fn background_work_keeps_a_feature_off_the_dormant_list() {
        let (mut app, id) = app();
        // The feature's window last saw output three hours ago.
        let mut tmux = crate::traits::MockTmuxOps::new();
        let stale = (Utc::now() - chrono::Duration::hours(3)).timestamp();
        tmux.expect_window_activity()
            .returning(move || vec![("aiw-hx-12".into(), "claude".into(), stale)]);
        app.tmux = Box::new(tmux);
        app.config.dormant_idle_minutes = 1;
        app.config.dormant_last_accessed_hours = 1;
        app.store.projects[0].features[0].last_accessed = Utc::now() - chrono::Duration::hours(3);
        let before = app.find_dormant_features().len();
        assert_eq!(before, 1, "dormant without background work");
        app.handle_ipc_message_value(event(&id, "stop", json!({"session_crons_count": 1})));
        assert_eq!(app.find_dormant_features().len(), 0);
        app.handle_ipc_message_value(event(&id, "stop", json!({})));
        assert_eq!(app.find_dormant_features().len(), before);
    }

    #[test]
    fn the_working_row_counts_tasks_and_background_work() {
        let (mut app, id) = app();
        app.handle_ipc_message_value(json!({
            "type": "thinking-start", "session_id": "claude-uuid", "aiw_feature_session_id": id
        }));
        for _ in 0..3 {
            app.handle_ipc_message_value(event(&id, "task-created", json!({"task_subject": "t"})));
        }
        app.handle_ipc_message_value(event(&id, "task-completed", json!({})));
        app.handle_ipc_message_value(event(
            &id,
            "stop",
            json!({"background_tasks_count": 1, "session_crons_count": 1}),
        ));
        let label = app.session_badge_at(0, 0, 0).unwrap().label_compact();
        assert_eq!(label, "working · tasks 1/3 · 1 bg · 1 loop");
        assert!(app.feature_has_background_work(0, 0));
    }

    #[test]
    fn session_end_forgets_the_state_and_unknown_messages_pass_through() {
        let (mut app, id) = app();
        app.handle_ipc_message_value(event(&id, "post-compact", json!({})));
        assert!(app.hook_state(&id).is_some());
        app.handle_ipc_message_value(event(&id, "session-end", json!({"reason": "other"})));
        assert!(app.hook_state(&id).is_none());
        assert!(!app.handle_hook_event_message(&json!({"type": "subagent"})));
        assert!(app.handle_hook_event_message(&json!({"type": "hook-event", "kind": "bogus"})));
    }

    #[test]
    fn a_turn_end_clears_a_tool_whose_post_tool_hook_never_came() {
        let app = crate::app::fork_workspace::workspace_app(&[SessionKind::Codex]);
        let mut app = app;
        let id = app.store.projects[0].features[0].sessions[0].id.clone();
        app.handle_ipc_message_value(json!({
            "type": "tool-start", "session_id": "codex-thread", "aiw_feature_session_id": id,
            "cwd": "/tmp"
        }));
        assert_eq!(
            app.session_badge_at(0, 0, 0).unwrap().kind,
            BadgeKind::Working
        );
        app.handle_ipc_message_value(json!({
            "type": "thinking-stop", "session_id": "codex-thread", "aiw_feature_session_id": id
        }));
        assert_ne!(
            app.session_badge_at(0, 0, 0).unwrap().kind,
            BadgeKind::Working
        );
    }

    #[test]
    fn a_hook_event_is_a_new_frame() {
        let (mut app, id) = app();
        let before = app.redraw_signature();
        app.handle_ipc_message_value(event(&id, "post-compact", json!({})));
        assert_ne!(before, app.redraw_signature());
    }

    impl App {
        /// Dispatch through the real IPC path; `true` when a hook event
        /// state now exists for the message's session.
        fn handle_ipc_message_value_returns(&mut self, raw: Value) -> bool {
            let id = raw["aiw_feature_session_id"].as_str().unwrap().to_string();
            self.handle_ipc_message_value(raw);
            self.hook_events.contains_key(&id)
        }
    }
}
