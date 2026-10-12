//! FORK: keys for the schedules list and editor (DESIGN.md §19.9).
//!
//! List: `j`/`k` move, `n` new, `e`/`Enter` edit, `Space`/`p` pause or
//! resume, `r` run now, `d` then `y` delete, `h` history, `Esc`/`q`/`t`
//! close. Editor: `Tab`/`↓` and `Shift+Tab`/`↑` move between fields,
//! `←`/`→`/`Space` change a choice, typing edits a text field (`Alt+Enter`
//! or `Ctrl+J` adds a line to the text), `Ctrl+U` clears it, `Enter` saves,
//! `Esc` goes back.

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::fork_schedules::{EditorField, Trigger};
use crate::app::{App, AppMode};

pub fn handle_schedules_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let AppMode::Schedules(state) = &app.mode else {
        return Ok(());
    };
    let rows = app.schedule_rows(state).len();
    if state.confirm_delete {
        let confirmed = matches!(key.code, KeyCode::Char('y'));
        if let AppMode::Schedules(state) = &mut app.mode {
            state.confirm_delete = false;
        }
        if confirmed && let Some(id) = app.schedule_list_selected_id() {
            match app.delete_schedule(&id) {
                Ok(()) => {
                    app.push_toast_success("Schedule deleted");
                    if let AppMode::Schedules(state) = &mut app.mode {
                        state.selected = state.selected.min(rows.saturating_sub(2));
                    }
                }
                Err(e) => app.push_toast_warning(e.to_string()),
            }
        }
        return Ok(());
    }
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('t') => app.mode = AppMode::Normal,
        KeyCode::Down | KeyCode::Char('j') if rows > 0 => {
            if let AppMode::Schedules(state) = &mut app.mode {
                state.selected = (state.selected + 1) % rows;
            }
        }
        KeyCode::Up | KeyCode::Char('k') if rows > 0 => {
            if let AppMode::Schedules(state) = &mut app.mode {
                state.selected = (state.selected + rows - 1) % rows;
            }
        }
        KeyCode::Char('n') => app.open_schedule_editor(None),
        KeyCode::Char('e') | KeyCode::Enter => {
            if let Some(id) = app.schedule_list_selected_id() {
                app.open_schedule_editor(Some(id));
            }
        }
        KeyCode::Char(' ') | KeyCode::Char('p') => {
            if let Some(id) = app.schedule_list_selected_id() {
                let paused = app.schedule(&id).is_some_and(|s| s.paused);
                match app.set_schedule_paused(&id, !paused) {
                    Ok(true) => app.push_toast_success("Schedule paused"),
                    Ok(false) => app.push_toast_success("Schedule resumed"),
                    Err(e) => app.push_toast_warning(e.to_string()),
                }
            }
        }
        KeyCode::Char('r') => {
            if let Some(id) = app.schedule_list_selected_id() {
                match app.run_schedule_now(&id, Trigger::RunNow) {
                    Ok(outcome) => app.push_toast_success(format!("Ran: {}", outcome.label())),
                    Err(e) => app.push_toast_warning(e.to_string()),
                }
            }
        }
        KeyCode::Char('d') if rows > 0 => {
            if let AppMode::Schedules(state) = &mut app.mode {
                state.confirm_delete = true;
            }
        }
        KeyCode::Char('h') => {
            if let AppMode::Schedules(state) = &mut app.mode {
                state.show_history = !state.show_history;
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn handle_schedule_editor_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let AppMode::ScheduleEditor(state) = &mut app.mode else {
        return Ok(());
    };
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        KeyCode::Esc => {
            app.cancel_schedule_editor();
            return Ok(());
        }
        KeyCode::Enter if alt && state.focus == EditorField::Text => state.text.push('\n'),
        KeyCode::Char('j') if ctrl && state.focus == EditorField::Text => state.text.push('\n'),
        KeyCode::Enter => {
            app.submit_schedule_editor();
            return Ok(());
        }
        KeyCode::Tab | KeyCode::Down => state.next_field(),
        KeyCode::BackTab | KeyCode::Up => state.prev_field(),
        KeyCode::Left => state.cycle(false),
        KeyCode::Right => state.cycle(true),
        KeyCode::Char('u') if ctrl => {
            if let Some(text) = state.field_text_mut() {
                text.clear();
            }
        }
        KeyCode::Backspace => {
            if let Some(text) = state.field_text_mut() {
                text.pop();
            }
        }
        KeyCode::Char(' ')
            if matches!(
                state.focus,
                EditorField::Kind | EditorField::StartIfStopped | EditorField::CatchUp
            ) || (state.focus == EditorField::Target
                && state.kind == crate::fork::schedules::ScheduleKind::Prompt) =>
        {
            state.cycle(true)
        }
        KeyCode::Char(c) if !ctrl => {
            if let Some(text) = state.field_text_mut() {
                text.push(c);
            }
        }
        _ => {}
    }
    if let AppMode::ScheduleEditor(state) = &mut app.mode {
        state.error = None;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Selection;
    use crate::project::{ProjectStatus, SessionKind};
    use crate::traits::{MockTmuxOps, MockWorktreeOps};

    fn app() -> App {
        let now = chrono::Utc::now();
        let mut feature = crate::project::Feature {
            id: "feat-1".into(),
            name: "f".into(),
            branch: "f".into(),
            workdir: std::path::PathBuf::from("/tmp/aiw-sched-test-workdir"),
            is_worktree: false,
            tmux_session: "aiw-f".into(),
            sessions: vec![],
            collapsed: false,
            mode: Default::default(),
            agent: Default::default(),
            enable_chrome: false,
            pending_worktree_script: false,
            ready: false,
            status: ProjectStatus::Active,
            created_at: now,
            last_accessed: now,
            summary: None,
            summary_updated_at: None,
            nickname: None,
            issue_source: None,
        };
        feature.add_session_named(SessionKind::Claude, "Claude 1".into());
        let project = crate::project::Project {
            id: "p".into(),
            name: "p".into(),
            repo: std::path::PathBuf::from("/tmp/aiw-sched-test-repo"),
            collapsed: false,
            features: vec![feature],
            created_at: now,
            preferred_agent: Default::default(),
            is_git: true,
        };
        let mut store = crate::project::ProjectStore::empty();
        store.projects.push(project);
        let mut app = App::new_for_test(
            store,
            Box::new(MockTmuxOps::new()),
            Box::new(MockWorktreeOps::new()),
        );
        app.config.aiw.schedule_min_interval_secs = 1;
        app.selection = Selection::Session(0, 0, 0);
        app
    }

    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn type_text(app: &mut App, text: &str) {
        for c in text.chars() {
            handle_schedule_editor_key(app, k(KeyCode::Char(c))).unwrap();
        }
    }

    #[test]
    fn the_editor_types_moves_between_fields_saves_and_goes_back() {
        let mut app = app();
        app.open_schedules_for_selection();
        handle_schedules_key(&mut app, k(KeyCode::Char('n'))).unwrap();
        assert!(matches!(app.mode, AppMode::ScheduleEditor(_)));
        type_text(&mut app, "every 5m");
        handle_schedule_editor_key(&mut app, k(KeyCode::Tab)).unwrap();
        type_text(&mut app, "hello");
        handle_schedule_editor_key(&mut app, KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT))
            .unwrap();
        type_text(&mut app, "x");
        handle_schedule_editor_key(&mut app, k(KeyCode::Backspace)).unwrap();
        type_text(&mut app, "world");
        {
            let AppMode::ScheduleEditor(state) = &app.mode else {
                panic!("editor closed");
            };
            assert_eq!(state.spec, "every 5m");
            assert_eq!(state.text, "hello\nworld");
        }
        // Space on a choice field toggles it.
        handle_schedule_editor_key(&mut app, k(KeyCode::Tab)).unwrap();
        handle_schedule_editor_key(&mut app, k(KeyCode::Char(' '))).unwrap();
        let AppMode::ScheduleEditor(state) = &app.mode else {
            panic!("editor closed");
        };
        assert_eq!(state.focus, EditorField::StartIfStopped);
        assert!(state.start_if_stopped);
        handle_schedule_editor_key(&mut app, k(KeyCode::Enter)).unwrap();
        assert!(matches!(app.mode, AppMode::Schedules(_)));
        assert_eq!(app.schedules.items.len(), 1);
        assert_eq!(app.schedules.items[0].text, "hello\nworld");
        assert!(app.schedules.items[0].start_if_stopped);

        // Esc from a fresh editor goes back to the list without saving.
        handle_schedules_key(&mut app, k(KeyCode::Char('n'))).unwrap();
        handle_schedule_editor_key(&mut app, k(KeyCode::Esc)).unwrap();
        assert!(matches!(app.mode, AppMode::Schedules(_)));
        assert_eq!(app.schedules.items.len(), 1);
    }

    #[test]
    fn the_list_pauses_and_deletes_after_confirming() {
        let mut app = app();
        let mut s = App::new_schedule(&app.store.projects[0].features[0].id.clone());
        s.kind = crate::fork::schedules::ScheduleKind::Shell;
        s.spec = "every 1h".into();
        s.text = "make".into();
        let id = app.save_schedule(s).unwrap();
        app.open_schedules_for_selection();
        handle_schedules_key(&mut app, k(KeyCode::Char(' '))).unwrap();
        assert!(app.schedule(&id).unwrap().paused);
        handle_schedules_key(&mut app, k(KeyCode::Char('h'))).unwrap();
        assert!(matches!(&app.mode, AppMode::Schedules(s) if s.show_history));
        handle_schedules_key(&mut app, k(KeyCode::Char('d'))).unwrap();
        handle_schedules_key(&mut app, k(KeyCode::Char('n'))).unwrap();
        assert!(app.schedule(&id).is_some(), "any key but y keeps it");
        handle_schedules_key(&mut app, k(KeyCode::Char('d'))).unwrap();
        handle_schedules_key(&mut app, k(KeyCode::Char('y'))).unwrap();
        assert!(app.schedule(&id).is_none());
        handle_schedules_key(&mut app, k(KeyCode::Esc)).unwrap();
        assert!(matches!(app.mode, AppMode::Normal));
    }
}
