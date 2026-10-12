//! FORK: the sidebar's own keys in the cockpit (DESIGN.md §18).
//!
//! Every dashboard key keeps its meaning; this file adds the two that are
//! about the agent pane beside the sidebar: `Enter` on a running session or
//! feature focuses it (`select-pane`, the agent's real tmux pane — typing
//! there is native), and `v` puts `aiw grid` in it. Nothing is typed
//! through AIW and nothing is captured: there is no pane focus, no `Tab`,
//! no layout to toggle.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{App, AppMode, Selection};

/// Returns `true` when the key was consumed.
pub fn handle_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    if !matches!(app.mode, AppMode::Normal)
        || app.leader_active
        || key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        || !app.workspace_active()
    {
        return Ok(false);
    }
    match key.code {
        // Enter on a running session or feature focuses the agent pane;
        // anything else (a stopped session's recovery dialog,
        // expand/collapse) is the dashboard's own Enter.
        // FORK: §19.4 — Enter on a collapsed group's row opens the group.
        KeyCode::Enter
            if matches!(
                app.selection,
                Selection::Feature(pi, fi) if app.collapsed_group_at(pi, fi).is_some()
            ) =>
        {
            Ok(false)
        }
        KeyCode::Enter
            if matches!(
                app.selection,
                Selection::Session(..) | Selection::Feature(..)
            ) && app.desired_live_target_exists() =>
        {
            app.cockpit_focus_agent();
            Ok(true)
        }
        KeyCode::Char('v') => {
            app.cockpit_show_grid();
            Ok(true)
        }
        _ => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::fork_workspace::workspace_app;
    use crate::fork::cockpit::{Link, Request};
    use crate::project::{ProjectStatus, SessionKind};

    fn app() -> App {
        let mut app = workspace_app(&[SessionKind::Claude, SessionKind::Codex]);
        app.cockpit_attach(
            Link::recording(),
            "/bin/aiw".into(),
            "/s/cockpit.sock".into(),
        );
        app.fork_cockpit.link.as_ref().unwrap().clear_recorded();
        app
    }

    fn recorded(app: &App) -> Vec<Request> {
        app.fork_cockpit.link.as_ref().unwrap().recorded()
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn enter_on_a_running_session_focuses_the_agent_pane() {
        let mut app = app();
        assert!(handle_key(&mut app, key(KeyCode::Enter)).unwrap());
        let requests = recorded(&app);
        assert!(
            matches!(&requests[0], Request::Respawn { title, .. } if title == "RS › hx-12 › Claude 1"),
            "{requests:?}"
        );
        assert_eq!(requests[1], Request::FocusAgent);
        assert!(matches!(app.mode, AppMode::Normal), "no view, no dialog");
    }

    #[test]
    fn enter_on_a_stopped_feature_is_the_dashboards_own_enter() {
        let mut app = app();
        app.store.projects[0].features[0].status = ProjectStatus::Stopped;
        app.selection = Selection::Feature(0, 0);
        assert!(!handle_key(&mut app, key(KeyCode::Enter)).unwrap());
        assert!(recorded(&app).is_empty());
    }

    #[test]
    fn v_puts_the_grid_in_the_agent_pane() {
        let mut app = app();
        crate::handlers::handle_normal_key(&mut app, key(KeyCode::Char('v'))).unwrap();
        assert!(
            matches!(recorded(&app).last(), Some(Request::Respawn { command, .. }) if command.contains("grid RS/hx-12"))
        );
        assert!(matches!(app.mode, AppMode::Normal));
    }

    #[test]
    fn b_o_and_tab_are_not_sidebar_keys() {
        let mut app = app();
        for code in [KeyCode::Char('b'), KeyCode::Char('o'), KeyCode::Tab] {
            assert!(!handle_key(&mut app, key(code)).unwrap());
        }
        assert!(recorded(&app).is_empty());
        // Ctrl+Q belongs to tmux: the sidebar does nothing with it.
        let ctrl_q = KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL);
        assert!(!handle_key(&mut app, ctrl_q).unwrap());
        crate::handlers::handle_normal_key(&mut app, ctrl_q).unwrap();
        assert!(!app.should_quit);
    }
}
