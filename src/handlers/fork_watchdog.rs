//! FORK: keys for the watchdog's recovery dialog (DESIGN.md §19.8):
//! `j`/`k` (or arrows) move, `Enter` runs the selected action, `1`–`9` run
//! that action directly, `Esc`/`q` close.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};

use crate::app::{App, AppMode};

pub fn handle_watchdog_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let AppMode::Watchdog(state) = &mut app.mode else {
        return Ok(());
    };
    let len = state.actions.len();
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => app.mode = AppMode::Normal,
        KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab if len > 0 => {
            state.selected = (state.selected + 1) % len;
        }
        KeyCode::Up | KeyCode::Char('k') | KeyCode::BackTab if len > 0 => {
            state.selected = (state.selected + len - 1) % len;
        }
        KeyCode::Enter => app.confirm_watchdog_dialog(),
        KeyCode::Char(c @ '1'..='9') => {
            let index = c as usize - '1' as usize;
            if index < len {
                state.selected = index;
                app.confirm_watchdog_dialog();
            }
        }
        _ => {}
    }
    Ok(())
}
