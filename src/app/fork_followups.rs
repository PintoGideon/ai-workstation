//! FORK: queued follow-ups on the dashboard (P7, DESIGN.md §16).
//!
//! `App::followups` mirrors the `aiw_followups` table (loaded at start,
//! every change written through), so the badge count and the dialog never
//! read the database while drawing. A Stop hook's `followup_take` pops the
//! session's first item and answers on its parked connection; nothing is
//! ever typed into an agent on its own.

use std::collections::HashMap;

use anyhow::Result;
use serde_json::Value;

use super::{App, AppMode, Selection};
use crate::editor::TextEditor;
use crate::fork::followups::{Followup, QUEUE_ACTION, TAKE_TYPE, take_reply};
use crate::project::SessionKind;

/// Which half of the dialog has the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FollowupFocus {
    Editor,
    List,
}

/// The "Queue a follow-up" dialog.
pub struct FollowupQueueState {
    pub session_id: String,
    /// `project › feature › session`.
    pub title: String,
    pub editor: TextEditor,
    pub scroll_offset: usize,
    pub sync_scroll_to_cursor: bool,
    pub selected: usize,
    pub focus: FollowupFocus,
    pub previous_mode: Box<AppMode>,
}

impl App {
    /// The follow-ups queued for `session_id`, first delivered first.
    pub fn followups_for(&self, session_id: &str) -> &[Followup] {
        self.followups
            .get(session_id)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn queued_followup_count(&self, session_id: &str) -> usize {
        self.followups_for(session_id).len()
    }

    /// Queue `text` for `session_id`. Returns the queue length.
    pub fn queue_followup(&mut self, session_id: &str, text: &str) -> Result<usize> {
        let text = text.trim();
        if text.is_empty() {
            anyhow::bail!("the follow-up is empty");
        }
        let followup = match self.db.as_ref() {
            Some(db) => db.push_followup(session_id, text)?,
            None => {
                self.followup_memory_id -= 1;
                Followup {
                    id: self.followup_memory_id,
                    session_id: session_id.to_string(),
                    position: self.queued_followup_count(session_id) as i64,
                    text: text.to_string(),
                    created_at: chrono::Utc::now().to_rfc3339(),
                }
            }
        };
        let queue = self.followups.entry(session_id.to_string()).or_default();
        queue.push(followup);
        let count = queue.len();
        self.log_info(
            "followup",
            format!(
                "queued #{count} for session {session_id}: {}",
                one_line(text)
            ),
        );
        Ok(count)
    }

    /// Drop the queued item at `index`.
    pub fn delete_followup(&mut self, session_id: &str, index: usize) -> Result<()> {
        let Some(queue) = self.followups.get_mut(session_id) else {
            return Ok(());
        };
        if index >= queue.len() {
            return Ok(());
        }
        let removed = queue.remove(index);
        if queue.is_empty() {
            self.followups.remove(session_id);
        }
        if let Some(db) = self.db.as_ref() {
            db.delete_followup(removed.id)?;
        }
        self.log_info(
            "followup",
            format!(
                "dropped a queued follow-up for session {session_id}: {}",
                one_line(&removed.text)
            ),
        );
        Ok(())
    }

    /// Move the item at `index` one place up (`-1`) or down (`+1`).
    pub fn move_followup(&mut self, session_id: &str, index: usize, delta: i32) -> Result<usize> {
        let Some(queue) = self.followups.get_mut(session_id) else {
            return Ok(index);
        };
        let target = index as i64 + delta as i64;
        if index >= queue.len() || target < 0 || target as usize >= queue.len() {
            return Ok(index);
        }
        let target = target as usize;
        queue.swap(index, target);
        let ids: Vec<i64> = queue.iter().map(|f| f.id).collect();
        if let Some(db) = self.db.as_ref() {
            db.reorder_followups(session_id, &ids)?;
        }
        Ok(target)
    }

    /// Pop the first queued item for `session_id`, if any.
    pub fn take_followup(&mut self, session_id: &str) -> Result<Option<Followup>> {
        let Some(queue) = self.followups.get_mut(session_id) else {
            return Ok(None);
        };
        if queue.is_empty() {
            self.followups.remove(session_id);
            return Ok(None);
        }
        let taken = queue.remove(0);
        if queue.is_empty() {
            self.followups.remove(session_id);
        }
        if let Some(db) = self.db.as_ref() {
            db.delete_followup(taken.id)?;
        }
        Ok(Some(taken))
    }

    /// A feature was deleted: drop its sessions' queues and settings files.
    pub(crate) fn forget_deleted_feature_sessions(&mut self, feature: &crate::project::Feature) {
        for session in &feature.sessions {
            self.clear_followups(&session.id);
            crate::fork::launch::remove_claude_settings(&session.id);
        }
    }

    /// Forget a session's queue (the session was deleted).
    pub fn clear_followups(&mut self, session_id: &str) {
        if self.followups.remove(session_id).is_some()
            && let Some(db) = self.db.as_ref()
            && let Err(err) = db.clear_followups(session_id)
        {
            self.log_warn("followup", format!("could not clear the queue: {err}"));
        }
    }

    /// A Stop hook asks whether something is queued. Returns `false` when
    /// `raw` is not that message.
    pub(crate) fn handle_followup_take_message(&mut self, raw: &Value) -> bool {
        if raw.get("type").and_then(Value::as_str) != Some(TAKE_TYPE) {
            return false;
        }
        let text = |key: &str| {
            raw.get(key)
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|v| !v.is_empty())
        };
        let Some(request_id) = text("request_id") else {
            return true;
        };
        let harness = text("harness").unwrap_or_default();
        let session_id = text("aiw_feature_session_id").or_else(|| {
            let aiw_session = text("aiw_tmux_session").or_else(|| text("aiw_session"));
            let cwd = text("cwd").unwrap_or_default();
            let (pi, fi) = self.fork_feature_for(aiw_session.as_deref(), &cwd)?;
            let sessions = &self.store.projects[pi].features[fi].sessions;
            sessions
                .iter()
                .find(|s| kind_matches(&s.kind, &harness))
                .or_else(|| sessions.iter().find(|s| s.kind.is_agent_harness()))
                .map(|s| s.id.clone())
        });
        let stop_hook_active = raw
            .get("stop_hook_active")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let taken = match session_id.as_deref() {
            Some(id) => self.take_followup(id).unwrap_or_else(|err| {
                self.log_warn("followup", format!("could not take a follow-up: {err}"));
                None
            }),
            None => None,
        };
        let delivered = crate::fork::ipc_reply::reply(
            &request_id,
            &take_reply(taken.as_ref().map(|f| f.text.as_str())),
        );
        match (&taken, session_id.as_deref()) {
            (Some(followup), Some(id)) => {
                let left = self.queued_followup_count(id);
                self.log_info(
                    "followup",
                    format!(
                        "delivered to session {id} ({harness}, stop_hook_active={stop_hook_active}, {left} left{}): {}",
                        if delivered { "" } else { "; the hook had stopped waiting" },
                        one_line(&followup.text)
                    ),
                );
                if !delivered {
                    // Put it back: the agent never saw it.
                    let queue = self.followups.entry(id.to_string()).or_default();
                    queue.insert(0, followup.clone());
                    if let Some(db) = self.db.as_ref()
                        && let Ok(fresh) = db.push_followup(id, &followup.text)
                    {
                        let ids: Vec<i64> = std::iter::once(fresh.id)
                            .chain(queue.iter().skip(1).map(|f| f.id))
                            .collect();
                        queue[0] = fresh;
                        let _ = db.reorder_followups(id, &ids);
                    }
                }
            }
            _ => self.log_debug(
                "followup",
                format!(
                    "nothing queued for {} ({harness}, stop_hook_active={stop_hook_active})",
                    session_id.as_deref().unwrap_or("an unknown session")
                ),
            ),
        }
        true
    }

    /// `aiw automation queue`: queue for a session id. Returns the response.
    pub(crate) fn automation_queue_followup(&mut self, raw: &Value) -> Value {
        let session_id = raw
            .get("session_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string();
        let text = raw.get("text").and_then(Value::as_str).unwrap_or_default();
        let known = self
            .store
            .projects
            .iter()
            .flat_map(|p| p.features.iter())
            .flat_map(|f| f.sessions.iter())
            .any(|s| s.id == session_id && s.kind.is_agent_harness());
        if !known {
            return crate::automation::automation_error_response(
                QUEUE_ACTION,
                format!("no agent session with id '{session_id}'"),
            );
        }
        match self.queue_followup(&session_id, text) {
            Ok(queued) => serde_json::json!({
                "type": crate::automation::AUTOMATION_RESULT_TYPE,
                "action": QUEUE_ACTION,
                "ok": true,
                "session_id": session_id,
                "queued": queued,
            }),
            Err(err) => crate::automation::automation_error_response(QUEUE_ACTION, err.to_string()),
        }
    }

    /// Load the queues from the database (at start).
    pub(crate) fn load_followups(&mut self) {
        if let Some(db) = self.db.as_ref() {
            match db.all_followups() {
                Ok(queues) => self.followups = queues,
                Err(err) => self.log_warn("followup", format!("could not load the queues: {err}")),
            }
        }
    }

    /// The agent session the selected row stands for.
    fn followup_target_for_selection(&self) -> Option<(usize, usize, usize)> {
        match self.selection {
            Selection::Session(pi, fi, si) => {
                let session = self
                    .store
                    .projects
                    .get(pi)?
                    .features
                    .get(fi)?
                    .sessions
                    .get(si)?;
                session.kind.is_agent_harness().then_some((pi, fi, si))
            }
            Selection::Feature(pi, fi) => {
                let feature = self.store.projects.get(pi)?.features.get(fi)?;
                feature
                    .sessions
                    .iter()
                    .position(|s| s.kind.is_agent_harness() && !s.stopped)
                    .map(|si| (pi, fi, si))
            }
            Selection::Project(_) => None,
        }
    }

    /// Open the dialog for the selected row. Returns `false` when the row is
    /// not an agent session.
    pub fn open_followup_queue_for_selection(&mut self) -> bool {
        let Some((pi, fi, si)) = self.followup_target_for_selection() else {
            self.message = Some("Pick an agent session to queue a follow-up for".into());
            return false;
        };
        let previous = std::mem::replace(&mut self.mode, AppMode::Normal);
        self.open_followup_queue(pi, fi, si, previous);
        true
    }

    fn open_followup_queue(&mut self, pi: usize, fi: usize, si: usize, previous: AppMode) {
        let project = &self.store.projects[pi];
        let feature = &project.features[fi];
        let session = &feature.sessions[si];
        let title = format!(
            "{} › {} › {}",
            project.name,
            feature
                .nickname
                .clone()
                .unwrap_or_else(|| feature.name.clone()),
            session.label
        );
        self.mode = AppMode::FollowupQueue(Box::new(FollowupQueueState {
            session_id: session.id.clone(),
            title,
            editor: TextEditor::new(String::new()),
            scroll_offset: 0,
            sync_scroll_to_cursor: true,
            selected: 0,
            focus: FollowupFocus::Editor,
            previous_mode: Box::new(previous),
        }));
    }

    /// Close the dialog, back to where it was opened from.
    pub fn close_followup_queue(&mut self) {
        if let AppMode::FollowupQueue(state) = std::mem::replace(&mut self.mode, AppMode::Normal) {
            self.mode = *state.previous_mode;
        }
    }
}

fn kind_matches(kind: &SessionKind, harness: &str) -> bool {
    matches!(
        (kind, harness),
        (SessionKind::Claude, "claude")
            | (SessionKind::Codex, "codex")
            | (SessionKind::Gemini, "gemini")
            | (SessionKind::Cursor, "cursor")
    )
}

fn one_line(text: &str) -> String {
    let joined: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut shown: String = joined.chars().take(120).collect();
    if shown.chars().count() < joined.chars().count() {
        shown.push('…');
    }
    shown
}

/// Queues loaded by [`App::load_followups`]; a fresh `App` starts empty.
pub(crate) type FollowupQueues = HashMap<String, Vec<Followup>>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::badges::BadgeKind;
    use serde_json::json;

    fn app() -> App {
        crate::app::fork_workspace::workspace_app(&[SessionKind::Claude, SessionKind::Codex])
    }

    /// A `followup_take` with a parked connection to answer on, as the Stop
    /// hook would hold; returns the message and the hook's end of the pipe.
    fn take(session_id: Option<&str>, harness: &str) -> (Value, std::os::unix::net::UnixStream) {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let request_id = format!(
            "take-{}",
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let (app_end, hook_end) = std::os::unix::net::UnixStream::pair().unwrap();
        crate::fork::ipc_reply::register(&request_id, app_end);
        let mut raw = json!({
            "type": "followup_take", "request_id": request_id, "harness": harness,
            "aiw_session": "aiw-hx-12", "cwd": "/tmp", "stop_hook_active": false
        });
        if let Some(id) = session_id {
            raw["aiw_feature_session_id"] = json!(id);
        }
        (raw, hook_end)
    }

    fn answer(hook_end: std::os::unix::net::UnixStream) -> Value {
        use std::io::BufRead;
        let mut line = String::new();
        std::io::BufReader::new(hook_end)
            .read_line(&mut line)
            .unwrap();
        serde_json::from_str(line.trim()).unwrap()
    }

    #[test]
    fn the_queue_is_a_fifo_per_session_and_decorates_the_badge() {
        let mut app = app();
        let claude = app.store.projects[0].features[0].sessions[0].id.clone();
        assert_eq!(app.queue_followup(&claude, " first ").unwrap(), 1);
        assert_eq!(app.queue_followup(&claude, "second").unwrap(), 2);
        assert!(app.queue_followup(&claude, "   ").is_err());
        assert_eq!(app.queued_followup_count(&claude), 2);
        let badge = app.session_badge_at(0, 0, 0).unwrap();
        assert_eq!(badge.kind, BadgeKind::Idle);
        assert_eq!(badge.label(chrono::Utc::now()), "idle · ⏭ 2 queued");
        assert_eq!(app.session_badge_at(0, 0, 1).unwrap().queued, 0);

        // Reorder and delete.
        assert_eq!(app.move_followup(&claude, 1, -1).unwrap(), 0);
        assert_eq!(app.followups_for(&claude)[0].text, "second");
        assert_eq!(
            app.move_followup(&claude, 0, -1).unwrap(),
            0,
            "stays at the top"
        );
        app.delete_followup(&claude, 0).unwrap();
        assert_eq!(app.followups_for(&claude)[0].text, "first");

        // Delivery pops the first; then nothing.
        let (raw, hook) = take(Some(&claude), "claude");
        assert!(app.handle_followup_take_message(&raw));
        assert_eq!(answer(hook)["followup"], "first");
        assert_eq!(app.queued_followup_count(&claude), 0);
        let (raw, hook) = take(Some(&claude), "claude");
        assert!(app.handle_followup_take_message(&raw));
        assert_eq!(answer(hook)["followup"], Value::Null);
        assert!(!app.handle_followup_take_message(&json!({"type": "stop"})));

        // A hook that stopped waiting never saw the item: it stays queued.
        app.queue_followup(&claude, "kept").unwrap();
        let (raw, hook) = take(Some(&claude), "claude");
        drop(hook);
        crate::fork::ipc_reply::abandon(raw["request_id"].as_str().unwrap());
        assert!(app.handle_followup_take_message(&raw));
        assert_eq!(app.followups_for(&claude)[0].text, "kept");
    }

    #[test]
    fn a_take_without_the_session_id_lands_on_the_harness_window() {
        let mut app = app();
        let codex = app.store.projects[0].features[0].sessions[1].id.clone();
        app.queue_followup(&codex, "codex next").unwrap();
        let (raw, hook) = take(None, "codex");
        assert!(app.handle_followup_take_message(&raw));
        assert_eq!(answer(hook)["followup"], "codex next");
        assert_eq!(app.queued_followup_count(&codex), 0);
    }

    #[test]
    fn the_dialog_opens_for_agent_rows_and_closes_back() {
        let mut app = app();
        app.selection = Selection::Project(0);
        assert!(!app.open_followup_queue_for_selection());
        app.selection = Selection::Session(0, 0, 1);
        assert!(app.open_followup_queue_for_selection());
        let AppMode::FollowupQueue(state) = &app.mode else {
            panic!("dialog");
        };
        assert_eq!(state.title, "RS › hx-12 › Codex 1");
        assert_eq!(state.focus, FollowupFocus::Editor);
        app.close_followup_queue();
        assert!(matches!(app.mode, AppMode::Normal));
        app.selection = Selection::Feature(0, 0);
        assert!(app.open_followup_queue_for_selection());
    }

    #[test]
    fn automation_queues_for_a_known_session_only() {
        let mut app = app();
        let claude = app.store.projects[0].features[0].sessions[0].id.clone();
        let ok = app.automation_queue_followup(&json!({"session_id": claude, "text": "rebase"}));
        assert_eq!(ok["ok"], true);
        assert_eq!(ok["queued"], 1);
        let bad = app.automation_queue_followup(&json!({"session_id": "nope", "text": "x"}));
        assert_eq!(bad["ok"], false);
        let empty = app.automation_queue_followup(&json!({"session_id": claude, "text": " "}));
        assert_eq!(empty["ok"], false);
    }
}
