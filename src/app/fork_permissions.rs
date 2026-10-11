//! FORK: pending permission requests on the dashboard (P6, DESIGN.md §16).
//!
//! A `permission_request` arrives over IPC with its connection parked
//! ([`crate::fork::ipc_reply`]); the request is kept here, the session gets
//! its `⛔` through the attention layer (reason `permission: <tool> — <input>`),
//! and `a` / `X` on the row, in the needs-you picker or in the detail overlay
//! answer it. Every decision is written to the debug log.

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::attention::{AttentionRecord, AttentionState, HarnessCapabilities};
use super::{App, AppMode, Selection};
use crate::fork::permissions::{Behavior, Harness, REQUEST_TYPE, detail, reply_json};
use crate::project::{AgentKind, SessionKind};

/// The attention record's `notification_type` for a hook-delivered request,
/// so the layer can tell it from Claude's older `permission_prompt`.
pub const NOTIFICATION_TYPE: &str = "permission_request";

/// One request waiting for the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPermission {
    pub request_id: String,
    pub harness: Harness,
    /// The feature's tmux session (the attention layer's key).
    pub tmux_session: String,
    /// `AIW_FEATURE_SESSION_ID` of the window the hook ran in, when known.
    pub feature_session_id: Option<String>,
    pub provider_session_id: Option<String>,
    pub tool_name: String,
    pub tool_input: Value,
    pub since: DateTime<Utc>,
}

impl PendingPermission {
    /// `Bash — cargo test`: the badge model prefixes `permission: `.
    pub fn detail(&self) -> String {
        detail(&self.tool_name, &self.tool_input)
    }

    /// The agent kind the request belongs to.
    pub fn agent(&self) -> AgentKind {
        match self.harness {
            Harness::Claude => AgentKind::Claude,
            Harness::Codex => AgentKind::Codex,
            Harness::Cursor => AgentKind::Cursor,
        }
    }

    fn session_kind(&self) -> SessionKind {
        match self.harness {
            Harness::Claude => SessionKind::Claude,
            Harness::Codex => SessionKind::Codex,
            Harness::Cursor => SessionKind::Cursor,
        }
    }
}

/// The detail overlay (`Enter` in the needs-you picker): the request and
/// where to go back to.
pub struct PermissionDetailState {
    pub request_id: String,
    pub previous_mode: Box<AppMode>,
}

impl App {
    /// A `permission_request` message. Returns `false` when `raw` is not one.
    pub(crate) fn handle_permission_request_message(&mut self, raw: &Value) -> bool {
        if raw.get("type").and_then(Value::as_str) != Some(REQUEST_TYPE) {
            return false;
        }
        let text = |key: &str| {
            raw.get(key)
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|v| !v.is_empty())
        };
        let Some(request_id) = text("request_id") else {
            self.log_warn(
                "permission",
                "permission_request without a request_id".into(),
            );
            return true;
        };
        let harness = text("harness")
            .as_deref()
            .and_then(Harness::parse)
            .unwrap_or(Harness::Claude);
        let aiw_session = text("aiw_tmux_session").or_else(|| text("aiw_session"));
        let cwd = text("cwd").unwrap_or_default();
        let feature = self.fork_feature_for(aiw_session.as_deref(), &cwd);
        let Some((pi, fi)) = feature else {
            self.log_warn(
                "permission",
                format!(
                    "permission_request {request_id} matched no feature (session={aiw_session:?}, cwd={cwd}); letting the harness prompt"
                ),
            );
            crate::fork::ipc_reply::reply(&request_id, &reply_json(Behavior::Pass, None));
            return true;
        };
        let feature = &self.store.projects[pi].features[fi];
        let tmux_session = feature.tmux_session.clone();
        let feature_name = feature.name.clone();
        let feature_session_id = text("aiw_feature_session_id");
        let pending = PendingPermission {
            request_id: request_id.clone(),
            harness,
            tmux_session: tmux_session.clone(),
            feature_session_id: feature_session_id.clone(),
            provider_session_id: text("session_id"),
            tool_name: text("tool_name").unwrap_or_else(|| "tool".into()),
            tool_input: raw.get("tool_input").cloned().unwrap_or(Value::Null),
            since: Utc::now(),
        };
        // One live request per window: an older one still parked is told to
        // let its harness prompt (it would otherwise wait out its timeout).
        let superseded: Vec<String> = self
            .pending_permissions
            .iter()
            .filter(|p| {
                p.tmux_session == tmux_session
                    && p.harness == harness
                    && (p.feature_session_id.is_none()
                        || p.feature_session_id == feature_session_id)
            })
            .map(|p| p.request_id.clone())
            .collect();
        for id in superseded {
            self.pending_permissions.retain(|p| p.request_id != id);
            crate::fork::ipc_reply::reply(&id, &reply_json(Behavior::Pass, None));
            self.log_debug(
                "permission",
                format!("request {id} superseded by {request_id}; let the harness prompt"),
            );
        }
        let reason = pending.detail();
        let agent = pending.agent();
        self.pending_permissions.push(pending);
        // Recorded directly: the capability table says Codex and Cursor cannot
        // ask a question through their older signals, but this hook can.
        let mut record = AttentionRecord::new(agent.clone(), AttentionState::Question, Utc::now());
        record.state = AttentionState::Question;
        record.capabilities = HarnessCapabilities {
            reports_question: true,
            ..record.capabilities
        };
        record.notification_type = Some(NOTIFICATION_TYPE.to_string());
        record.detail = Some(reason.clone());
        self.attention.insert(tmux_session.clone(), record);
        self.log_info(
            "permission",
            format!(
                "{feature_name} ({}) asks permission: {reason} [request {request_id}]",
                harness.as_str()
            ),
        );
        self.push_toast_warning(format!(
            "{feature_name} asks permission: {reason}  (a allow · X deny)"
        ));
        true
    }

    /// The hook gave up waiting (its timeout, or it was killed): the harness
    /// prompts itself, so the row's `⛔` is no longer ours to answer.
    pub(crate) fn handle_reply_abandoned(&mut self, request_id: &str) {
        let Some(index) = self
            .pending_permissions
            .iter()
            .position(|p| p.request_id == request_id)
        else {
            return;
        };
        let pending = self.pending_permissions.remove(index);
        self.clear_permission_attention(&pending.tmux_session);
        self.log_info(
            "permission",
            format!(
                "request {request_id} ({}) was not answered in time; the harness shows its own prompt",
                pending.detail()
            ),
        );
        if matches!(&self.mode, AppMode::PermissionDetail(state) if state.request_id == request_id)
        {
            self.close_permission_detail();
        }
    }

    fn clear_permission_attention(&mut self, tmux_session: &str) {
        if self
            .attention
            .get(tmux_session)
            .is_some_and(|r| r.notification_type.as_deref() == Some(NOTIFICATION_TYPE))
        {
            self.attention.remove(tmux_session);
        }
    }

    /// Answer `request_id`. Returns whether a request by that id was pending.
    pub fn answer_permission(&mut self, request_id: &str, behavior: Behavior) -> bool {
        let Some(index) = self
            .pending_permissions
            .iter()
            .position(|p| p.request_id == request_id)
        else {
            return false;
        };
        let pending = self.pending_permissions.remove(index);
        let delivered = crate::fork::ipc_reply::reply(request_id, &reply_json(behavior, None));
        // FORK(cockpit): the info block shows the last decision (DESIGN.md §18).
        self.record_permission_decision(
            pending.feature_session_id.as_deref(),
            &pending.tool_name,
            behavior,
        );
        self.clear_permission_attention(&pending.tmux_session);
        let reason = pending.detail();
        let feature = self
            .store
            .projects
            .iter()
            .flat_map(|p| p.features.iter())
            .find(|f| f.tmux_session == pending.tmux_session)
            .map(|f| f.name.clone())
            .unwrap_or_else(|| pending.tmux_session.clone());
        if delivered {
            self.log_info(
                "permission",
                format!(
                    "{feature}: {} — {reason} [request {request_id}]",
                    match behavior {
                        Behavior::Allow => "allowed",
                        Behavior::Deny => "denied",
                        Behavior::Pass => "left to the harness",
                    }
                ),
            );
            match behavior {
                Behavior::Allow => self.push_toast_success(format!("{feature}: allowed {reason}")),
                Behavior::Deny => self.push_toast_warning(format!("{feature}: denied {reason}")),
                Behavior::Pass => {
                    self.push_toast_info(format!("{feature}: the agent will ask in its own pane"))
                }
            }
        } else {
            self.log_warn(
                "permission",
                format!(
                    "{feature}: {} {reason}, but the hook had already stopped waiting [request {request_id}]",
                    behavior.as_str()
                ),
            );
            self.push_toast_warning(format!(
                "{feature}: the agent stopped waiting; answer it in its pane"
            ));
        }
        if matches!(&self.mode, AppMode::PermissionDetail(state) if state.request_id == request_id)
        {
            self.close_permission_detail();
        }
        true
    }

    /// The request waiting on the session at `(pi, fi, si)`, if any.
    pub fn pending_permission_at(
        &self,
        pi: usize,
        fi: usize,
        si: usize,
    ) -> Option<&PendingPermission> {
        let feature = self.store.projects.get(pi)?.features.get(fi)?;
        let session = feature.sessions.get(si)?;
        self.pending_permissions
            .iter()
            .find(|p| p.feature_session_id.as_deref() == Some(session.id.as_str()))
            .or_else(|| {
                self.pending_permissions.iter().find(|p| {
                    p.tmux_session == feature.tmux_session
                        && p.feature_session_id.is_none()
                        && p.session_kind() == session.kind
                })
            })
    }

    /// The request waiting on the feature `tmux_session` (any of its windows).
    pub fn pending_permission_for_tmux_session(
        &self,
        tmux_session: &str,
    ) -> Option<&PendingPermission> {
        self.pending_permissions
            .iter()
            .find(|p| p.tmux_session == tmux_session)
    }

    /// The request waiting on the selected row (a session row, or a feature
    /// row's first waiting window).
    pub fn pending_permission_for_selection(&self) -> Option<&PendingPermission> {
        match self.selection {
            Selection::Session(pi, fi, si) => self.pending_permission_at(pi, fi, si),
            Selection::Feature(pi, fi) => {
                let feature = self.store.projects.get(pi)?.features.get(fi)?;
                self.pending_permission_for_tmux_session(&feature.tmux_session)
            }
            Selection::Project(_) => None,
        }
    }

    pub fn pending_permission_by_id(&self, request_id: &str) -> Option<&PendingPermission> {
        self.pending_permissions
            .iter()
            .find(|p| p.request_id == request_id)
    }

    /// Where a pending request's session sits in the store.
    pub fn pending_permission_location(
        &self,
        pending: &PendingPermission,
    ) -> Option<(usize, usize, Option<usize>)> {
        for (pi, project) in self.store.projects.iter().enumerate() {
            for (fi, feature) in project.features.iter().enumerate() {
                if feature.tmux_session != pending.tmux_session {
                    continue;
                }
                let si = feature
                    .sessions
                    .iter()
                    .position(|s| pending.feature_session_id.as_deref() == Some(s.id.as_str()))
                    .or_else(|| {
                        feature
                            .sessions
                            .iter()
                            .position(|s| s.kind == pending.session_kind())
                    });
                return Some((pi, fi, si));
            }
        }
        None
    }

    /// Open the detail overlay for `request_id` over the current mode.
    pub fn open_permission_detail(&mut self, request_id: &str) {
        if self.pending_permission_by_id(request_id).is_none() {
            return;
        }
        let previous = std::mem::replace(&mut self.mode, AppMode::Normal);
        self.mode = AppMode::PermissionDetail(Box::new(PermissionDetailState {
            request_id: request_id.to_string(),
            previous_mode: Box::new(previous),
        }));
    }

    /// Close the overlay, back to where it was opened from.
    pub fn close_permission_detail(&mut self) {
        if let AppMode::PermissionDetail(state) = std::mem::replace(&mut self.mode, AppMode::Normal)
        {
            self.mode = *state.previous_mode;
        }
    }

    /// Close the overlay and open the request's session.
    pub fn open_permission_session(&mut self, request_id: &str) -> anyhow::Result<()> {
        let Some(location) = self
            .pending_permission_by_id(request_id)
            .and_then(|p| self.pending_permission_location(p))
        else {
            self.close_permission_detail();
            return Ok(());
        };
        self.mode = AppMode::Normal;
        self.selection = match location {
            (pi, fi, Some(si)) => Selection::Session(pi, fi, si),
            (pi, fi, None) => Selection::Feature(pi, fi),
        };
        // The agent pane follows the selection; focus it.
        self.cockpit_focus_agent();
        Ok(())
    }

    /// The feature a hook message belongs to: by its tmux session, else by
    /// the deepest feature workdir containing `cwd`.
    pub(crate) fn fork_feature_for(
        &self,
        aiw_session: Option<&str>,
        cwd: &str,
    ) -> Option<(usize, usize)> {
        if let Some(name) = aiw_session.filter(|s| !s.is_empty()) {
            for (pi, project) in self.store.projects.iter().enumerate() {
                if let Some(fi) = project.features.iter().position(|f| f.tmux_session == name) {
                    return Some((pi, fi));
                }
            }
        }
        if cwd.is_empty() {
            return None;
        }
        let cwd = std::path::Path::new(cwd);
        let mut best: Option<((usize, usize), usize)> = None;
        for (pi, project) in self.store.projects.iter().enumerate() {
            for (fi, feature) in project.features.iter().enumerate() {
                let len = feature.workdir.as_os_str().len();
                if cwd.starts_with(&feature.workdir) && best.is_none_or(|(_, l)| len > l) {
                    best = Some(((pi, fi), len));
                }
            }
        }
        best.map(|(at, _)| at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::badges::BadgeKind;
    use serde_json::json;

    fn app() -> App {
        crate::app::fork_workspace::workspace_app(&[SessionKind::Claude, SessionKind::Codex])
    }

    fn request(app: &mut App, id: &str, harness: &str, session_id: Option<&str>) {
        let mut raw = json!({
            "type": "permission_request",
            "request_id": id,
            "harness": harness,
            "session_id": "prov-1",
            "aiw_session": "aiw-hx-12",
            "aiw_tmux_session": "aiw-hx-12",
            "cwd": "/tmp",
            "tool_name": "Bash",
            "tool_input": {"command": "cargo test"}
        });
        if let Some(sid) = session_id {
            raw["aiw_feature_session_id"] = json!(sid);
        }
        assert!(app.handle_permission_request_message(&raw));
    }

    #[test]
    fn a_request_marks_the_session_as_needing_you_with_the_tool_and_input() {
        let mut app = app();
        let claude_id = app.store.projects[0].features[0].sessions[0].id.clone();
        request(&mut app, "r1", "claude", Some(&claude_id));
        let badge = app.session_badge_at(0, 0, 0).unwrap();
        assert_eq!(badge.kind, BadgeKind::NeedsYou);
        assert_eq!(badge.label(Utc::now()), "permission: Bash — cargo test");
        assert_eq!(
            app.session_badge_at(0, 0, 1).unwrap().kind,
            BadgeKind::Idle,
            "the Codex window is not asking"
        );
        app.selection = Selection::Session(0, 0, 0);
        assert_eq!(
            app.pending_permission_for_selection()
                .map(|p| p.request_id.as_str()),
            Some("r1")
        );
        app.selection = Selection::Session(0, 0, 1);
        assert!(app.pending_permission_for_selection().is_none());
        app.selection = Selection::Feature(0, 0);
        assert!(app.pending_permission_for_selection().is_some());
    }

    #[test]
    fn a_codex_request_is_a_question_despite_the_capability_table() {
        let mut app = app();
        let codex_id = app.store.projects[0].features[0].sessions[1].id.clone();
        request(&mut app, "r2", "codex", Some(&codex_id));
        let badge = app.session_badge_at(0, 0, 1).unwrap();
        assert_eq!(badge.kind, BadgeKind::NeedsYou);
        assert!(badge.label(Utc::now()).starts_with("permission: "));
        assert_eq!(app.session_badge_at(0, 0, 0).unwrap().kind, BadgeKind::Idle);
    }

    #[test]
    fn answering_clears_the_badge_and_the_request() {
        let mut app = app();
        request(&mut app, "r3", "claude", None);
        assert_eq!(
            app.session_badge_at(0, 0, 0).unwrap().kind,
            BadgeKind::NeedsYou
        );
        assert!(app.answer_permission("r3", Behavior::Allow));
        assert!(app.pending_permissions.is_empty());
        assert_eq!(app.session_badge_at(0, 0, 0).unwrap().kind, BadgeKind::Idle);
        assert!(
            !app.answer_permission("r3", Behavior::Deny),
            "already answered"
        );
    }

    #[test]
    fn an_abandoned_request_is_dropped_and_a_newer_one_supersedes() {
        let mut app = app();
        request(&mut app, "r4", "claude", None);
        request(&mut app, "r5", "claude", None);
        assert_eq!(app.pending_permissions.len(), 1);
        assert_eq!(app.pending_permissions[0].request_id, "r5");
        app.handle_reply_abandoned("r5");
        assert!(app.pending_permissions.is_empty());
        assert_eq!(app.session_badge_at(0, 0, 0).unwrap().kind, BadgeKind::Idle);
    }

    #[test]
    fn a_request_for_an_unknown_feature_is_ignored() {
        let mut app = app();
        let raw = json!({
            "type": "permission_request", "request_id": "r6", "harness": "claude",
            "aiw_session": "aiw-nope", "cwd": "/nowhere", "tool_name": "Bash", "tool_input": {}
        });
        assert!(app.handle_permission_request_message(&raw));
        assert!(app.pending_permissions.is_empty());
        assert!(!app.handle_permission_request_message(&json!({"type": "stop"})));
    }

    #[test]
    fn the_detail_overlay_opens_over_the_picker_and_closes_back() {
        let mut app = app();
        request(&mut app, "r7", "claude", None);
        app.mode = AppMode::NotificationPicker(0, None);
        app.open_permission_detail("r7");
        assert!(matches!(app.mode, AppMode::PermissionDetail(_)));
        app.close_permission_detail();
        assert!(matches!(app.mode, AppMode::NotificationPicker(0, None)));
        app.open_permission_detail("missing");
        assert!(matches!(app.mode, AppMode::NotificationPicker(0, None)));
    }
}
