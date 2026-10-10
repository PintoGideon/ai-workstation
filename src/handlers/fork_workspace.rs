//! FORK: keys and mouse for the workspace dashboard layout (DESIGN.md §15).
//!
//! With the sidebar focused every dashboard key keeps its meaning; this
//! file only adds `b` (layout toggle, in both layouts), `Enter` (attach
//! natively, DESIGN.md §15.4), `Tab` (type through the in-app pane) and `o`
//! (open the full view). With the pane focused,
//! keys go to the agent through the single view's `forward_tmux_key` with
//! the same compose rules; `Ctrl+Q` returns to the sidebar and `Ctrl+Space`
//! opens the view's leader, reusing `handle_leader_key`.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::fork_workspace::LiveTarget;
use crate::app::{App, AppMode, Selection};
use crate::project::SessionKind;

/// The dashboard key that toggles the layout (`w` is context settings).
pub const LAYOUT_TOGGLE_KEY: char = 'b';

/// Returns `true` when the key was consumed.
pub fn handle_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    if !matches!(app.mode, AppMode::Normal) {
        return Ok(false);
    }
    if app.workspace_pane_focused() {
        handle_pane_key(app, key)?;
        return Ok(true);
    }
    if app.leader_active
        || key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return Ok(false);
    }
    // A user who bound `b` to a dashboard action keeps it.
    if key.code == KeyCode::Char(LAYOUT_TOGGLE_KEY)
        && !app
            .active_extension
            .keybindings
            .values()
            .any(|&bound| bound == LAYOUT_TOGGLE_KEY)
    {
        app.toggle_dashboard_layout();
        return Ok(true);
    }
    if !app.workspace_active() {
        return Ok(false);
    }
    match key.code {
        KeyCode::Tab => {
            if !app.workspace_focus_pane() {
                app.message = Some("Nothing to type into here — pick a running session".into());
            }
            Ok(true)
        }
        // Enter on a running session or feature attaches to it natively
        // (the terminal's own typing, no proxy); anything else (a stopped
        // session's recovery dialog, expand/collapse) is the dashboard's
        // own Enter.
        KeyCode::Enter
            if matches!(
                app.selection,
                Selection::Session(..) | Selection::Feature(..)
            ) && !app.paused_plan_interview_matches_selection()
                && app.desired_live_target_exists()
                && app.request_attach_selected() =>
        {
            Ok(true)
        }
        KeyCode::Char('o') => {
            if matches!(
                app.selection,
                Selection::Session(..) | Selection::Feature(..)
            ) && !app.open_stopped_session_dialog()?
            {
                app.enter_view()?;
            }
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn handle_pane_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let Some(live) = app.fork_workspace.live.clone() else {
        app.workspace_focus_sidebar();
        return Ok(());
    };
    if app.leader_active {
        app.deactivate_leader();
        return handle_pane_leader_key(app, key, live);
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('q') {
        app.workspace_focus_sidebar();
        return Ok(());
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char(' ') {
        let _ = app.flush_view_input_batch();
        app.activate_leader();
        return Ok(());
    }
    // The compose box, when it is on for this session: typing opens it
    // over the full view of the same session (the box returns there).
    let view = live.view_state();
    if app.compose_intercept_active(&view)
        && let KeyCode::Char(c) = key.code
        && !key.modifiers.contains(KeyModifiers::CONTROL)
        && !key.modifiers.contains(KeyModifiers::ALT)
    {
        let _ = app.flush_view_input_batch();
        app.mode = AppMode::Viewing(view);
        app.refresh_sidebar_for_current_view();
        return app.open_compose_from_view(Some(c));
    }
    match super::view::forward_tmux_key(app, &key, &live.session, &live.window) {
        Ok(submitted) => {
            if submitted && live.kind == SessionKind::Codex {
                app.note_codex_prompt_submit(&live.session, &live.window);
            }
            app.mark_finished_seen(&live.session_id);
        }
        Err(e) => app.show_error(e),
    }
    Ok(())
}

/// Leader keys in the pane. `q` and `z` are the pane's own (back to the
/// sidebar, open the full view); everything else is the full view's leader,
/// run against the same session.
fn handle_pane_leader_key(app: &mut App, key: KeyEvent, live: LiveTarget) -> Result<()> {
    match key.code {
        KeyCode::Char('q') => {
            app.workspace_focus_sidebar();
            return Ok(());
        }
        KeyCode::Char('z') => {
            app.workspace_select_window(&live.session, &live.window);
            app.workspace_focus_sidebar();
            app.enter_view()?;
            return Ok(());
        }
        KeyCode::Enter => {
            app.request_attach_window(&live.session, &live.window);
            return Ok(());
        }
        // P7: queue a follow-up for the shown session; closes back here.
        KeyCode::Char(super::fork_followups::LEADER_QUEUE_KEY) => {
            app.open_followup_queue_for_view(&live.view_state(), AppMode::Normal);
            return Ok(());
        }
        _ => {}
    }
    app.mode = AppMode::Viewing(live.view_state());
    let visible_rows = app.viewport_rows;
    let result = super::view::handle_leader_key(app, key, visible_rows);
    match std::mem::replace(&mut app.mode, AppMode::Normal) {
        // Still a view of some session: back to the dashboard, the sidebar
        // on whichever session the action landed on (t/T move).
        AppMode::Viewing(view) => {
            if (view.session.as_str(), view.window.as_str())
                != (live.session.as_str(), live.window.as_str())
            {
                app.workspace_select_window(&view.session, &view.window);
                app.fork_workspace.focus = crate::app::fork_workspace::WorkspaceFocus::Pane;
            }
        }
        // Overlays that would close back into the full view close into
        // the workspace instead.
        AppMode::NotificationPicker(index, Some(_)) => {
            app.mode = AppMode::NotificationPicker(index, None);
        }
        AppMode::Help(state) => {
            app.mode = AppMode::Help(crate::app::HelpState {
                from_view: None,
                scroll_offset: state.scroll_offset,
            });
        }
        other => app.mode = other,
    }
    result
}

/// A left click on the pane focuses it; on the sidebar it focuses the
/// sidebar and lets the dashboard's row hit test run. Returns `true` when
/// the click was consumed.
pub fn handle_click(app: &mut App, col: u16, row: u16) -> Result<bool> {
    if !matches!(app.mode, AppMode::Normal) || !app.workspace_active() {
        return Ok(false);
    }
    let inside = |rect: Option<ratatui::layout::Rect>| {
        rect.is_some_and(|r| {
            col >= r.x && col < r.x.saturating_add(r.width) && row >= r.y && row < r.y + r.height
        })
    };
    if inside(app.fork_workspace.pane_rect) {
        app.workspace_focus_pane();
        return Ok(true);
    }
    if inside(app.fork_workspace.sidebar_rect) {
        app.workspace_focus_sidebar();
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::fork_workspace::{SELECTION_DEBOUNCE, WorkspaceFocus};
    use crate::fork::config::DashboardLayout;
    use crate::project::{AgentKind, Feature, Project, ProjectStatus, ProjectStore, VibeMode};
    use crate::traits::{MockTmuxOps, MockWorktreeOps};
    use std::path::PathBuf;

    fn live() -> MockTmuxOps {
        let mut tmux = MockTmuxOps::new();
        tmux.expect_session_exists().returning(|_| true);
        tmux.expect_window_exists().returning(|_, _| true);
        tmux
    }

    fn app_with(tmux: MockTmuxOps) -> App {
        let mut feature = Feature::new(
            "hx-12".into(),
            "hx-12".into(),
            PathBuf::from("/tmp"),
            false,
            VibeMode::Vibeless,
            false,
            false,
            AgentKind::Claude,
            false,
            false,
        );
        feature.status = ProjectStatus::Active;
        feature.add_session(SessionKind::Claude);
        feature.add_session(SessionKind::Codex);
        let mut project = Project::new("RS".into(), PathBuf::from("/tmp"), true, AgentKind::Claude);
        project.features.push(feature);
        let store = ProjectStore {
            version: 5,
            projects: vec![project],
            session_bookmarks: vec![],
            available_harnesses: vec![],
            prompt_templates: Vec::new(),
            extra: Default::default(),
        };
        let mut app = App::new_for_test(store, Box::new(tmux), Box::new(MockWorktreeOps::new()));
        app.config.aiw.dashboard_layout = DashboardLayout::Workspace;
        app.viewport_cols = 120;
        app.viewport_rows = 37;
        app.viewport_total_rows = 40;
        app.selection = Selection::Session(0, 0, 0);
        app
    }

    fn settle(app: &mut App) {
        app.workspace_tick();
        std::thread::sleep(SELECTION_DEBOUNCE + std::time::Duration::from_millis(20));
        app.workspace_tick();
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    #[test]
    fn b_toggles_the_layout_in_both_layouts() {
        let mut app = app_with(MockTmuxOps::new());
        app.config.aiw.dashboard_layout = DashboardLayout::Tree;
        crate::handlers::handle_normal_key(&mut app, key(KeyCode::Char('b'))).unwrap();
        assert_eq!(app.config.aiw.dashboard_layout, DashboardLayout::Workspace);
        assert!(app.message.as_deref().unwrap().contains("workspace"));
        crate::handlers::handle_normal_key(&mut app, key(KeyCode::Char('b'))).unwrap();
        assert_eq!(app.config.aiw.dashboard_layout, DashboardLayout::Tree);
    }

    #[test]
    fn tab_focuses_the_pane_and_typing_reaches_the_agent() {
        let mut tmux = live();
        tmux.expect_send_literal()
            .withf(|session, window, text| {
                session == "aiw-hx-12" && window == "claude" && text == "hi"
            })
            .times(1)
            .returning(|_, _, _| Ok(()));
        tmux.expect_send_key_name()
            .withf(|session, window, name| {
                session == "aiw-hx-12" && window == "claude" && name == "Enter"
            })
            .times(1)
            .returning(|_, _, _| Ok(()));
        let mut app = app_with(tmux);
        settle(&mut app);
        assert!(handle_key(&mut app, key(KeyCode::Tab)).unwrap());
        assert_eq!(app.fork_workspace.focus, WorkspaceFocus::Pane);
        for c in ['h', 'i'] {
            assert!(handle_key(&mut app, key(KeyCode::Char(c))).unwrap());
        }
        assert!(handle_key(&mut app, key(KeyCode::Enter)).unwrap());
        // A dashboard key is not a dashboard key while the pane has focus.
        assert!(matches!(app.mode, AppMode::Normal));
    }

    #[test]
    fn enter_on_a_session_requests_a_native_attach_not_the_pane() {
        let mut app = app_with(live());
        assert!(!app.workspace_tick() && app.fork_workspace.live.is_none());
        assert!(handle_key(&mut app, key(KeyCode::Enter)).unwrap());
        let req = app.pending_attach.clone().expect("attach requested");
        assert_eq!(
            (req.session.as_str(), req.window.as_str()),
            ("aiw-hx-12", "claude")
        );
        assert_eq!(req.title, "RS › hx-12 › Claude 1");
        assert_eq!(app.fork_workspace.focus, WorkspaceFocus::Sidebar);
        assert!(matches!(app.mode, AppMode::Normal), "not the full view");
    }

    #[test]
    fn enter_on_a_stopped_feature_is_the_dashboards_own_enter() {
        let mut app = app_with(live());
        app.store.projects[0].features[0].status = ProjectStatus::Stopped;
        app.selection = Selection::Feature(0, 0);
        assert!(!handle_key(&mut app, key(KeyCode::Enter)).unwrap());
        assert!(app.pending_attach.is_none());
    }

    #[test]
    fn tab_still_types_through_the_pane() {
        let mut app = app_with(live());
        app.workspace_tick();
        assert!(handle_key(&mut app, key(KeyCode::Tab)).unwrap());
        assert_eq!(app.fork_workspace.focus, WorkspaceFocus::Pane);
        assert!(app.pending_attach.is_none());
    }

    #[test]
    fn the_pane_leader_enter_attaches_to_the_live_session() {
        let mut app = app_with(live());
        settle(&mut app);
        handle_key(&mut app, key(KeyCode::Tab)).unwrap();
        handle_key(&mut app, ctrl(' ')).unwrap();
        assert!(app.leader_active);
        assert!(handle_key(&mut app, key(KeyCode::Enter)).unwrap());
        assert_eq!(
            app.pending_attach.as_ref().map(|r| r.window.as_str()),
            Some("claude")
        );
        assert!(matches!(app.mode, AppMode::Normal));
    }

    #[test]
    fn ctrl_q_returns_to_the_sidebar_not_the_tree() {
        let mut app = app_with(live());
        settle(&mut app);
        handle_key(&mut app, key(KeyCode::Tab)).unwrap();
        assert!(handle_key(&mut app, ctrl('q')).unwrap());
        assert_eq!(app.fork_workspace.focus, WorkspaceFocus::Sidebar);
        assert!(matches!(app.mode, AppMode::Normal));
        assert!(app.workspace_active());
        // On the sidebar Ctrl+Q is the dashboard's no-op.
        assert!(!handle_key(&mut app, ctrl('q')).unwrap());
        assert!(!app.should_quit);
    }

    #[test]
    fn leader_in_the_pane_reuses_the_views_commands() {
        let mut app = app_with(live());
        settle(&mut app);
        handle_key(&mut app, key(KeyCode::Tab)).unwrap();
        // i with nothing pending: the view's toast, still on the dashboard.
        handle_key(&mut app, ctrl(' ')).unwrap();
        assert!(app.leader_active);
        handle_key(&mut app, key(KeyCode::Char('i'))).unwrap();
        assert!(!app.leader_active);
        assert!(matches!(app.mode, AppMode::Normal));
        assert!(
            app.toasts
                .iter()
                .any(|t| t.message.contains("Nothing needs you"))
        );
        // ? opens the help overlay that closes back to the workspace.
        handle_key(&mut app, ctrl(' ')).unwrap();
        handle_key(&mut app, key(KeyCode::Char('?'))).unwrap();
        assert!(matches!(app.mode, AppMode::Help(ref s) if s.from_view.is_none()));
        app.mode = AppMode::Normal;
        // t moves to the next session of the feature; the sidebar follows.
        handle_key(&mut app, ctrl(' ')).unwrap();
        handle_key(&mut app, key(KeyCode::Char('t'))).unwrap();
        assert!(
            matches!(app.selection, Selection::Session(0, 0, 1)),
            "{:?}",
            app.selection
        );
        assert_eq!(app.fork_workspace.focus, WorkspaceFocus::Pane);
        // q goes back to the sidebar.
        handle_key(&mut app, ctrl(' ')).unwrap();
        handle_key(&mut app, key(KeyCode::Char('q'))).unwrap();
        assert_eq!(app.fork_workspace.focus, WorkspaceFocus::Sidebar);
        assert!(matches!(app.mode, AppMode::Normal));
    }

    #[test]
    fn v_opens_split_view_and_leaving_it_returns_to_the_workspace() {
        let mut app = app_with(live());
        settle(&mut app);
        crate::handlers::handle_normal_key(&mut app, key(KeyCode::Char('v'))).unwrap();
        assert!(matches!(app.mode, AppMode::SplitView(_)));
        assert!(!app.workspace_pane_live(), "one capture target: the tiles'");
        app.leave_split_view();
        assert!(matches!(app.mode, AppMode::Normal));
        assert!(app.workspace_active());
    }

    #[test]
    fn clicks_focus_the_half_under_the_pointer() {
        use ratatui::layout::Rect;
        let mut app = app_with(live());
        settle(&mut app);
        app.fork_workspace.sidebar_rect = Some(Rect::new(0, 3, 30, 34));
        app.fork_workspace.pane_rect = Some(Rect::new(30, 3, 90, 34));
        assert!(handle_click(&mut app, 60, 10).unwrap());
        assert_eq!(app.fork_workspace.focus, WorkspaceFocus::Pane);
        assert!(
            !handle_click(&mut app, 5, 10).unwrap(),
            "row hit test still runs"
        );
        assert_eq!(app.fork_workspace.focus, WorkspaceFocus::Sidebar);
        assert!(
            !handle_click(&mut app, 60, 1).unwrap(),
            "header is not ours"
        );
    }

    #[test]
    fn keys_sent_to_the_pane_mark_its_finished_turn_seen() {
        use crate::app::attention::{AttentionSource, AttentionState};
        use crate::fork::badges::BadgeKind;
        let mut tmux = live();
        tmux.expect_send_key_name().returning(|_, _, _| Ok(()));
        let mut app = app_with(tmux);
        settle(&mut app);
        handle_key(&mut app, key(KeyCode::Tab)).unwrap();
        app.record_attention(
            "aiw-hx-12",
            &AgentKind::Claude,
            AttentionState::CompletedAwaitingReview,
            AttentionSource::Hook,
        );
        assert_eq!(
            app.session_badge_at(0, 0, 0).unwrap().kind,
            BadgeKind::FinishedUnread
        );
        handle_key(&mut app, key(KeyCode::Enter)).unwrap();
        assert_eq!(
            app.session_badge_at(0, 0, 0).unwrap().kind,
            BadgeKind::FinishedRead
        );
    }
}
