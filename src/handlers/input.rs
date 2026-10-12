use anyhow::Result;

use crate::app::{
    App, AppMode, CreateFeatureStep, CreateProjectStep, PromptEditorFocus, TodoEditTarget,
};

pub fn handle_paste(app: &mut App, text: &str) -> Result<()> {
    match &app.mode {
        AppMode::CreatingProject(_) => {
            if let AppMode::CreatingProject(state) = &mut app.mode {
                match state.step {
                    CreateProjectStep::Name => {
                        state.name.push_str(text);
                    }
                    CreateProjectStep::Path => {
                        state.path.push_str(text);
                    }
                    CreateProjectStep::Agent => {}
                }
            }
            app.refresh_create_project_agent_selection();
        }
        AppMode::CreatingFeature(_) => {
            if let AppMode::CreatingFeature(state) = &mut app.mode {
                match state.step {
                    CreateFeatureStep::Branch => {
                        state.branch.push_str(text);
                        state.branch_error = None;
                    }
                    CreateFeatureStep::SessionName => {
                        state.session_name.push_str(text);
                    }
                    // FORK: §19.3 — a pasted prompt keeps its newlines.
                    CreateFeatureStep::Prompt if state.prompt_picker.is_none() => {
                        state
                            .task_prompt
                            .push_str(&text.replace("\r\n", "\n").replace('\r', "\n"));
                    }
                    _ => {}
                }
            }
        }
        // FORK: §19.4 — a pasted note keeps its newlines; a group name is
        // one line.
        AppMode::NoteEditor(_) => {
            if let AppMode::NoteEditor(state) = &mut app.mode {
                state
                    .text
                    .push_str(&text.replace("\r\n", "\n").replace('\r', "\n"));
            }
        }
        AppMode::FeatureGroups(_) => {
            if let AppMode::FeatureGroups(state) = &mut app.mode
                && let Some((_, value)) = state.edit.as_mut()
            {
                for chunk in text.split(['\n', '\r']) {
                    value.push_str(chunk);
                }
            }
        }
        AppMode::RenamingSession(_) => {
            if let AppMode::RenamingSession(state) = &mut app.mode {
                state.input.push_str(text);
            }
        }
        AppMode::RenamingFeature(_) => {
            if let AppMode::RenamingFeature(state) = &mut app.mode {
                state.input.push_str(text);
            }
        }
        AppMode::Searching(_) => {
            if let AppMode::Searching(state) = &mut app.mode {
                state.query.push_str(text);
                app.perform_search();
            }
        }
        AppMode::PromptEditor(_) => {
            if let AppMode::PromptEditor(state) = &mut app.mode {
                match state.focus {
                    // Name and Tags are single-line; collapse pasted newlines.
                    PromptEditorFocus::Name | PromptEditorFocus::Tags => {
                        let field = match state.focus {
                            PromptEditorFocus::Tags => &mut state.tags,
                            _ => &mut state.name,
                        };
                        for chunk in text.split(['\n', '\r']) {
                            field.push_str(chunk);
                        }
                    }
                    PromptEditorFocus::Body => {
                        state.editor.insert_str(text);
                    }
                }
            }
        }
        AppMode::Todos(_) => {
            if let AppMode::Todos(state) = &mut app.mode
                && let Some(editor) = &mut state.editor
            {
                // Titles are single-line: discard line breaks instead of letting
                // a pasted newline submit the edit. Notes and scratchpads use
                // the editor's existing multi-line representation.
                let text = match editor.target {
                    TodoEditTarget::New | TodoEditTarget::Title => {
                        text.split(['\n', '\r']).collect::<String>()
                    }
                    TodoEditTarget::Notes | TodoEditTarget::Scratchpad => {
                        text.replace("\r\n", "\n").replace('\r', "\n")
                    }
                };
                editor.editor.insert_str(&text);
            }
        }
        AppMode::PlaceholderFill(_) => {
            if let AppMode::PlaceholderFill(state) = &mut app.mode {
                // Select slots have no text field to paste into.
                if !state.is_select() {
                    state.input.insert_str(text);
                }
            }
        }
        _ => {}
    }
    Ok(())
}
