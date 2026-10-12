//! FORK: keys for the `W` list of worktrees Claude created (DESIGN.md §19.6).

use anyhow::Result;
use crossterm::event::KeyCode;

use crate::app::{App, AppMode};

pub fn handle_claude_worktrees_key(app: &mut App, key: KeyCode) -> Result<()> {
    let confirming = matches!(&app.mode, AppMode::ClaudeWorktrees(s) if s.confirm_remove);
    if confirming {
        match key {
            KeyCode::Char('y') | KeyCode::Char('Y') => app.remove_selected_claude_worktree(),
            _ => {
                if let AppMode::ClaudeWorktrees(state) = &mut app.mode {
                    state.confirm_remove = false;
                }
            }
        }
        return Ok(());
    }
    match key {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('W') => app.mode = AppMode::Normal,
        KeyCode::Down | KeyCode::Char('j') => {
            if let AppMode::ClaudeWorktrees(state) = &mut app.mode
                && state.selected + 1 < state.ids.len()
            {
                state.selected += 1;
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if let AppMode::ClaudeWorktrees(state) = &mut app.mode {
                state.selected = state.selected.saturating_sub(1);
            }
        }
        KeyCode::Enter | KeyCode::Char('t') => app.open_claude_worktree_terminal()?,
        KeyCode::Char('y') => app.copy_claude_worktree_path(),
        KeyCode::Char('d') | KeyCode::Delete => {
            if let AppMode::ClaudeWorktrees(state) = &mut app.mode {
                state.confirm_remove = true;
            }
        }
        _ => {}
    }
    // A hook can remove a worktree under the open list.
    app.refresh_claude_worktrees_dialog();
    Ok(())
}
