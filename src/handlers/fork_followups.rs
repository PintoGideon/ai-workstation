//! FORK: keys for queued follow-ups (P7, DESIGN.md §16).
//!
//! `e` on a session row ("enqueue"; `Q` is the quick plan interview) and
//! `Ctrl+Space Q` in the pane or a view open the dialog. In the dialog the
//! editor has the keys until `Tab` moves them to the queue list: `Enter`
//! queues the text (`Shift+Enter` is a newline), `j`/`k` pick an item,
//! `J`/`K` move it, `d` drops it, `Enter` takes it back into the editor,
//! `Esc` closes.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::fork_followups::FollowupFocus;
use crate::app::{App, AppMode};

/// The dashboard key that opens the dialog on a session row.
pub const QUEUE_KEY: char = 'e';

/// A dashboard key. Returns `true` when consumed.
pub fn handle_dashboard_key(app: &mut App, key: KeyEvent) -> Result<bool> {
    if !matches!(app.mode, AppMode::Normal)
        || app.leader_active
        || key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        || key.code != KeyCode::Char(QUEUE_KEY)
    {
        return Ok(false);
    }
    // A user who bound `e` to a dashboard action keeps it.
    if app
        .active_extension
        .keybindings
        .values()
        .any(|&bound| bound == QUEUE_KEY)
    {
        return Ok(false);
    }
    app.open_followup_queue_for_selection();
    Ok(true)
}

/// Keys in the dialog.
pub fn handle_dialog_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let (session_id, focus, selected) = match &app.mode {
        AppMode::FollowupQueue(state) => (state.session_id.clone(), state.focus, state.selected),
        _ => return Ok(()),
    };
    let len = app.queued_followup_count(&session_id);
    match key.code {
        KeyCode::Esc => {
            app.close_followup_queue();
            return Ok(());
        }
        KeyCode::Tab | KeyCode::BackTab => {
            if let AppMode::FollowupQueue(state) = &mut app.mode {
                state.focus = match state.focus {
                    FollowupFocus::Editor if len > 0 => FollowupFocus::List,
                    _ => FollowupFocus::Editor,
                };
            }
            return Ok(());
        }
        _ => {}
    }
    match focus {
        FollowupFocus::Editor => {
            let AppMode::FollowupQueue(state) = &mut app.mode else {
                return Ok(());
            };
            match key.code {
                KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                    state.editor.insert_str("\n");
                    state.sync_scroll_to_cursor = true;
                }
                KeyCode::Enter => {
                    let text = state.editor.text().trim().to_string();
                    if text.is_empty() {
                        return Ok(());
                    }
                    match app.queue_followup(&session_id, &text) {
                        Ok(count) => {
                            if let AppMode::FollowupQueue(state) = &mut app.mode {
                                state.editor = crate::editor::TextEditor::new(String::new());
                                state.scroll_offset = 0;
                                state.sync_scroll_to_cursor = true;
                                state.selected = count.saturating_sub(1);
                            }
                            app.push_toast_success(format!(
                                "Queued #{count} — delivered when the agent finishes its turn"
                            ));
                        }
                        Err(err) => app.push_toast_warning(err.to_string()),
                    }
                }
                _ => {
                    let outcome = state.editor.handle_key(key);
                    if outcome.text_changed || outcome.cursor_moved {
                        state.sync_scroll_to_cursor = true;
                    }
                }
            }
        }
        FollowupFocus::List => {
            if len == 0 {
                if let AppMode::FollowupQueue(state) = &mut app.mode {
                    state.focus = FollowupFocus::Editor;
                }
                return Ok(());
            }
            let selected = selected.min(len - 1);
            let set_selected = |app: &mut App, value: usize| {
                if let AppMode::FollowupQueue(state) = &mut app.mode {
                    state.selected = value;
                }
            };
            match key.code {
                KeyCode::Down | KeyCode::Char('j') => set_selected(app, (selected + 1) % len),
                KeyCode::Up | KeyCode::Char('k') => {
                    set_selected(app, if selected == 0 { len - 1 } else { selected - 1 })
                }
                KeyCode::Char('J') => {
                    let moved = app.move_followup(&session_id, selected, 1)?;
                    set_selected(app, moved);
                }
                KeyCode::Char('K') => {
                    let moved = app.move_followup(&session_id, selected, -1)?;
                    set_selected(app, moved);
                }
                KeyCode::Char('d') | KeyCode::Delete => {
                    app.delete_followup(&session_id, selected)?;
                    let left = app.queued_followup_count(&session_id);
                    if let AppMode::FollowupQueue(state) = &mut app.mode {
                        state.selected = selected.min(left.saturating_sub(1));
                        if left == 0 {
                            state.focus = FollowupFocus::Editor;
                        }
                    }
                }
                KeyCode::Enter => {
                    // Take it back into the editor to change it.
                    let text = app
                        .followups_for(&session_id)
                        .get(selected)
                        .map(|f| f.text.clone());
                    if let Some(text) = text {
                        app.delete_followup(&session_id, selected)?;
                        let left = app.queued_followup_count(&session_id);
                        if let AppMode::FollowupQueue(state) = &mut app.mode {
                            state.editor = crate::editor::TextEditor::new(text);
                            state.sync_scroll_to_cursor = true;
                            state.selected = selected.min(left.saturating_sub(1));
                            state.focus = FollowupFocus::Editor;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Selection;
    use crate::project::SessionKind;

    fn app() -> App {
        let mut app = crate::app::fork_workspace::workspace_app(&[SessionKind::Claude]);
        app.selection = Selection::Session(0, 0, 0);
        app
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            handle_dialog_key(app, key(KeyCode::Char(c))).unwrap();
        }
    }

    #[test]
    fn e_opens_the_dialog_and_enter_queues_in_order() {
        let mut app = app();
        let id = app.store.projects[0].features[0].sessions[0].id.clone();
        assert!(handle_dashboard_key(&mut app, key(KeyCode::Char('e'))).unwrap());
        assert!(matches!(app.mode, AppMode::FollowupQueue(_)));
        type_text(&mut app, "first");
        handle_dialog_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT)).unwrap();
        type_text(&mut app, "line two");
        handle_dialog_key(&mut app, key(KeyCode::Enter)).unwrap();
        type_text(&mut app, "second");
        handle_dialog_key(&mut app, key(KeyCode::Enter)).unwrap();
        let texts: Vec<&str> = app
            .followups_for(&id)
            .iter()
            .map(|f| f.text.as_str())
            .collect();
        assert_eq!(texts, ["first\nline two", "second"]);
        // Empty text queues nothing.
        handle_dialog_key(&mut app, key(KeyCode::Enter)).unwrap();
        assert_eq!(app.queued_followup_count(&id), 2);
        handle_dialog_key(&mut app, key(KeyCode::Esc)).unwrap();
        assert!(matches!(app.mode, AppMode::Normal));
    }

    #[test]
    fn the_list_reorders_drops_and_edits() {
        let mut app = app();
        let id = app.store.projects[0].features[0].sessions[0].id.clone();
        app.queue_followup(&id, "a").unwrap();
        app.queue_followup(&id, "b").unwrap();
        app.queue_followup(&id, "c").unwrap();
        app.open_followup_queue_for_selection();
        handle_dialog_key(&mut app, key(KeyCode::Tab)).unwrap();
        let AppMode::FollowupQueue(state) = &app.mode else {
            panic!()
        };
        assert_eq!(state.focus, FollowupFocus::List);
        assert_eq!(state.selected, 0);
        handle_dialog_key(&mut app, key(KeyCode::Char('j'))).unwrap();
        handle_dialog_key(&mut app, key(KeyCode::Char('j'))).unwrap();
        handle_dialog_key(&mut app, key(KeyCode::Char('K'))).unwrap();
        let texts: Vec<&str> = app
            .followups_for(&id)
            .iter()
            .map(|f| f.text.as_str())
            .collect();
        assert_eq!(texts, ["a", "c", "b"]);
        handle_dialog_key(&mut app, key(KeyCode::Char('k'))).unwrap();
        handle_dialog_key(&mut app, key(KeyCode::Char('d'))).unwrap();
        let texts: Vec<&str> = app
            .followups_for(&id)
            .iter()
            .map(|f| f.text.as_str())
            .collect();
        assert_eq!(texts, ["c", "b"]);
        handle_dialog_key(&mut app, key(KeyCode::Enter)).unwrap();
        let AppMode::FollowupQueue(state) = &app.mode else {
            panic!()
        };
        assert_eq!(state.focus, FollowupFocus::Editor);
        assert_eq!(state.editor.text(), "c");
        assert_eq!(app.queued_followup_count(&id), 1);
    }

    #[test]
    fn e_does_nothing_when_the_user_bound_it() {
        let mut app = app();
        app.active_extension
            .keybindings
            .insert("mark_ready".into(), 'e');
        assert!(!handle_dashboard_key(&mut app, key(KeyCode::Char('e'))).unwrap());
    }
}
