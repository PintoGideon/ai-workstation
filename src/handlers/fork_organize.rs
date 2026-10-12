//! FORK: keys for the `G` groups dialog and the `#` note editor
//! (DESIGN.md §19.4).

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::fork_organize::GroupEdit;
use crate::app::{App, AppMode};

pub fn handle_feature_groups_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let (editing, confirming) = match &app.mode {
        AppMode::FeatureGroups(state) => (state.edit.is_some(), state.confirm_delete.is_some()),
        _ => return Ok(()),
    };

    // The name field, while open, takes every key.
    if editing {
        match key.code {
            KeyCode::Esc => {
                if let AppMode::FeatureGroups(state) = &mut app.mode {
                    state.edit = None;
                    state.error = None;
                }
            }
            KeyCode::Enter => app.group_dialog_submit_edit(),
            KeyCode::Backspace => {
                if let AppMode::FeatureGroups(state) = &mut app.mode
                    && let Some((_, text)) = state.edit.as_mut()
                {
                    text.pop();
                    state.error = None;
                }
            }
            KeyCode::Char('u') | KeyCode::Char('U') if ctrl => {
                if let AppMode::FeatureGroups(state) = &mut app.mode
                    && let Some((_, text)) = state.edit.as_mut()
                {
                    text.clear();
                }
            }
            KeyCode::Char(c) if !ctrl => {
                if let AppMode::FeatureGroups(state) = &mut app.mode
                    && let Some((_, text)) = state.edit.as_mut()
                {
                    text.push(c);
                    state.error = None;
                }
            }
            _ => {}
        }
        return Ok(());
    }

    if confirming {
        let id = match &mut app.mode {
            AppMode::FeatureGroups(state) => state.confirm_delete.take(),
            _ => None,
        };
        if let (Some(id), KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter) = (id, key.code)
        {
            app.delete_feature_group(&id);
            clamp_selection(app);
        }
        return Ok(());
    }

    let rows = match &app.mode {
        AppMode::FeatureGroups(state) => app.group_dialog_rows(state).len(),
        _ => 0,
    };
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('G') => app.mode = AppMode::Normal,
        KeyCode::Down | KeyCode::Char('j') => {
            if let AppMode::FeatureGroups(state) = &mut app.mode {
                state.selected = (state.selected + 1) % rows.max(1);
                state.error = None;
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if let AppMode::FeatureGroups(state) = &mut app.mode {
                state.selected = state
                    .selected
                    .checked_sub(1)
                    .unwrap_or(rows.saturating_sub(1));
                state.error = None;
            }
        }
        KeyCode::Enter => app.group_dialog_choose(),
        KeyCode::Char('n') => {
            if let AppMode::FeatureGroups(state) = &mut app.mode {
                state.edit = Some((GroupEdit::New, String::new()));
                state.error = None;
            }
        }
        KeyCode::Char('r')
        | KeyCode::Char('c')
        | KeyCode::Char('d')
        | KeyCode::Char('J')
        | KeyCode::Char('K') => {
            let Some(group) = app.group_dialog_selected_group() else {
                if let AppMode::FeatureGroups(state) = &mut app.mode {
                    state.error = Some("Select a group first".into());
                }
                return Ok(());
            };
            match key.code {
                KeyCode::Char('r') => {
                    if let AppMode::FeatureGroups(state) = &mut app.mode {
                        state.edit =
                            Some((GroupEdit::Rename(group.id.clone()), group.name.clone()));
                        state.error = None;
                    }
                }
                KeyCode::Char('c') => app.recolor_feature_group(&group.id),
                KeyCode::Char('d') => {
                    if let AppMode::FeatureGroups(state) = &mut app.mode {
                        state.confirm_delete = Some(group.id.clone());
                    }
                }
                KeyCode::Char('J') | KeyCode::Char('K') => {
                    let delta = if key.code == KeyCode::Char('J') {
                        1
                    } else {
                        -1
                    };
                    app.move_feature_group(&group.id, delta);
                    // Keep the moved group selected.
                    let index = match &app.mode {
                        AppMode::FeatureGroups(state) => {
                            app.groups_dialog_project(state).and_then(|pi| {
                                app.project_groups(pi).iter().position(|g| g.id == group.id)
                            })
                        }
                        _ => None,
                    };
                    if let (Some(index), AppMode::FeatureGroups(state)) = (index, &mut app.mode) {
                        state.selected = index + 1;
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
    Ok(())
}

/// Keep the selection on a row after a group disappears.
fn clamp_selection(app: &mut App) {
    let rows = match &app.mode {
        AppMode::FeatureGroups(state) => app.group_dialog_rows(state).len(),
        _ => return,
    };
    if let AppMode::FeatureGroups(state) = &mut app.mode
        && state.selected >= rows
    {
        state.selected = rows.saturating_sub(1);
    }
}

pub fn handle_note_editor_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    match key.code {
        KeyCode::Esc => {
            app.mode = AppMode::Normal;
        }
        KeyCode::Enter if alt || shift => push(app, '\n'),
        KeyCode::Char('j') | KeyCode::Char('J') if ctrl => push(app, '\n'),
        KeyCode::Enter => app.save_note_editor(),
        KeyCode::Char('u') | KeyCode::Char('U') if ctrl => {
            if let AppMode::NoteEditor(state) = &mut app.mode {
                state.text.clear();
            }
        }
        KeyCode::Backspace => {
            if let AppMode::NoteEditor(state) = &mut app.mode {
                state.text.pop();
            }
        }
        KeyCode::Char(c) if !ctrl => push(app, c),
        _ => {}
    }
    Ok(())
}

fn push(app: &mut App, c: char) {
    if let AppMode::NoteEditor(state) = &mut app.mode {
        state.text.push(c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::fork_organize::NoteEditorState;
    use crate::fork::organize::NoteTarget;
    use crate::traits::{MockTmuxOps, MockWorktreeOps};

    fn app() -> App {
        App::new_for_test(
            crate::project::ProjectStore::empty(),
            Box::new(MockTmuxOps::new()),
            Box::new(MockWorktreeOps::new()),
        )
    }

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn the_note_editor_types_breaks_lines_clears_and_cancels() {
        let mut app = app();
        app.mode = AppMode::NoteEditor(NoteEditorState {
            target: NoteTarget::Feature("f".into()),
            title: "f".into(),
            text: String::new(),
            original: String::new(),
        });
        for c in "ab".chars() {
            handle_note_editor_key(&mut app, k(KeyCode::Char(c))).unwrap();
        }
        handle_note_editor_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT)).unwrap();
        handle_note_editor_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL),
        )
        .unwrap();
        handle_note_editor_key(&mut app, k(KeyCode::Char('c'))).unwrap();
        handle_note_editor_key(&mut app, k(KeyCode::Backspace)).unwrap();
        handle_note_editor_key(&mut app, k(KeyCode::Char('d'))).unwrap();
        let AppMode::NoteEditor(state) = &app.mode else {
            panic!("editor closed");
        };
        assert_eq!(state.text, "ab\n\nd");
        handle_note_editor_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
        )
        .unwrap();
        let AppMode::NoteEditor(state) = &app.mode else {
            panic!("editor closed");
        };
        assert!(state.text.is_empty());
        handle_note_editor_key(&mut app, k(KeyCode::Esc)).unwrap();
        assert!(matches!(app.mode, AppMode::Normal));
    }
}
