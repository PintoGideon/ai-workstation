//! FORK: keys for the `f` fork dialog and the `E` external-sessions list
//! (DESIGN.md §19.2).

use anyhow::Result;
use crossterm::event::KeyCode;

use crate::app::{App, AppMode};

pub fn handle_fork_session_key(app: &mut App, key: KeyCode) -> Result<()> {
    match key {
        KeyCode::Esc | KeyCode::Char('q') => app.mode = AppMode::Normal,
        KeyCode::Up | KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('k') | KeyCode::Tab => {
            if let AppMode::ForkSession(state) = &mut app.mode {
                state.choice = 1 - state.choice.min(1);
            }
        }
        KeyCode::Char('1') => {
            if let AppMode::ForkSession(state) = &mut app.mode {
                state.choice = 0;
            }
            // A failed start was already reported as a toast.
            let _ = app.confirm_fork_session();
        }
        KeyCode::Char('2') => {
            if let AppMode::ForkSession(state) = &mut app.mode {
                state.choice = 1;
            }
            let _ = app.confirm_fork_session();
        }
        KeyCode::Enter => {
            let _ = app.confirm_fork_session();
        }
        _ => {}
    }
    Ok(())
}

pub fn handle_external_sessions_key(app: &mut App, key: KeyCode) -> Result<()> {
    let count = app.session_identity.externals.len();
    match key {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('E') => app.mode = AppMode::Normal,
        KeyCode::Down | KeyCode::Char('j') => {
            if let AppMode::ExternalSessions(state) = &mut app.mode
                && state.selected + 1 < count
            {
                state.selected += 1;
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if let AppMode::ExternalSessions(state) = &mut app.mode {
                state.selected = state.selected.saturating_sub(1);
            }
        }
        KeyCode::Enter | KeyCode::Char('y') => app.copy_external_attach_command(),
        _ => {}
    }
    // The roster can shrink under an open list.
    if let AppMode::ExternalSessions(state) = &mut app.mode {
        if count == 0 {
            app.mode = AppMode::Normal;
        } else if state.selected >= count {
            state.selected = count - 1;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::fork_identity::{ExternalSessionsState, ForkSessionState};
    use crate::fork::claude_roster::RosterEntry;
    use crate::fork::session_identity::ForkSource;
    use crate::project::{AgentKind, SessionKind};

    fn source() -> ForkSource {
        ForkSource {
            agent: AgentKind::Claude,
            harness_session_id: "conv-1".into(),
            source_label: "Claude 1".into(),
        }
    }

    #[test]
    fn the_fork_dialog_toggles_and_cancels() {
        let mut app = crate::app::fork_workspace::workspace_app(&[SessionKind::Claude]);
        app.mode = AppMode::ForkSession(ForkSessionState {
            pi: 0,
            fi: 0,
            source: source(),
            choice: 0,
        });
        handle_fork_session_key(&mut app, KeyCode::Char('j')).unwrap();
        assert!(matches!(&app.mode, AppMode::ForkSession(s) if s.choice == 1));
        handle_fork_session_key(&mut app, KeyCode::Tab).unwrap();
        assert!(matches!(&app.mode, AppMode::ForkSession(s) if s.choice == 0));
        handle_fork_session_key(&mut app, KeyCode::Esc).unwrap();
        assert!(matches!(app.mode, AppMode::Normal));
    }

    #[test]
    fn choosing_a_new_worktree_opens_the_feature_fork_with_the_conversation() {
        let mut app = crate::app::fork_workspace::workspace_app(&[SessionKind::Claude]);
        app.mode = AppMode::ForkSession(ForkSessionState {
            pi: 0,
            fi: 0,
            source: source(),
            choice: 0,
        });
        handle_fork_session_key(&mut app, KeyCode::Char('2')).unwrap();
        let AppMode::ForkingFeature(fork) = &app.mode else {
            panic!("expected the feature fork dialog");
        };
        assert_eq!(fork.fork_conversation.as_ref(), Some(&source()));
        assert_eq!(fork.agent, AgentKind::Claude);
        assert!(!fork.include_context);
    }

    #[test]
    fn the_external_list_moves_and_closes_when_the_roster_empties() {
        let mut app = crate::app::fork_workspace::workspace_app(&[SessionKind::Claude]);
        let entry = |id: &str| RosterEntry {
            cwd: "/tmp/x".into(),
            kind: "background".into(),
            id: Some(id.into()),
            ..Default::default()
        };
        app.apply_claude_roster(vec![entry("a"), entry("b")]);
        assert_eq!(app.session_identity.externals.len(), 2);
        app.mode = AppMode::ExternalSessions(ExternalSessionsState::default());
        handle_external_sessions_key(&mut app, KeyCode::Char('j')).unwrap();
        handle_external_sessions_key(&mut app, KeyCode::Char('j')).unwrap();
        assert!(matches!(&app.mode, AppMode::ExternalSessions(s) if s.selected == 1));
        app.apply_claude_roster(Vec::new());
        handle_external_sessions_key(&mut app, KeyCode::Char('k')).unwrap();
        assert!(matches!(app.mode, AppMode::Normal));
    }
}
