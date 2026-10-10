//! FORK: keys that answer a pending permission request (P6, DESIGN.md §16).
//!
//! `y` (allow) and `Y` (deny) were the obvious keys, but `y` is "mark the
//! feature ready" on the dashboard, so the keys are `a` (allow — approve)
//! and `X` (deny — the shifted stop key), the same on a selected row, in the
//! needs-you picker and in the detail overlay. They only do anything while
//! the row's session has a request waiting, so no existing binding changes.
//! `Enter` in the picker on such a row opens the detail overlay (tool and
//! full input) instead of jumping to the session; `o` there opens it.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::attention::AttentionRow;
use crate::app::{App, AppMode};
use crate::fork::permissions::Behavior;

pub const ALLOW_KEY: char = 'a';
pub const DENY_KEY: char = 'X';
pub const PASS_KEY: char = 'p';

fn plain(key: KeyEvent) -> bool {
    !key.modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
}

/// A dashboard key on the selected row. Returns `true` when consumed.
pub fn handle_dashboard_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    if !matches!(app.mode, AppMode::Normal) || app.leader_active || !plain(key) {
        return Ok(false);
    }
    let behavior = match key.code {
        KeyCode::Char(ALLOW_KEY) => Behavior::Allow,
        KeyCode::Char(DENY_KEY) => Behavior::Deny,
        _ => return Ok(false),
    };
    let Some(request_id) = app
        .pending_permission_for_selection()
        .map(|p| p.request_id.clone())
    else {
        return Ok(false);
    };
    app.answer_permission(&request_id, behavior);
    Ok(true)
}

/// The request behind the needs-you picker's row `index`, if any.
fn picker_request_id(app: &App, index: usize) -> Option<String> {
    let row = app.attention_rows().into_iter().nth(index)?;
    let AttentionRow::Attention { entry, .. } = row else {
        return None;
    };
    app.pending_permission_for_tmux_session(&entry.tmux_session)
        .map(|p| p.request_id.clone())
}

/// A key in the needs-you picker. Returns `true` when consumed.
pub fn handle_picker_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    let AppMode::NotificationPicker(index, _) = &app.mode else {
        return Ok(false);
    };
    if !plain(key) {
        return Ok(false);
    }
    let Some(request_id) = picker_request_id(app, *index) else {
        return Ok(false);
    };
    match key.code {
        KeyCode::Char(ALLOW_KEY) => {
            app.answer_permission(&request_id, Behavior::Allow);
        }
        KeyCode::Char(DENY_KEY) => {
            app.answer_permission(&request_id, Behavior::Deny);
        }
        KeyCode::Enter => app.open_permission_detail(&request_id),
        _ => return Ok(false),
    }
    if matches!(app.mode, AppMode::NotificationPicker(..)) && app.attention_rows().is_empty() {
        app.mode = AppMode::Normal;
    }
    Ok(true)
}

/// Keys in the detail overlay.
pub fn handle_detail_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let AppMode::PermissionDetail(state) = &app.mode else {
        return Ok(());
    };
    let request_id = state.request_id.clone();
    match key.code {
        KeyCode::Char(ALLOW_KEY) => {
            app.answer_permission(&request_id, Behavior::Allow);
        }
        KeyCode::Char(DENY_KEY) => {
            app.answer_permission(&request_id, Behavior::Deny);
        }
        KeyCode::Char(PASS_KEY) => {
            app.answer_permission(&request_id, Behavior::Pass);
        }
        KeyCode::Char('o') | KeyCode::Enter => {
            app.open_permission_session(&request_id)?;
        }
        KeyCode::Esc | KeyCode::Char('q') => app.close_permission_detail(),
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Selection;
    use crate::fork::badges::BadgeKind;
    use crate::project::SessionKind;
    use serde_json::json;

    fn app_with_request(id: &str) -> App {
        let mut app =
            crate::app::fork_workspace::workspace_app(&[SessionKind::Claude, SessionKind::Codex]);
        let session_id = app.store.projects[0].features[0].sessions[0].id.clone();
        assert!(app.handle_permission_request_message(&json!({
            "type": "permission_request", "request_id": id, "harness": "claude",
            "aiw_session": "aiw-hx-12", "aiw_feature_session_id": session_id,
            "cwd": "/tmp", "tool_name": "Bash", "tool_input": {"command": "cargo test"}
        })));
        app.selection = Selection::Session(0, 0, 0);
        app
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[test]
    fn a_and_shift_x_answer_the_selected_rows_request() {
        let mut app = app_with_request("r1");
        assert!(crate::handlers::handle_normal_key(&mut app, key('a')).is_ok());
        assert!(app.pending_permissions.is_empty());
        assert_eq!(app.session_badge_at(0, 0, 0).unwrap().kind, BadgeKind::Idle);

        let mut app = app_with_request("r2");
        assert!(handle_dashboard_key(&mut app, key('X')).unwrap());
        assert!(app.pending_permissions.is_empty());
    }

    #[test]
    fn the_keys_do_nothing_on_a_row_without_a_request() {
        let mut app = app_with_request("r3");
        app.selection = Selection::Session(0, 0, 1);
        assert!(!handle_dashboard_key(&mut app, key('a')).unwrap());
        assert!(!handle_dashboard_key(&mut app, key('X')).unwrap());
        assert_eq!(app.pending_permissions.len(), 1);
        // `y` still marks the feature ready rather than answering.
        crate::handlers::handle_normal_key(&mut app, key('y')).unwrap();
        assert_eq!(app.pending_permissions.len(), 1);
    }

    #[test]
    fn the_picker_answers_and_enter_opens_the_detail() {
        let mut app = app_with_request("r4");
        app.mode = AppMode::NotificationPicker(0, None);
        assert!(
            handle_picker_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)).unwrap()
        );
        assert!(matches!(app.mode, AppMode::PermissionDetail(_)));
        handle_detail_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)).unwrap();
        assert!(matches!(app.mode, AppMode::NotificationPicker(0, None)));
        assert!(handle_picker_key(&mut app, key('a')).unwrap());
        assert!(app.pending_permissions.is_empty());
        assert!(matches!(app.mode, AppMode::Normal), "nothing left to pick");
    }

    #[test]
    fn the_overlay_answers_passes_and_opens_the_session() {
        let mut app = app_with_request("r5");
        app.open_permission_detail("r5");
        handle_detail_key(&mut app, key('X')).unwrap();
        assert!(app.pending_permissions.is_empty());
        assert!(matches!(app.mode, AppMode::Normal));

        let mut app = app_with_request("r6");
        app.open_permission_detail("r6");
        handle_detail_key(&mut app, key('p')).unwrap();
        assert!(app.pending_permissions.is_empty());

        let mut app = app_with_request("r7");
        app.selection = Selection::Project(0);
        app.open_permission_detail("r7");
        handle_detail_key(&mut app, key('o')).unwrap();
        assert!(matches!(app.selection, Selection::Session(0, 0, 0)));
        assert_eq!(app.pending_permissions.len(), 1, "opening is not answering");
    }
}
