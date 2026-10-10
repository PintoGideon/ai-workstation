//! Keys, mouse and paste in split view (DESIGN.md §4.1). Routing decisions
//! are the pure `crate::fork::split::route_key`; keys for the agent go
//! through the single view's own `forward_tmux_key`, so batching, named-key
//! mapping and Codex submit tracking behave exactly as there.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

use crate::app::{App, AppMode};
use crate::fork::split::{self, SplitAction};
use crate::project::SessionKind;

fn focused_target(app: &App) -> Option<(String, String, SessionKind)> {
    match &app.mode {
        AppMode::SplitView(state) => state
            .focused_tile()
            .map(|t| (t.session.clone(), t.window.clone(), t.kind.clone())),
        _ => None,
    }
}

pub fn handle_split_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let leader = app.leader_active;
    if leader {
        app.deactivate_leader();
    }
    match split::route_key(leader, &key) {
        SplitAction::Forward => {
            let Some((session, window, kind)) = focused_target(app) else {
                return Ok(());
            };
            // `Ctrl+Space` twice: tmux names that key `C-Space`, not `C- `.
            if key.code == KeyCode::Char(' ') && key.modifiers.contains(KeyModifiers::CONTROL) {
                app.flush_view_input_batch()?;
                if let Err(err) = app.tmux.send_key_name(&session, &window, "C-Space")
                    && !app.handle_vanished_tmux_target(&err)
                {
                    return Err(err);
                }
                app.request_split_focus_refresh();
                return Ok(());
            }
            match super::view::forward_tmux_key(app, &key, &session, &window) {
                Ok(submitted) => {
                    if submitted && kind == SessionKind::Codex {
                        app.note_codex_prompt_submit(&session, &window);
                    }
                    app.request_split_focus_refresh();
                }
                Err(e) => app.show_error(e),
            }
        }
        SplitAction::Leader => {
            app.flush_view_input_batch()?;
            app.activate_leader();
        }
        SplitAction::Focus(dir) => {
            app.flush_view_input_batch()?;
            if let AppMode::SplitView(state) = &mut app.mode {
                state.move_focus(dir);
            }
        }
        SplitAction::Zoom => app.zoom_split_tile(),
        SplitAction::Leave => app.leave_split_view(),
        SplitAction::NextPage => turn_page(app, 1)?,
        SplitAction::PrevPage => turn_page(app, -1)?,
        SplitAction::FocusAttention => {
            app.flush_view_input_batch()?;
            app.focus_split_attention();
        }
        SplitAction::Nothing => {}
    }
    Ok(())
}

fn turn_page(app: &mut App, delta: isize) -> Result<()> {
    app.flush_view_input_batch()?;
    let AppMode::SplitView(state) = &mut app.mode else {
        return Ok(());
    };
    if state.page_count() <= 1 {
        app.push_toast_info("All sessions fit on one page");
    } else {
        state.turn_page(delta);
    }
    Ok(())
}

/// A left click focuses the tile under the pointer.
pub fn handle_split_mouse(app: &mut App, mouse: MouseEvent) -> Result<()> {
    if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
        return Ok(());
    }
    let AppMode::SplitView(state) = &app.mode else {
        return Ok(());
    };
    let Some(slot) = split::tile_at(&state.rects, mouse.column, mouse.row) else {
        return Ok(());
    };
    let index = state.visible().start + slot;
    if index != state.focus {
        app.flush_view_input_batch()?;
        if let AppMode::SplitView(state) = &mut app.mode {
            state.focus_tile(index);
        }
    }
    Ok(())
}

/// Paste goes to the focused tile, like typing.
pub fn handle_split_paste(app: &mut App, text: &str) -> Result<()> {
    let Some((session, window, _)) = focused_target(app) else {
        return Ok(());
    };
    app.flush_view_input_batch()?;
    app.tmux.paste_text(&session, &window, text)?;
    app.request_split_focus_refresh();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Selection;
    use crate::fork::split::Direction;
    use crate::project::{
        AgentKind, Feature, Project, ProjectStatus, ProjectStore, SessionKind, VibeMode,
    };
    use crate::traits::{MockTmuxOps, MockWorktreeOps};

    use ratatui::layout::Rect;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn app_with(tmux: MockTmuxOps, sessions: usize) -> App {
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
        for i in 0..sessions {
            feature.add_session(if i % 2 == 0 {
                SessionKind::Claude
            } else {
                SessionKind::Codex
            });
        }
        let mut project = Project::new("RS".into(), PathBuf::from("/tmp"), true, AgentKind::Claude);
        project.features.push(feature);
        let store = ProjectStore {
            version: 5,
            projects: vec![project],
            session_bookmarks: vec![],
            available_harnesses: vec![],
            prompt_templates: Vec::new(),
            extra: HashMap::new(),
        };
        let mut app = App::new_for_test(store, Box::new(tmux), Box::new(MockWorktreeOps::new()));
        app.selection = Selection::Feature(0, 0);
        app.open_split_view();
        assert!(matches!(app.mode, AppMode::SplitView(_)));
        app
    }

    fn live() -> MockTmuxOps {
        let mut tmux = MockTmuxOps::new();
        tmux.expect_session_exists().returning(|_| true);
        tmux.expect_window_exists().returning(|_, _| true);
        tmux
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    fn focus(app: &App) -> usize {
        match &app.mode {
            AppMode::SplitView(state) => state.focus,
            _ => usize::MAX,
        }
    }

    fn lay_out(app: &mut App) {
        if let AppMode::SplitView(state) = &mut app.mode {
            let n = state.visible().len();
            state.rects = split::tile_rects(Rect::new(0, 0, 200, 54), n);
        }
    }

    #[test]
    fn typed_text_goes_to_the_focused_tile_only() {
        let mut tmux = live();
        tmux.expect_send_literal()
            .withf(|session, window, text| {
                session == "aiw-hx-12" && window == "codex" && text == "hi"
            })
            .times(1)
            .returning(|_, _, _| Ok(()));
        tmux.expect_send_key_name()
            .withf(|session, window, name| {
                session == "aiw-hx-12" && window == "codex" && name == "Enter"
            })
            .times(1)
            .returning(|_, _, _| Ok(()));
        let mut app = app_with(tmux, 2);
        lay_out(&mut app);
        // Move focus to the right tile (codex), then type.
        handle_split_key(&mut app, ctrl(' ')).unwrap();
        assert!(app.leader_active);
        handle_split_key(&mut app, key(KeyCode::Right)).unwrap();
        assert!(!app.leader_active);
        assert_eq!(focus(&app), 1);
        handle_split_key(&mut app, key(KeyCode::Char('h'))).unwrap();
        handle_split_key(&mut app, key(KeyCode::Char('i'))).unwrap();
        handle_split_key(&mut app, key(KeyCode::Enter)).unwrap();
    }

    #[test]
    fn leader_arrows_move_focus_in_a_2x2() {
        let mut app = app_with(live(), 4);
        lay_out(&mut app);
        for (dir, expected) in [
            (KeyCode::Down, 2),
            (KeyCode::Right, 3),
            (KeyCode::Up, 1),
            (KeyCode::Left, 0),
            (KeyCode::Left, 0),
        ] {
            handle_split_key(&mut app, ctrl(' ')).unwrap();
            handle_split_key(&mut app, key(dir)).unwrap();
            assert_eq!(focus(&app), expected, "{dir:?}");
        }
        if let AppMode::SplitView(state) = &mut app.mode {
            state.move_focus(Direction::Right);
            assert_eq!(state.focus, 1);
        }
    }

    #[test]
    fn ctrl_q_and_leader_v_leave_split_view() {
        let mut app = app_with(live(), 2);
        handle_split_key(&mut app, ctrl('q')).unwrap();
        assert!(matches!(app.mode, AppMode::Normal));

        let mut app = app_with(live(), 2);
        handle_split_key(&mut app, ctrl(' ')).unwrap();
        handle_split_key(&mut app, key(KeyCode::Char('v'))).unwrap();
        assert!(matches!(app.mode, AppMode::Normal));
    }

    #[test]
    fn leader_z_zooms_and_z_again_in_the_view_unzooms() {
        let mut app = app_with(live(), 2);
        handle_split_key(&mut app, ctrl(' ')).unwrap();
        handle_split_key(&mut app, key(KeyCode::Char('z'))).unwrap();
        assert!(matches!(app.mode, AppMode::Viewing(_)));
        crate::handlers::handle_view_key(&mut app, ctrl(' '), 20).unwrap();
        crate::handlers::handle_view_key(&mut app, key(KeyCode::Char('z')), 20).unwrap();
        assert!(matches!(app.mode, AppMode::SplitView(_)));
    }

    #[test]
    fn leader_brackets_turn_pages() {
        let mut app = app_with(live(), 6);
        handle_split_key(&mut app, ctrl(' ')).unwrap();
        handle_split_key(&mut app, key(KeyCode::Char(']'))).unwrap();
        assert_eq!(focus(&app), 4);
        handle_split_key(&mut app, ctrl(' ')).unwrap();
        handle_split_key(&mut app, key(KeyCode::Char('['))).unwrap();
        assert_eq!(focus(&app), 0);
    }

    #[test]
    fn clicking_a_tile_focuses_it() {
        let mut app = app_with(live(), 3);
        lay_out(&mut app);
        let click = |column, row| MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        };
        handle_split_mouse(&mut app, click(150, 40)).unwrap();
        assert_eq!(focus(&app), 2);
        handle_split_mouse(&mut app, click(10, 10)).unwrap();
        assert_eq!(focus(&app), 0);
        handle_split_mouse(&mut app, click(500, 500)).unwrap();
        assert_eq!(focus(&app), 0);
    }

    #[test]
    fn double_ctrl_space_sends_ctrl_space_to_the_agent() {
        let mut tmux = live();
        tmux.expect_send_key_name()
            .withf(|_, window, name| window == "claude" && name == "C-Space")
            .times(1)
            .returning(|_, _, _| Ok(()));
        let mut app = app_with(tmux, 2);
        handle_split_key(&mut app, ctrl(' ')).unwrap();
        handle_split_key(&mut app, ctrl(' ')).unwrap();
        assert!(!app.leader_active);
    }

    #[test]
    fn paste_goes_to_the_focused_tile() {
        let mut tmux = live();
        tmux.expect_paste_text()
            .withf(|session, window, text| {
                session == "aiw-hx-12" && window == "claude" && text == "pasted"
            })
            .times(1)
            .returning(|_, _, _| Ok(()));
        let mut app = app_with(tmux, 2);
        crate::handlers::handle_paste(&mut app, "pasted").unwrap();
    }
}
