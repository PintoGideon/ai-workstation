//! FORK: keys for background Claude sessions (DESIGN.md §19.7): the row's
//! own `Enter` / `c` / `x` / `d`, the first-prompt step of the `s` picker,
//! and the `claude rm` confirmation.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{App, AppMode};

/// Keys on a selected background session row. Returns whether the key was
/// taken; anything else is the dashboard's own.
pub fn handle_dashboard_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    if app.leader_active
        || key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return Ok(false);
    }
    let Some((pi, fi, si)) = app.selected_background_session() else {
        return Ok(false);
    };
    match key.code {
        KeyCode::Enter => {
            if let Err(e) = app.attach_background_session(pi, fi, si) {
                app.push_toast_error(format!("Could not attach: {e}"));
            }
        }
        KeyCode::Char('c') => app.restart_background_session(pi, fi, si),
        KeyCode::Char('x') => app.stop_background_session(pi, fi, si),
        KeyCode::Char('d') => app.open_background_remove(pi, fi, si),
        _ => return Ok(false),
    }
    Ok(true)
}

/// The first prompt of a background session: `Enter` starts it,
/// `Alt+Enter` / `Ctrl+J` add a line, `Ctrl+U` clears, `Esc` cancels.
pub fn handle_background_prompt_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let AppMode::BackgroundPrompt(state) = &mut app.mode else {
        return Ok(());
    };
    match key.code {
        KeyCode::Esc => {
            let label = state.label.clone();
            app.mode = AppMode::Normal;
            app.message = Some(format!("'{label}' was not added"));
        }
        KeyCode::Enter if alt => state.text.push('\n'),
        KeyCode::Char('j') | KeyCode::Char('J') if ctrl => state.text.push('\n'),
        KeyCode::Enter => app.submit_background_prompt(),
        KeyCode::Backspace => {
            state.text.pop();
        }
        KeyCode::Char('u') | KeyCode::Char('U') if ctrl => state.text.clear(),
        KeyCode::Char(c) if !ctrl => state.text.push(c),
        _ => {}
    }
    Ok(())
}

/// The `claude rm` confirmation: `y` / `Enter` removes (the second time
/// with the flags Claude's refusal printed), `n` / `Esc` keeps it.
pub fn handle_background_remove_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let running = matches!(&app.mode, AppMode::BackgroundRemove(state) if state.running);
    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter if !running => {
            app.confirm_background_remove()?;
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Char('q') => {
            app.mode = AppMode::Normal;
        }
        _ => {}
    }
    Ok(())
}
