//! FORK: keys for the land dialog (DESIGN.md §19.10): `Enter` lands (`y`
//! confirms while agents still work), `r` re-runs the preflight, `a` aborts a
//! rebase or merge left in progress, `t` opens a terminal in the worktree,
//! `x` dismisses the last failure, `Esc`/`q` close (a running land goes on).

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};

use crate::app::{App, AppMode};

pub fn handle_land_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let AppMode::Land(state) = &app.mode else {
        return Ok(());
    };
    let landing = app.land_stage(&state.feature_id).is_some();
    let confirming = state.confirm_working;
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') => {
            if confirming {
                if let AppMode::Land(state) = &mut app.mode {
                    state.confirm_working = false;
                    state.note = None;
                }
            } else {
                app.mode = AppMode::Normal;
            }
        }
        KeyCode::Enter if !landing && !confirming => app.confirm_land(),
        KeyCode::Char('y') if !landing && confirming => app.confirm_land(),
        KeyCode::Char('r') if !landing => app.refresh_land_preflight(),
        KeyCode::Char('a') if !landing => app.abort_land_in_progress(),
        KeyCode::Char('x') => app.dismiss_land_failure(),
        KeyCode::Char('t') => app.open_land_shell(),
        _ => {}
    }
    Ok(())
}
