use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{
    App, AppMode, CreateProjectStep, StoppedSessionChoice, ThemePickerEntry, ThemePickerGroupState,
};

pub fn handle_create_project_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('b') {
        let is_path_step = matches!(
            &app.mode,
            AppMode::CreatingProject(s)
                if matches!(s.step, CreateProjectStep::Path)
        );
        if is_path_step {
            let browse = std::mem::replace(&mut app.mode, AppMode::Normal);
            if let AppMode::CreatingProject(state) = browse {
                app.start_browse_path(state);
            }
            return Ok(());
        }
    }

    let is_agent_step = matches!(
        &app.mode,
        AppMode::CreatingProject(state)
            if matches!(state.step, CreateProjectStep::Agent)
    );

    match key.code {
        KeyCode::Esc => {
            app.cancel_create();
        }
        KeyCode::Enter => {
            let step = match &app.mode {
                AppMode::CreatingProject(state) => state.step.clone(),
                _ => return Ok(()),
            };

            match step {
                CreateProjectStep::Name => {
                    if let AppMode::CreatingProject(state) = &mut app.mode {
                        state.step = CreateProjectStep::Path;
                    }
                }
                CreateProjectStep::Path => {
                    if let AppMode::CreatingProject(state) = &mut app.mode {
                        state.step = CreateProjectStep::Agent;
                    }
                }
                CreateProjectStep::Agent => {
                    app.create_project()?;
                }
            }
        }
        KeyCode::Tab => {
            if let AppMode::CreatingProject(state) = &mut app.mode {
                state.step = match state.step {
                    CreateProjectStep::Name => CreateProjectStep::Path,
                    CreateProjectStep::Path => CreateProjectStep::Agent,
                    CreateProjectStep::Agent => CreateProjectStep::Name,
                };
            }
        }
        KeyCode::BackTab => {
            if let AppMode::CreatingProject(state) = &mut app.mode {
                state.step = match state.step {
                    CreateProjectStep::Name => CreateProjectStep::Agent,
                    CreateProjectStep::Path => CreateProjectStep::Name,
                    CreateProjectStep::Agent => CreateProjectStep::Path,
                };
            }
        }
        KeyCode::Backspace => {
            if let AppMode::CreatingProject(state) = &mut app.mode {
                match state.step {
                    CreateProjectStep::Name => {
                        state.name.pop();
                    }
                    CreateProjectStep::Path => {
                        state.path.pop();
                    }
                    CreateProjectStep::Agent => {}
                }
            }
            app.refresh_create_project_agent_selection();
        }
        code if is_agent_step
            && matches!(code, KeyCode::Char('j') | KeyCode::Down | KeyCode::Right) =>
        {
            let (is_agent_step, path, next_index) = match &app.mode {
                AppMode::CreatingProject(state) => (
                    matches!(state.step, CreateProjectStep::Agent),
                    std::path::PathBuf::from(&state.path),
                    state.agent_index.saturating_add(1),
                ),
                _ => (false, std::path::PathBuf::new(), 0),
            };
            if is_agent_step {
                let allowed = app.allowed_agents_for_project_path(&path);
                if next_index < allowed.len()
                    && let AppMode::CreatingProject(state) = &mut app.mode
                {
                    state.agent_index = next_index;
                    state.agent = allowed[next_index].clone();
                }
            }
        }
        code if is_agent_step
            && matches!(code, KeyCode::Char('k') | KeyCode::Up | KeyCode::Left) =>
        {
            let (is_agent_step, path, current_index) = match &app.mode {
                AppMode::CreatingProject(state) => (
                    matches!(state.step, CreateProjectStep::Agent),
                    std::path::PathBuf::from(&state.path),
                    state.agent_index,
                ),
                _ => (false, std::path::PathBuf::new(), 0),
            };
            if is_agent_step && current_index > 0 {
                let next_index = current_index - 1;
                let allowed = app.allowed_agents_for_project_path(&path);
                if let AppMode::CreatingProject(state) = &mut app.mode {
                    state.agent_index = next_index;
                    state.agent = allowed[next_index].clone();
                }
            }
        }
        KeyCode::Char(c) => {
            if let AppMode::CreatingProject(state) = &mut app.mode {
                match state.step {
                    CreateProjectStep::Name => state.name.push(c),
                    CreateProjectStep::Path => state.path.push(c),
                    CreateProjectStep::Agent => {}
                }
            }
            app.refresh_create_project_agent_selection();
        }
        _ => {}
    }
    Ok(())
}

pub fn handle_help_key(app: &mut App, key: KeyEvent) -> Result<()> {
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
            let from_view = match std::mem::replace(&mut app.mode, AppMode::Normal) {
                AppMode::Help(state) => state.from_view,
                other => {
                    app.mode = other;
                    return Ok(());
                }
            };
            if let Some(view) = from_view {
                app.return_to_dashboard(&view);
            }
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if let AppMode::Help(state) = &mut app.mode {
                state.scroll_offset = state.scroll_offset.saturating_add(1);
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if let AppMode::Help(state) = &mut app.mode {
                state.scroll_offset = state.scroll_offset.saturating_sub(1);
            }
        }
        KeyCode::PageDown => {
            if let AppMode::Help(state) = &mut app.mode {
                state.scroll_offset = state.scroll_offset.saturating_add(10);
            }
        }
        KeyCode::PageUp => {
            if let AppMode::Help(state) = &mut app.mode {
                state.scroll_offset = state.scroll_offset.saturating_sub(10);
            }
        }
        KeyCode::Home | KeyCode::Char('g') => {
            if let AppMode::Help(state) = &mut app.mode {
                state.scroll_offset = 0;
            }
        }
        KeyCode::End | KeyCode::Char('G') => {
            if let AppMode::Help(state) = &mut app.mode {
                state.scroll_offset = usize::MAX;
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn handle_latest_prompt_key(app: &mut App, key: KeyCode) -> Result<()> {
    match key {
        KeyCode::Esc | KeyCode::Char('q') => {
            let view = match std::mem::replace(&mut app.mode, AppMode::Normal) {
                AppMode::LatestPrompt(state) => state.view,
                other => {
                    app.mode = other;
                    return Ok(());
                }
            };
            app.return_to_dashboard(&view);
        }
        KeyCode::Tab | KeyCode::Enter => {
            app.inject_latest_prompt()?;
        }
        KeyCode::Char('j') | KeyCode::Down => {
            app.latest_prompt_select_next();
        }
        KeyCode::Char('k') | KeyCode::Up => {
            app.latest_prompt_select_prev();
        }
        KeyCode::Char('y') => {
            app.copy_selected_prompt_to_clipboard()?;
        }
        KeyCode::Char('s') => {
            app.save_latest_prompt_as_template();
        }
        _ => {}
    }
    Ok(())
}

const MARKDOWN_FAST_SCROLL_STEP: usize = 8;

pub fn handle_markdown_viewer_key(app: &mut App, key: KeyEvent) -> Result<()> {
    if key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(
            key.code,
            KeyCode::Char('j') | KeyCode::Down | KeyCode::Char('k') | KeyCode::Up
        )
    {
        if let AppMode::MarkdownViewer(state) = &mut app.mode {
            match key.code {
                KeyCode::Char('j') | KeyCode::Down => {
                    state.scroll_offset = state
                        .scroll_offset
                        .saturating_add(MARKDOWN_FAST_SCROLL_STEP);
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    state.scroll_offset = state
                        .scroll_offset
                        .saturating_sub(MARKDOWN_FAST_SCROLL_STEP);
                }
                _ => {}
            }
        }
        return Ok(());
    }

    match key.code {
        KeyCode::Char('b') => {
            let picker = match std::mem::replace(&mut app.mode, AppMode::Normal) {
                AppMode::MarkdownViewer(mut state) => {
                    if let Some(picker) = state.return_to_picker.take() {
                        picker
                    } else {
                        app.mode = AppMode::MarkdownViewer(state);
                        return Ok(());
                    }
                }
                other => {
                    app.mode = other;
                    return Ok(());
                }
            };
            app.mode = AppMode::MarkdownFilePicker(picker);
        }
        KeyCode::Char('/') => {
            app.open_markdown_file_picker_from_viewer()?;
        }
        KeyCode::Char('r') => {
            app.refresh_markdown_viewer()?;
        }
        KeyCode::Esc | KeyCode::Char('q') => {
            let from_view = match std::mem::replace(&mut app.mode, AppMode::Normal) {
                AppMode::MarkdownViewer(state) => state.from_view,
                other => {
                    app.mode = other;
                    return Ok(());
                }
            };
            if let Some(view) = from_view {
                app.return_to_dashboard(&view);
            }
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if let AppMode::MarkdownViewer(state) = &mut app.mode {
                state.scroll_offset = state.scroll_offset.saturating_add(1);
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if let AppMode::MarkdownViewer(state) = &mut app.mode {
                state.scroll_offset = state.scroll_offset.saturating_sub(1);
            }
        }
        KeyCode::PageDown => {
            if let AppMode::MarkdownViewer(state) = &mut app.mode {
                state.scroll_offset = state.scroll_offset.saturating_add(10);
            }
        }
        KeyCode::PageUp => {
            if let AppMode::MarkdownViewer(state) = &mut app.mode {
                state.scroll_offset = state.scroll_offset.saturating_sub(10);
            }
        }
        KeyCode::Home | KeyCode::Char('g') => {
            if let AppMode::MarkdownViewer(state) = &mut app.mode {
                state.scroll_offset = 0;
            }
        }
        KeyCode::End | KeyCode::Char('G') => {
            if let AppMode::MarkdownViewer(state) = &mut app.mode {
                state.scroll_offset = usize::MAX;
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn handle_delete_project_key(app: &mut App, key: KeyCode) -> Result<()> {
    match key {
        KeyCode::Char('y') => {
            app.delete_project()?;
        }
        KeyCode::Char('n') | KeyCode::Esc => {
            app.mode = AppMode::Normal;
        }
        _ => {}
    }
    Ok(())
}

pub fn handle_delete_feature_key(app: &mut App, key: KeyCode) -> Result<()> {
    match key {
        KeyCode::Char('y') => {
            app.delete_feature()?;
        }
        KeyCode::Char('n') | KeyCode::Esc => {
            app.mode = AppMode::Normal;
        }
        _ => {}
    }
    Ok(())
}

/// Pre-start resource warning. Confirming starts the agent anyway — the gate
/// is advisory, so `y` must always be able to proceed.
pub fn handle_resource_confirm_key(app: &mut App, key: KeyCode) -> Result<()> {
    match key {
        KeyCode::Char('y') | KeyCode::Enter => app.confirm_pending_start()?,
        KeyCode::Char('n') | KeyCode::Esc => app.cancel_pending_start(),
        _ => {}
    }
    Ok(())
}

pub fn handle_theme_picker_key(app: &mut App, key: KeyCode) -> Result<()> {
    match key {
        KeyCode::Char('j') | KeyCode::Down => {
            let preview = if let AppMode::ThemePicker(state) = &mut app.mode {
                let preview = if let Some(group) = &mut state.group {
                    if group.selected + 1 < group.themes.len() {
                        group.selected += 1;
                    }
                    group.themes.get(group.selected).copied()
                } else {
                    // FORK: skip the Dark / Light section headers.
                    state.selected = crate::fork::theme_picker::step_selection(
                        &state.entries,
                        state.selected,
                        1,
                    );
                    state
                        .entries
                        .get(state.selected)
                        .and_then(|e| e.preview_theme())
                };
                if let Some(name) = preview {
                    state.previewed = name;
                }
                preview
            } else {
                None
            };
            if let Some(name) = preview {
                app.preview_theme(name);
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            let preview = if let AppMode::ThemePicker(state) = &mut app.mode {
                let preview = if let Some(group) = &mut state.group {
                    if group.selected > 0 {
                        group.selected -= 1;
                    }
                    group.themes.get(group.selected).copied()
                } else {
                    // FORK: skip the Dark / Light section headers.
                    state.selected = crate::fork::theme_picker::step_selection(
                        &state.entries,
                        state.selected,
                        -1,
                    );
                    state
                        .entries
                        .get(state.selected)
                        .and_then(|e| e.preview_theme())
                };
                if let Some(name) = preview {
                    state.previewed = name;
                }
                preview
            } else {
                None
            };
            if let Some(name) = preview {
                app.preview_theme(name);
            }
        }
        KeyCode::Enter => {
            if let AppMode::ThemePicker(state) = &app.mode {
                if let Some(group) = &state.group {
                    if let Some(name) = group.themes.get(group.selected).copied() {
                        app.apply_theme(name);
                    }
                } else {
                    match state.entries.get(state.selected) {
                        Some(ThemePickerEntry::Theme(name)) => {
                            app.apply_theme(*name);
                        }
                        Some(ThemePickerEntry::Group { label, themes }) => {
                            let label = *label;
                            let themes = themes.clone();
                            // Land on whatever this row was already previewing
                            // so the drill-in doesn't jump somewhere else.
                            let group_selected = themes
                                .iter()
                                .position(|t| *t == state.previewed)
                                .unwrap_or(0);
                            let preview = themes.get(group_selected).copied();
                            if let AppMode::ThemePicker(state) = &mut app.mode {
                                state.group = Some(ThemePickerGroupState {
                                    label,
                                    themes,
                                    selected: group_selected,
                                });
                                if let Some(name) = preview {
                                    state.previewed = name;
                                }
                            }
                            if let Some(name) = preview {
                                app.preview_theme(name);
                            }
                        }
                        Some(ThemePickerEntry::Header(_)) | None => {} // FORK
                    }
                }
            }
        }
        KeyCode::Char('t') => {
            app.toggle_transparent_background();
        }
        KeyCode::Esc | KeyCode::Char('q') => {
            let mut had_group = false;
            let mut preview = None;
            if let AppMode::ThemePicker(state) = &mut app.mode {
                if state.group.take().is_some() {
                    // Leave the current preview as-is: it's already a member
                    // of the group we just backed out of, so there's
                    // nothing more accurate to show at the top level.
                    had_group = true;
                } else {
                    preview = Some(state.original_theme);
                }
            }
            if let Some(name) = preview {
                app.preview_theme(name);
            }
            if !had_group {
                app.mode = AppMode::Normal;
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn handle_rename_session_key(app: &mut App, key: KeyCode) -> Result<()> {
    match key {
        KeyCode::Esc => {
            app.cancel_rename_session();
        }
        KeyCode::Enter => {
            app.apply_rename_session()?;
        }
        KeyCode::Backspace => {
            if let AppMode::RenamingSession(state) = &mut app.mode {
                state.input.pop();
            }
        }
        KeyCode::Char(c) => {
            if let AppMode::RenamingSession(state) = &mut app.mode {
                state.input.push(c);
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn handle_rename_feature_key(app: &mut App, key: KeyCode) -> Result<()> {
    match key {
        KeyCode::Esc => {
            app.cancel_rename_feature();
        }
        KeyCode::Enter => {
            app.apply_rename_feature()?;
        }
        KeyCode::Backspace => {
            if let AppMode::RenamingFeature(state) = &mut app.mode {
                state.input.pop();
            }
        }
        KeyCode::Char(c) => {
            if let AppMode::RenamingFeature(state) = &mut app.mode {
                state.input.push(c);
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn handle_session_config_key(app: &mut App, key: KeyCode) -> Result<()> {
    match key {
        KeyCode::Char('j') | KeyCode::Down => match &mut app.mode {
            AppMode::SessionConfig(state)
                if state.selected_agent + 1 < state.allowed_agents.len() =>
            {
                state.selected_agent += 1;
            }
            AppMode::ProjectAgentConfig(state)
                if state.selected_agent + 1 < state.allowed_agents.len() =>
            {
                state.selected_agent += 1;
            }
            _ => {}
        },
        KeyCode::Char('k') | KeyCode::Up => match &mut app.mode {
            AppMode::SessionConfig(state) if state.selected_agent > 0 => {
                state.selected_agent -= 1;
            }
            AppMode::ProjectAgentConfig(state) if state.selected_agent > 0 => {
                state.selected_agent -= 1;
            }
            _ => {}
        },
        KeyCode::Enter => {
            app.apply_session_config()?;
        }
        KeyCode::Esc | KeyCode::Char('q') => {
            app.cancel_session_config();
        }
        _ => {}
    }
    Ok(())
}

pub fn handle_stopped_session_dialog_key(app: &mut App, key: KeyEvent) -> Result<()> {
    let choice_count = match &app.mode {
        AppMode::StoppedSessionDialog(state) => state.choices.len(),
        _ => return Ok(()),
    };
    if choice_count == 0 {
        return Ok(());
    }

    // Ctrl+C is a cancel reflex in a TUI, not a request to discard the saved
    // resume ID, and no letter shortcut here should fire with a modifier held.
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        if matches!(key.code, KeyCode::Char('c')) {
            app.confirm_stopped_session_choice(StoppedSessionChoice::Cancel);
        }
        return Ok(());
    }
    if matches!(key.code, KeyCode::Char(_)) && key.modifiers != KeyModifiers::NONE {
        return Ok(());
    }

    let shortcut = |wanted: StoppedSessionChoice| match &app.mode {
        AppMode::StoppedSessionDialog(state) => state.choices.contains(&wanted),
        _ => false,
    };

    match key.code {
        KeyCode::Char('j') | KeyCode::Down | KeyCode::Tab => {
            if let AppMode::StoppedSessionDialog(state) = &mut app.mode {
                state.selected = (state.selected + 1) % choice_count;
            }
        }
        KeyCode::Char('k') | KeyCode::Up | KeyCode::BackTab => {
            if let AppMode::StoppedSessionDialog(state) = &mut app.mode {
                state.selected = state.selected.checked_sub(1).unwrap_or(choice_count - 1);
            }
        }
        KeyCode::Char('r') => {
            if shortcut(StoppedSessionChoice::Resume) {
                app.confirm_stopped_session_choice(StoppedSessionChoice::Resume);
            }
        }
        KeyCode::Char('c') => {
            if shortcut(StoppedSessionChoice::Clear) {
                app.confirm_stopped_session_choice(StoppedSessionChoice::Clear);
            }
        }
        KeyCode::Char('p') => {
            if shortcut(StoppedSessionChoice::PickSession) {
                app.confirm_stopped_session_choice(StoppedSessionChoice::PickSession);
            }
        }
        KeyCode::Esc | KeyCode::Char('q') => {
            app.confirm_stopped_session_choice(StoppedSessionChoice::Cancel);
        }
        KeyCode::Enter => {
            let choice = match &app.mode {
                AppMode::StoppedSessionDialog(state) => state.choices.get(state.selected).copied(),
                _ => None,
            };
            if let Some(choice) = choice {
                app.confirm_stopped_session_choice(choice);
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn handle_debug_log_key(app: &mut App, key: KeyCode) -> Result<()> {
    match key {
        KeyCode::Esc | KeyCode::Char('q') => {
            let from_view = match std::mem::replace(&mut app.mode, AppMode::Normal) {
                AppMode::DebugLog(state) => state.from_view,
                other => {
                    app.mode = other;
                    return Ok(());
                }
            };
            if let Some(view) = from_view {
                app.return_to_dashboard(&view);
            }
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if let AppMode::DebugLog(state) = &mut app.mode {
                state.scroll_offset = state.scroll_offset.saturating_add(1);
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if let AppMode::DebugLog(state) = &mut app.mode {
                state.scroll_offset = state.scroll_offset.saturating_sub(1);
            }
        }
        KeyCode::PageDown => {
            if let AppMode::DebugLog(state) = &mut app.mode {
                state.scroll_offset = state.scroll_offset.saturating_add(10);
            }
        }
        KeyCode::PageUp => {
            if let AppMode::DebugLog(state) = &mut app.mode {
                state.scroll_offset = state.scroll_offset.saturating_sub(10);
            }
        }
        KeyCode::Home | KeyCode::Char('g') => {
            if let AppMode::DebugLog(state) = &mut app.mode {
                state.scroll_offset = 0;
            }
        }
        KeyCode::End | KeyCode::Char('G') => {
            if let AppMode::DebugLog(state) = &mut app.mode {
                state.scroll_offset = usize::MAX;
            }
        }
        KeyCode::Char('c') => {
            app.debug_log.clear();
            if let AppMode::DebugLog(state) = &mut app.mode {
                state.scroll_offset = 0;
            }
        }
        KeyCode::Char('p') => {
            if let AppMode::DebugLog(state) = &mut app.mode {
                state.hide_perf_logs = !state.hide_perf_logs;
                state.scroll_offset = 0;
            }
        }
        KeyCode::Char('/') => {
            let from_view = match std::mem::replace(&mut app.mode, AppMode::Normal) {
                AppMode::DebugLog(state) => state.from_view,
                other => {
                    app.mode = other;
                    return Ok(());
                }
            };
            app.open_command_picker_with_focus(from_view, crate::app::CommandPickerFocus::Local);
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::*;
    use crate::app::{CommandAction, MarkdownFilePickerState, MarkdownViewerState, ViewState};
    use crate::project::{AgentKind, Feature, Project, ProjectStore, VibeMode};
    use crate::theme::{Theme, ThemeName};
    use crate::traits::{MockTmuxOps, MockWorktreeOps};
    use tempfile::TempDir;

    fn markdown_view() -> ViewState {
        ViewState::new(
            "demo".to_string(),
            "feature".to_string(),
            "aiw-feature".to_string(),
            "claude".to_string(),
            "Claude 1".to_string(),
            crate::project::SessionKind::Claude,
            VibeMode::Vibeless,
            false,
        )
    }

    fn markdown_app() -> App {
        let store = ProjectStore {
            version: 5,
            projects: vec![Project::new(
                "demo".into(),
                PathBuf::from("/tmp/demo"),
                true,
                AgentKind::Claude,
            )],
            session_bookmarks: vec![],
            available_harnesses: vec![],
            prompt_templates: Vec::new(),
            extra: HashMap::new(),
        };
        App::new_for_test(
            store,
            Box::new(MockTmuxOps::new()),
            Box::new(MockWorktreeOps::new()),
        )
    }

    fn markdown_app_with_feature(workdir: &std::path::Path, repo_root: &std::path::Path) -> App {
        let mut project = Project::new(
            "demo".into(),
            PathBuf::from("/tmp/demo"),
            true,
            AgentKind::Claude,
        );
        project.features.push(Feature::new(
            "feature".into(),
            "main".into(),
            workdir.to_path_buf(),
            true,
            VibeMode::Vibeless,
            false,
            false,
            AgentKind::Claude,
            false,
        ));
        let store = ProjectStore {
            version: 5,
            projects: vec![project],
            session_bookmarks: vec![],
            available_harnesses: vec![],
            prompt_templates: Vec::new(),
            extra: HashMap::new(),
        };
        let mut worktree = MockWorktreeOps::new();
        let repo_root = repo_root.to_path_buf();
        worktree
            .expect_repo_root()
            .returning(move |_| Ok(repo_root.clone()));
        App::new_for_test(store, Box::new(MockTmuxOps::new()), Box::new(worktree))
    }

    #[test]
    fn markdown_viewer_b_returns_to_picker_when_available() {
        let mut app = markdown_app();
        let view = markdown_view();
        let picker = MarkdownFilePickerState {
            files: vec![PathBuf::from("a.md"), PathBuf::from("b.md")],
            selected: 1,
            plan_only: true,
            search_active: false,
            query: String::new(),
            workdir: PathBuf::from("/tmp/demo"),
            repo_root: Some(PathBuf::from("/tmp/demo-repo")),
            purpose: crate::app::MarkdownFilePickerPurpose::Browse,
            from_view: Some(view.clone()),
        };
        app.mode = AppMode::MarkdownViewer(MarkdownViewerState {
            title: "b.md".into(),
            source_path: PathBuf::from("b.md"),
            content: "# Title".into(),
            scroll_offset: 0,
            rendered_width: 0,
            rendered_lines: Vec::new(),
            return_to_picker: Some(picker),
            from_view: Some(view),
            current_plan: false,
        });

        handle_markdown_viewer_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE),
        )
        .unwrap();

        match &app.mode {
            AppMode::MarkdownFilePicker(state) => {
                assert_eq!(state.selected, 1);
                assert_eq!(state.files.len(), 2);
            }
            _ => panic!("expected markdown picker after pressing b"),
        }
    }

    #[test]
    fn markdown_viewer_b_is_noop_without_picker_context() {
        let mut app = markdown_app();
        let view = markdown_view();
        app.mode = AppMode::MarkdownViewer(MarkdownViewerState {
            title: "notes.md".into(),
            source_path: PathBuf::from("notes.md"),
            content: "# Title".into(),
            scroll_offset: 0,
            rendered_width: 0,
            rendered_lines: Vec::new(),
            return_to_picker: None,
            from_view: Some(view),
            current_plan: false,
        });

        handle_markdown_viewer_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE),
        )
        .unwrap();

        assert!(matches!(app.mode, AppMode::MarkdownViewer(_)));
    }

    #[test]
    fn markdown_viewer_slash_opens_picker_for_current_feature() {
        let repo = TempDir::new().unwrap();
        let repo_root = TempDir::new().unwrap();
        std::fs::write(repo.path().join("alpha.md"), "# Alpha\n").unwrap();
        std::fs::write(repo.path().join("beta.md"), "# Beta\n").unwrap();

        let mut app = markdown_app_with_feature(repo.path(), repo_root.path());
        let view = markdown_view();
        app.mode = AppMode::MarkdownViewer(MarkdownViewerState {
            title: "alpha.md".into(),
            source_path: PathBuf::from("alpha.md"),
            content: "# Alpha".into(),
            scroll_offset: 0,
            rendered_width: 0,
            rendered_lines: Vec::new(),
            return_to_picker: None,
            from_view: Some(view),
            current_plan: false,
        });

        handle_markdown_viewer_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE),
        )
        .unwrap();
        // File discovery is async (MarkdownLoading); drive it to completion
        // as the event loop does before asserting the picker opened.
        app.complete_markdown_loading();

        match &app.mode {
            AppMode::MarkdownFilePicker(state) => {
                assert_eq!(state.files.len(), 2);
                assert_eq!(state.query, "");
                assert_eq!(state.selected, 0);
            }
            _ => panic!("expected markdown picker after pressing /"),
        }
    }

    #[test]
    fn markdown_viewer_ctrl_j_scrolls_faster() {
        let mut app = markdown_app();
        let view = markdown_view();
        app.mode = AppMode::MarkdownViewer(MarkdownViewerState {
            title: "notes.md".into(),
            source_path: PathBuf::from("notes.md"),
            content: "# Title".into(),
            scroll_offset: 3,
            rendered_width: 0,
            rendered_lines: Vec::new(),
            return_to_picker: None,
            from_view: Some(view),
            current_plan: false,
        });

        handle_markdown_viewer_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL),
        )
        .unwrap();

        match &app.mode {
            AppMode::MarkdownViewer(state) => {
                assert_eq!(state.scroll_offset, 3 + MARKDOWN_FAST_SCROLL_STEP);
            }
            _ => panic!("expected markdown viewer to stay open"),
        }
    }

    #[test]
    fn markdown_viewer_ctrl_up_scrolls_back_faster() {
        let mut app = markdown_app();
        let view = markdown_view();
        app.mode = AppMode::MarkdownViewer(MarkdownViewerState {
            title: "notes.md".into(),
            source_path: PathBuf::from("notes.md"),
            content: "# Title".into(),
            scroll_offset: 12,
            rendered_width: 0,
            rendered_lines: Vec::new(),
            return_to_picker: None,
            from_view: Some(view),
            current_plan: false,
        });

        handle_markdown_viewer_key(&mut app, KeyEvent::new(KeyCode::Up, KeyModifiers::CONTROL))
            .unwrap();

        match &app.mode {
            AppMode::MarkdownViewer(state) => {
                assert_eq!(state.scroll_offset, 12 - MARKDOWN_FAST_SCROLL_STEP);
            }
            _ => panic!("expected markdown viewer to stay open"),
        }
    }

    #[test]
    fn current_plan_viewer_refreshes_edits_in_place() {
        let workdir = TempDir::new().unwrap();
        let path = workdir.path().join("AIW_PLAN.md");
        std::fs::write(&path, "# Before\n").unwrap();
        let mut app = markdown_app_with_feature(workdir.path(), workdir.path());
        app.mode = AppMode::MarkdownViewer(MarkdownViewerState {
            title: "AIW_PLAN.md".into(),
            source_path: path.clone(),
            content: "# Before\n".into(),
            scroll_offset: 0,
            rendered_width: 80,
            rendered_lines: vec![ratatui::text::Line::from("stale")],
            return_to_picker: None,
            from_view: Some(markdown_view()),
            current_plan: true,
        });
        std::fs::write(&path, "# After\n").unwrap();

        handle_markdown_viewer_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
        )
        .unwrap();

        assert!(matches!(
            &app.mode,
            AppMode::MarkdownViewer(state)
                if state.content == "# After\n"
                    && state.rendered_width == 0
                    && state.rendered_lines.is_empty()
        ));
    }

    #[test]
    fn deleted_selected_plan_is_invalidated_and_returns_to_agent_view() {
        let workdir = TempDir::new().unwrap();
        let path = workdir.path().join("accepted.md");
        std::fs::write(&path, "# Accepted\n").unwrap();
        let canonical = path.canonicalize().unwrap();
        let mut app = markdown_app_with_feature(workdir.path(), workdir.path());
        app.store.projects[0].features[0].selected_plan_path = Some(canonical.clone());
        app.mode = AppMode::MarkdownViewer(MarkdownViewerState {
            title: "accepted.md".into(),
            source_path: canonical,
            content: "# Accepted\n".into(),
            scroll_offset: 0,
            rendered_width: 0,
            rendered_lines: Vec::new(),
            return_to_picker: None,
            from_view: Some(markdown_view()),
            current_plan: true,
        });
        std::fs::remove_file(path).unwrap();

        handle_markdown_viewer_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
        )
        .unwrap();

        assert!(matches!(app.mode, AppMode::Normal));
        assert!(
            app.store.projects[0].features[0]
                .selected_plan_path
                .is_none()
        );
        assert!(app.toasts.last().is_some_and(|toast| {
            toast
                .message
                .starts_with("Current plan is no longer available")
        }));
    }

    #[test]
    fn debug_log_slash_opens_local_command_picker() {
        let mut app = markdown_app();
        let view = markdown_view();
        app.mode = AppMode::DebugLog(crate::app::DebugLogState {
            scroll_offset: 0,
            from_view: Some(view.clone()),
            hide_perf_logs: false,
        });

        handle_debug_log_key(&mut app, KeyCode::Char('/')).unwrap();

        match &app.mode {
            AppMode::CommandPicker(state) => {
                assert!(matches!(
                    state
                        .commands
                        .get(state.selected)
                        .map(|entry| &entry.action),
                    Some(CommandAction::Local { .. })
                ));
                assert_eq!(
                    state.from_view.as_ref().map(|from_view| &from_view.window),
                    Some(&view.window)
                );
            }
            _ => panic!("expected command picker after pressing / in debug log"),
        }
    }

    #[test]
    fn debug_log_g_jumps_to_top() {
        let mut app = markdown_app();
        app.mode = AppMode::DebugLog(crate::app::DebugLogState {
            scroll_offset: 42,
            from_view: None,
            hide_perf_logs: false,
        });

        handle_debug_log_key(&mut app, KeyCode::Char('g')).unwrap();

        match &app.mode {
            AppMode::DebugLog(state) => assert_eq!(state.scroll_offset, 0),
            _ => panic!("expected debug log to stay open"),
        }
    }

    #[test]
    fn debug_log_g_uppercase_jumps_to_bottom() {
        let mut app = markdown_app();
        app.mode = AppMode::DebugLog(crate::app::DebugLogState {
            scroll_offset: 0,
            from_view: None,
            hide_perf_logs: false,
        });

        handle_debug_log_key(&mut app, KeyCode::Char('G')).unwrap();

        match &app.mode {
            AppMode::DebugLog(state) => assert_eq!(state.scroll_offset, usize::MAX),
            _ => panic!("expected debug log to stay open"),
        }
    }

    #[test]
    fn debug_log_p_toggles_perf_visibility() {
        let mut app = markdown_app();
        app.mode = AppMode::DebugLog(crate::app::DebugLogState {
            scroll_offset: 9,
            from_view: None,
            hide_perf_logs: false,
        });

        handle_debug_log_key(&mut app, KeyCode::Char('p')).unwrap();

        match &app.mode {
            AppMode::DebugLog(state) => {
                assert!(state.hide_perf_logs);
                assert_eq!(state.scroll_offset, 0);
            }
            _ => panic!("expected debug log to stay open"),
        }
    }

    fn theme_app(active: ThemeName) -> App {
        let store = ProjectStore {
            version: 5,
            projects: vec![],
            session_bookmarks: vec![],
            available_harnesses: vec![],
            prompt_templates: Vec::new(),
            extra: HashMap::new(),
        };
        let mut app = App::new_for_test(
            store,
            Box::new(MockTmuxOps::new()),
            Box::new(MockWorktreeOps::new()),
        );
        app.config.theme = active;
        app.start_theme_picker();
        app
    }

    fn picker(app: &App) -> &crate::app::ThemePickerState {
        match &app.mode {
            AppMode::ThemePicker(state) => state,
            _ => panic!("expected the theme picker to be open"),
        }
    }

    fn highlighted_group_theme(app: &App) -> ThemeName {
        let group = picker(app)
            .group
            .as_ref()
            .expect("expected to be on a group's second screen");
        group.themes[group.selected]
    }

    #[test]
    fn opening_the_picker_on_a_grouped_theme_lands_inside_the_group() {
        let app = theme_app(ThemeName::CatppuccinMacchiato);

        assert_eq!(picker(&app).group.as_ref().unwrap().label, "Catppuccin");
        assert_eq!(
            highlighted_group_theme(&app),
            ThemeName::CatppuccinMacchiato
        );
        match picker(&app).entries.get(picker(&app).selected) {
            Some(ThemePickerEntry::Group { label, .. }) => assert_eq!(*label, "Catppuccin"),
            _ => panic!("expected the group's own row to be selected at the top level"),
        }
    }

    #[test]
    fn opening_the_picker_on_a_standalone_theme_stays_at_the_top_level() {
        let app = theme_app(ThemeName::Nord);

        assert!(picker(&app).group.is_none());
        match picker(&app).entries.get(picker(&app).selected) {
            Some(ThemePickerEntry::Theme(name)) => assert_eq!(*name, ThemeName::Nord),
            _ => panic!("expected the Nord row to be selected"),
        }
    }

    #[test]
    fn entering_a_group_lands_on_the_variant_being_previewed() {
        // Opens straight onto Catppuccin's screen with Mocha highlighted.
        let mut app = theme_app(ThemeName::CatppuccinMocha);
        handle_theme_picker_key(&mut app, KeyCode::Esc).unwrap();
        let group_row = picker(&app).selected;

        // Navigating off the group row and back previews its first variant,
        // so drilling in must land there rather than on the active theme.
        // FORK: the dark Catppuccin group starts at Frappe (Latte is listed
        // under Light).
        handle_theme_picker_key(&mut app, KeyCode::Char('k')).unwrap();
        handle_theme_picker_key(&mut app, KeyCode::Char('j')).unwrap();
        assert_eq!(picker(&app).selected, group_row);
        assert_eq!(picker(&app).previewed, ThemeName::CatppuccinFrappe);

        handle_theme_picker_key(&mut app, KeyCode::Enter).unwrap();

        assert_eq!(highlighted_group_theme(&app), ThemeName::CatppuccinFrappe);
    }

    #[test]
    fn backing_out_of_a_group_keeps_its_variant_as_the_drill_in_target() {
        let mut app = theme_app(ThemeName::CatppuccinMocha);
        handle_theme_picker_key(&mut app, KeyCode::Esc).unwrap();

        assert!(
            picker(&app).group.is_none(),
            "Esc should not close the picker"
        );
        assert_eq!(picker(&app).previewed, ThemeName::CatppuccinMocha);

        handle_theme_picker_key(&mut app, KeyCode::Enter).unwrap();

        assert_eq!(highlighted_group_theme(&app), ThemeName::CatppuccinMocha);
    }

    #[test]
    fn enter_on_a_standalone_theme_applies_it_and_closes() {
        let mut app = theme_app(ThemeName::Nord);
        handle_theme_picker_key(&mut app, KeyCode::Char('j')).unwrap();

        handle_theme_picker_key(&mut app, KeyCode::Enter).unwrap();

        assert_eq!(app.config.theme, ThemeName::GruvboxDark);
        assert!(matches!(app.mode, AppMode::Normal));
    }

    #[test]
    fn enter_inside_a_group_applies_the_highlighted_variant() {
        // FORK: Latte moved to the Light section; Frappe opens the dark group.
        let mut app = theme_app(ThemeName::CatppuccinFrappe);
        handle_theme_picker_key(&mut app, KeyCode::Char('j')).unwrap();

        handle_theme_picker_key(&mut app, KeyCode::Enter).unwrap();

        assert_eq!(app.config.theme, ThemeName::CatppuccinMacchiato);
        assert!(matches!(app.mode, AppMode::Normal));
    }

    #[test]
    fn esc_at_the_top_level_restores_the_original_theme_and_closes() {
        let mut app = theme_app(ThemeName::Nord);
        handle_theme_picker_key(&mut app, KeyCode::Char('j')).unwrap();
        assert_eq!(app.theme.name, Theme::load(&ThemeName::GruvboxDark).name);

        handle_theme_picker_key(&mut app, KeyCode::Esc).unwrap();

        assert!(matches!(app.mode, AppMode::Normal));
        assert_eq!(app.config.theme, ThemeName::Nord);
        assert_eq!(app.theme.name, Theme::load(&ThemeName::Nord).name);
    }

    // FORK: live preview across the Dark / Light sections.
    #[test]
    fn moving_previews_live_skips_headers_and_esc_reverts() {
        let mut app = theme_app(ThemeName::Ember);
        // Ember is the first row under the "Dark" header: up stays put.
        handle_theme_picker_key(&mut app, KeyCode::Char('k')).unwrap();
        assert_eq!(picker(&app).previewed, ThemeName::Ember);

        handle_theme_picker_key(&mut app, KeyCode::Char('j')).unwrap();
        assert_eq!(
            app.theme.name, "graphite",
            "dashboard restyles while moving"
        );

        let mut app = theme_app(ThemeName::Paper);
        handle_theme_picker_key(&mut app, KeyCode::Char('k')).unwrap();
        assert!(
            matches!(
                picker(&app).entries.get(picker(&app).selected),
                Some(ThemePickerEntry::Group {
                    label: "Gruvbox Material",
                    ..
                })
            ),
            "up from Paper skips the Light header"
        );
        handle_theme_picker_key(&mut app, KeyCode::Esc).unwrap();
        assert!(matches!(app.mode, AppMode::Normal));
        assert_eq!(app.theme.name, "paper");
        assert_eq!(app.config.theme, ThemeName::Paper);
    }
}
