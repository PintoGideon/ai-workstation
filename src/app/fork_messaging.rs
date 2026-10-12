//! FORK: messages to agent sessions — `@` on the dashboard and
//! `aiw automation message` (DESIGN.md §19.5).
//!
//! A Claude session's `SessionStart` hook reports its inbox socket; a message
//! to it is written straight into that inbox (Claude queues it as the next
//! prompt, attributed to the sender, without anything being typed into the
//! pane). Everything else — Codex, a Claude without a reported inbox, an
//! inbox that turns out to be gone — goes through the follow-up queue and is
//! delivered when the agent next finishes a turn. The route taken is always
//! reported.

use std::collections::HashMap;

use serde_json::Value;

use super::App;
use crate::app::{AppMode, Selection};
use crate::fork::messaging::{
    self, Delivery, InboxAddress, MESSAGE_ACTION, PostError, Route, TargetCandidate,
};
use crate::project::SessionKind;

/// Inboxes reported by Claude sessions, keyed by feature session id.
#[derive(Debug, Default)]
pub struct MessagingRuntime {
    pub(crate) inboxes: HashMap<String, InboxAddress>,
}

/// One agent session the dialog can address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageTarget {
    pub session_id: String,
    /// `auth-fix · Claude 1`.
    pub title: String,
    /// What the dialog shows as the route (`inbox` / `queue: …`).
    pub route: String,
}

/// The `@` dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageDialogState {
    pub targets: Vec<MessageTarget>,
    pub target_index: usize,
    pub text: String,
}

impl MessageDialogState {
    pub fn target(&self) -> Option<&MessageTarget> {
        self.targets.get(self.target_index)
    }

    /// Move the target selection by `delta`, wrapping.
    pub fn move_target(&mut self, delta: i32) {
        let n = self.targets.len() as i32;
        if n == 0 {
            return;
        }
        self.target_index = (self.target_index as i32 + delta).rem_euclid(n) as usize;
    }
}

impl App {
    // ── inboxes ──────────────────────────────────────────────────

    /// Load stored inboxes (start-up). Tokens are not stored, so reloaded
    /// addresses post without the optional auth line.
    pub(crate) fn load_inbox_addresses(&mut self) {
        let Some(db) = self.db.as_ref() else {
            return;
        };
        match db.all_inbox_addresses() {
            Ok(all) => self.messaging.inboxes = all,
            Err(e) => self.log_warn("messaging", format!("could not load inboxes: {e}")),
        }
    }

    /// A `session-start` hook event: remember the session's inbox, if it
    /// reported one.
    pub(crate) fn messaging_session_started(&mut self, session_id: &str, raw: &Value) {
        let Some(address) = InboxAddress::from_hook_event(raw) else {
            return;
        };
        if self.messaging.inboxes.get(session_id) == Some(&address) {
            return;
        }
        // Never log the token.
        self.log_debug(
            "messaging",
            format!(
                "{session_id}: inbox {} (token {})",
                address.socket.display(),
                if address.token.is_some() {
                    "known"
                } else {
                    "absent"
                }
            ),
        );
        if let Some(db) = self.db.as_ref()
            && let Err(e) = db.save_inbox_address(session_id, &address)
        {
            self.log_warn("messaging", format!("could not save an inbox: {e}"));
        }
        self.messaging
            .inboxes
            .insert(session_id.to_string(), address);
    }

    /// Forget a session's inbox (it ended, was deleted, or proved stale).
    pub(crate) fn forget_inbox(&mut self, session_id: &str) {
        if self.messaging.inboxes.remove(session_id).is_some()
            && let Some(db) = self.db.as_ref()
            && let Err(e) = db.delete_inbox_address(session_id)
        {
            self.log_warn("messaging", format!("could not delete an inbox: {e}"));
        }
    }

    /// The inbox reported by `session_id`, if any.
    pub fn inbox_for(&self, session_id: &str) -> Option<&InboxAddress> {
        self.messaging.inboxes.get(session_id)
    }

    // ── routing and delivery ─────────────────────────────────────

    fn session_kind(&self, session_id: &str) -> Option<SessionKind> {
        self.store
            .projects
            .iter()
            .flat_map(|p| p.features.iter())
            .flat_map(|f| f.sessions.iter())
            .find(|s| s.id == session_id)
            .map(|s| s.kind.clone())
    }

    /// How a message to `session_id` would travel right now.
    pub fn message_route(&self, session_id: &str) -> Route {
        messaging::route(
            self.session_kind(session_id) == Some(SessionKind::Claude),
            self.inbox_for(session_id).is_some(),
            self.config.aiw.messaging_inbox,
        )
    }

    /// Every agent session, in dashboard order, as an addressable target.
    pub fn message_targets(&self) -> Vec<MessageTarget> {
        let mut out = Vec::new();
        for project in &self.store.projects {
            for feature in &project.features {
                let name = feature
                    .nickname
                    .clone()
                    .unwrap_or_else(|| feature.name.clone());
                for session in feature
                    .sessions
                    .iter()
                    .filter(|s| s.kind.is_agent_harness())
                {
                    let route = match self.message_route(&session.id) {
                        Route::Inbox => "inbox".to_string(),
                        Route::Queue(_) => "queue (next turn end)".to_string(),
                    };
                    out.push(MessageTarget {
                        session_id: session.id.clone(),
                        title: format!("{name} · {}", session.label),
                        route,
                    });
                }
            }
        }
        out
    }

    /// Resolve `query` (an id, a harness name, `<feature>-<label>`,
    /// `<feature>/<label>`, or a unique label) to an agent session id.
    pub fn resolve_message_target(&self, query: &str) -> Result<String, String> {
        let candidates: Vec<TargetCandidate<'_>> = self
            .store
            .projects
            .iter()
            .flat_map(|p| p.features.iter())
            .flat_map(|f| {
                f.sessions
                    .iter()
                    .filter(|s| s.kind.is_agent_harness())
                    .map(move |s| (f, s))
            })
            .map(|(f, s)| TargetCandidate {
                id: &s.id,
                harness_name: self.session_harness_name(&s.id),
                feature: &f.name,
                label: &s.label,
            })
            .collect();
        messaging::resolve_target(&candidates, query)
    }

    /// The display name of `session_id` for results (`auth-fix-claude-1` or
    /// `auth-fix/Codex 1`).
    pub(crate) fn message_target_name(&self, session_id: &str) -> String {
        if let Some(name) = self.session_harness_name(session_id) {
            return name.to_string();
        }
        self.store
            .projects
            .iter()
            .flat_map(|p| p.features.iter())
            .find_map(|f| {
                f.sessions
                    .iter()
                    .find(|s| s.id == session_id)
                    .map(|s| format!("{}/{}", f.name, s.label))
            })
            .unwrap_or_else(|| session_id.to_string())
    }

    fn queue_message(
        &mut self,
        session_id: &str,
        text: &str,
        from: &str,
        reason: String,
    ) -> Delivery {
        match self.queue_followup(session_id, &messaging::queued_text(text, from)) {
            Ok(position) => Delivery::Queued { position, reason },
            Err(e) => Delivery::Failed(e.to_string()),
        }
    }

    /// Deliver `text` to agent session `session_id` from `from`: into its
    /// inbox when it has one, through the follow-up queue otherwise. A stale
    /// inbox is forgotten and the message falls back to the queue.
    pub fn send_session_message(&mut self, session_id: &str, text: &str, from: &str) -> Delivery {
        let text = match messaging::validate_text(text) {
            Ok(text) => text.to_string(),
            Err(e) => return Delivery::Failed(e),
        };
        match self.session_kind(session_id) {
            Some(kind) if kind.is_agent_harness() => {}
            Some(_) => return Delivery::Failed("that session is not an agent".into()),
            None => return Delivery::Failed(format!("no session with id '{session_id}'")),
        }
        let delivery = match self.message_route(session_id) {
            Route::Queue(reason) => self.queue_message(session_id, &text, from, reason.to_string()),
            Route::Inbox => {
                let address = self.messaging.inboxes[session_id].clone();
                match messaging::post(&address, &text, from) {
                    Ok(()) => Delivery::Inbox,
                    Err(err) => {
                        if matches!(
                            err,
                            PostError::Missing | PostError::Refused | PostError::NotOurSocket
                        ) {
                            self.forget_inbox(session_id);
                        }
                        self.queue_message(
                            session_id,
                            &text,
                            from,
                            format!("inbox unreachable ({err})"),
                        )
                    }
                }
            }
        };
        let target = self.message_target_name(session_id);
        self.log_info(
            "messaging",
            format!(
                "{from} → {target}: {} ({} chars)",
                delivery.status(),
                text.chars().count()
            ),
        );
        delivery
    }

    // ── `@` dialog ───────────────────────────────────────────────

    /// Open the message dialog, addressed to the selected agent session (or
    /// the first agent of the selected feature).
    pub fn open_message_dialog(&mut self) {
        let targets = self.message_targets();
        if targets.is_empty() {
            self.push_toast_warning("No agent sessions to message");
            return;
        }
        let selected = match self.selection {
            Selection::Session(pi, fi, si) => self
                .store
                .projects
                .get(pi)
                .and_then(|p| p.features.get(fi))
                .and_then(|f| f.sessions.get(si))
                .filter(|s| s.kind.is_agent_harness())
                .map(|s| s.id.clone()),
            Selection::Feature(pi, fi) => self
                .store
                .projects
                .get(pi)
                .and_then(|p| p.features.get(fi))
                .and_then(|f| f.sessions.iter().find(|s| s.kind.is_agent_harness()))
                .map(|s| s.id.clone()),
            _ => None,
        };
        let target_index = selected
            .and_then(|id| targets.iter().position(|t| t.session_id == id))
            .unwrap_or(0);
        self.mode = AppMode::MessageSession(MessageDialogState {
            targets,
            target_index,
            text: String::new(),
        });
    }

    /// Send the dialog's message. The dialog stays open (with its text) when
    /// the send failed, so nothing typed is lost.
    pub fn submit_message_dialog(&mut self) {
        let AppMode::MessageSession(state) = &self.mode else {
            return;
        };
        let Some(target) = state.target().cloned() else {
            return;
        };
        let text = state.text.clone();
        if let Err(e) = messaging::validate_text(&text) {
            self.push_toast_warning(e);
            return;
        }
        let delivery = self.send_session_message(&target.session_id, &text, messaging::AIW_SENDER);
        let line = delivery.summary(&target.title);
        match delivery {
            Delivery::Inbox => {
                self.mode = AppMode::Normal;
                self.push_toast_success(line);
            }
            Delivery::Queued { .. } => {
                self.mode = AppMode::Normal;
                self.push_toast_info(line);
            }
            Delivery::Failed(_) => self.push_toast_error(line),
        }
    }

    // ── `aiw automation message` ─────────────────────────────────

    /// `aiw automation message --to <session> --text <text> [--from <name>]`.
    pub(crate) fn automation_message(&mut self, raw: &Value) -> Value {
        let text_field = |key: &str| {
            raw.get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        let to = text_field("to");
        let text = text_field("text");
        let from = messaging::sender(raw.get("from").and_then(Value::as_str));
        let session_id = match self.resolve_message_target(&to) {
            Ok(id) => id,
            Err(e) => {
                return serde_json::json!({
                    "type": crate::automation::AUTOMATION_RESULT_TYPE,
                    "action": MESSAGE_ACTION,
                    "ok": false,
                    "status": "failed",
                    "error": e,
                });
            }
        };
        let delivery = self.send_session_message(&session_id, &text, &from);
        let target = self.message_target_name(&session_id);
        let mut out = serde_json::json!({
            "type": crate::automation::AUTOMATION_RESULT_TYPE,
            "action": MESSAGE_ACTION,
            "ok": !matches!(delivery, Delivery::Failed(_)),
            "status": delivery.status(),
            "session_id": session_id,
            "target": target,
            "from": from,
            "detail": delivery.summary(&target),
        });
        match &delivery {
            Delivery::Inbox => out["via"] = Value::String("inbox".into()),
            Delivery::Queued { position, reason } => {
                out["via"] = Value::String("followup-queue".into());
                out["queued"] = Value::from(*position);
                out["reason"] = Value::String(reason.clone());
            }
            Delivery::Failed(reason) => out["error"] = Value::String(reason.clone()),
        }
        out
    }
}
