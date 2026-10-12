//! FORK: keys for the `@` message dialog (DESIGN.md §19.5).

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{App, AppMode};

pub fn handle_message_dialog_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    match key.code {
        KeyCode::Esc => app.mode = AppMode::Normal,
        KeyCode::Enter if alt || shift => push(app, '\n'),
        KeyCode::Char('j') | KeyCode::Char('J') if ctrl => push(app, '\n'),
        KeyCode::Enter => app.submit_message_dialog(),
        KeyCode::Up | KeyCode::BackTab => move_target(app, -1),
        KeyCode::Down | KeyCode::Tab => move_target(app, 1),
        KeyCode::Char('p') | KeyCode::Char('P') if ctrl => move_target(app, -1),
        KeyCode::Char('n') | KeyCode::Char('N') if ctrl => move_target(app, 1),
        KeyCode::Char('u') | KeyCode::Char('U') if ctrl => {
            if let AppMode::MessageSession(state) = &mut app.mode {
                state.text.clear();
            }
        }
        KeyCode::Backspace => {
            if let AppMode::MessageSession(state) = &mut app.mode {
                state.text.pop();
            }
        }
        KeyCode::Char(c) if !ctrl => push(app, c),
        _ => {}
    }
    Ok(())
}

fn push(app: &mut App, c: char) {
    if let AppMode::MessageSession(state) = &mut app.mode {
        state.text.push(c);
    }
}

fn move_target(app: &mut App, delta: i32) {
    if let AppMode::MessageSession(state) = &mut app.mode {
        state.move_target(delta);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::fork_messaging::{MessageDialogState, MessageTarget};
    use crate::traits::{MockTmuxOps, MockWorktreeOps};

    fn app_with_dialog() -> App {
        let mut app = App::new_for_test(
            crate::project::ProjectStore::empty(),
            Box::new(MockTmuxOps::new()),
            Box::new(MockWorktreeOps::new()),
        );
        let target = |id: &str| MessageTarget {
            session_id: id.into(),
            title: id.into(),
            route: "inbox".into(),
        };
        app.mode = AppMode::MessageSession(MessageDialogState {
            targets: vec![target("a"), target("b"), target("c")],
            target_index: 0,
            text: String::new(),
        });
        app
    }

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn state(app: &App) -> &MessageDialogState {
        match &app.mode {
            AppMode::MessageSession(state) => state,
            _ => panic!("dialog closed"),
        }
    }

    #[test]
    fn typing_editing_and_newlines() {
        let mut app = app_with_dialog();
        for c in "hi j".chars() {
            handle_message_dialog_key(&mut app, k(KeyCode::Char(c))).unwrap();
        }
        handle_message_dialog_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT))
            .unwrap();
        handle_message_dialog_key(&mut app, k(KeyCode::Char('x'))).unwrap();
        assert_eq!(state(&app).text, "hi j\nx");
        handle_message_dialog_key(&mut app, k(KeyCode::Backspace)).unwrap();
        assert_eq!(state(&app).text, "hi j\n");
        handle_message_dialog_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        )
        .unwrap();
        assert_eq!(state(&app).text, "");
    }

    #[test]
    fn arrows_tab_and_ctrl_n_p_move_the_target_and_esc_closes() {
        let mut app = app_with_dialog();
        handle_message_dialog_key(&mut app, k(KeyCode::Down)).unwrap();
        handle_message_dialog_key(&mut app, k(KeyCode::Tab)).unwrap();
        assert_eq!(state(&app).target_index, 2);
        handle_message_dialog_key(&mut app, k(KeyCode::Down)).unwrap();
        assert_eq!(state(&app).target_index, 0, "wraps");
        handle_message_dialog_key(&mut app, k(KeyCode::Up)).unwrap();
        assert_eq!(state(&app).target_index, 2);
        handle_message_dialog_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('p'), KeyModifiers::CONTROL),
        )
        .unwrap();
        assert_eq!(state(&app).target_index, 1);
        // j and k are text, not navigation.
        handle_message_dialog_key(&mut app, k(KeyCode::Char('j'))).unwrap();
        assert_eq!(state(&app).target_index, 1);
        assert_eq!(state(&app).text, "j");
        handle_message_dialog_key(&mut app, k(KeyCode::Esc)).unwrap();
        assert!(matches!(app.mode, AppMode::Normal));
    }
}
