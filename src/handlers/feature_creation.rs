use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{App, AppMode, CreateFeatureStep, FeaturePromptPicker};
use crate::fork::fan_out::FanOutLayout;
use crate::project::{AgentKind, VibeMode, normalized_feature_name};

fn branch_error(app: &App) -> Option<String> {
    let state = match &app.mode {
        AppMode::CreatingFeature(state) => state,
        _ => return None,
    };

    let branch = state.branch.trim();
    if branch.is_empty() {
        return Some("Feature name cannot be empty".to_string());
    }

    let project = app.store.find_project(&state.project_name)?;
    let normalized_branch = normalized_feature_name(branch);
    if project
        .features
        .iter()
        .any(|feature| normalized_feature_name(&feature.name) == normalized_branch)
    {
        return Some(format!(
            "Feature '{}' already exists in '{}'",
            state.branch, state.project_name
        ));
    }

    // FORK: the rules of `git check-ref-format --branch`, said plainly here
    // instead of as raw git output after the worktree add fails.
    crate::fork::branch_name::validate_branch_name(branch).err()
}

pub fn handle_create_feature_key(app: &mut App, key: KeyCode) -> Result<()> {
    let step = match &app.mode {
        AppMode::CreatingFeature(state) => state.step.clone(),
        _ => return Ok(()),
    };

    match step {
        CreateFeatureStep::Source => {
            // Max source options:
            //   0 = new branch
            //   1 = existing worktree
            //   2 = use preset (only if presets exist)
            let preset_count = match &app.mode {
                AppMode::CreatingFeature(state) => state.feature_presets.len(),
                _ => 0,
            };
            let source_options = if preset_count > 0 { 3 } else { 2 };
            match key {
                KeyCode::Esc => {
                    app.cancel_create();
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if let AppMode::CreatingFeature(state) = &mut app.mode {
                        state.source_index = (state.source_index + 1) % source_options;
                    }
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    if let AppMode::CreatingFeature(state) = &mut app.mode {
                        state.source_index = if state.source_index == 0 {
                            source_options - 1
                        } else {
                            state.source_index - 1
                        };
                    }
                }
                KeyCode::Enter => {
                    if let AppMode::CreatingFeature(state) = &mut app.mode {
                        state.step = match state.source_index {
                            0 => CreateFeatureStep::Branch,
                            1 => CreateFeatureStep::ExistingWorktree,
                            _ => CreateFeatureStep::SelectPreset,
                        };
                    }
                }
                _ => {}
            }
        }
        CreateFeatureStep::ExistingWorktree => match key {
            KeyCode::Esc => {
                if let AppMode::CreatingFeature(state) = &mut app.mode {
                    if state.worktree_search_active {
                        state.worktree_search_active = false;
                    } else {
                        state.step = CreateFeatureStep::Source;
                    }
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let AppMode::CreatingFeature(state) = &mut app.mode {
                    state.clamp_worktree_selection();
                    let visible = state.visible_worktree_indices();
                    if !visible.is_empty() {
                        let pos = visible
                            .iter()
                            .position(|&idx| idx == state.worktree_index)
                            .unwrap_or(0);
                        state.worktree_index = visible[(pos + 1) % visible.len()];
                    }
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let AppMode::CreatingFeature(state) = &mut app.mode {
                    state.clamp_worktree_selection();
                    let visible = state.visible_worktree_indices();
                    if !visible.is_empty() {
                        let pos = visible
                            .iter()
                            .position(|&idx| idx == state.worktree_index)
                            .unwrap_or(0);
                        state.worktree_index = if pos == 0 {
                            *visible.last().unwrap()
                        } else {
                            visible[pos - 1]
                        };
                    }
                }
            }
            KeyCode::Char('/') => {
                if let AppMode::CreatingFeature(state) = &mut app.mode {
                    if state.worktree_search_active {
                        state.worktree_query.push('/');
                    } else {
                        state.worktree_search_active = true;
                        state.worktree_query.clear();
                    }
                    state.clamp_worktree_selection();
                }
            }
            KeyCode::Backspace => {
                if let AppMode::CreatingFeature(state) = &mut app.mode
                    && state.worktree_search_active
                {
                    state.worktree_query.pop();
                    state.clamp_worktree_selection();
                }
            }
            KeyCode::Enter => {
                if let AppMode::CreatingFeature(state) = &mut app.mode {
                    state.clamp_worktree_selection();
                    let visible = state.visible_worktree_indices();
                    if state.worktrees.is_empty() {
                        app.push_toast_warning("No available worktrees");
                    } else if visible.is_empty() {
                        app.push_toast_warning("No worktrees match the current filter");
                    } else if let Some(wt) = state.worktrees.get(state.worktree_index) {
                        state.branch = wt.branch.clone().unwrap_or_else(|| {
                            wt.path
                                .file_name()
                                .map(|n: &std::ffi::OsStr| n.to_string_lossy().into_owned())
                                .unwrap_or_default()
                        });
                        state.step = CreateFeatureStep::Mode;
                    }
                }
            }
            KeyCode::Char(c) => {
                if let AppMode::CreatingFeature(state) = &mut app.mode
                    && state.worktree_search_active
                {
                    state.worktree_query.push(c);
                    state.clamp_worktree_selection();
                }
            }
            _ => {}
        },
        CreateFeatureStep::SelectPreset => match key {
            KeyCode::Esc => {
                if let AppMode::CreatingFeature(state) = &mut app.mode {
                    state.step = CreateFeatureStep::Source;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let AppMode::CreatingFeature(state) = &mut app.mode {
                    let len = state.feature_presets.len();
                    if len > 0 {
                        state.preset_index = (state.preset_index + 1) % len;
                    }
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let AppMode::CreatingFeature(state) = &mut app.mode {
                    let len = state.feature_presets.len();
                    if len > 0 {
                        state.preset_index = if state.preset_index == 0 {
                            len - 1
                        } else {
                            state.preset_index - 1
                        };
                    }
                }
            }
            KeyCode::Enter => {
                let preset_index = match &app.mode {
                    AppMode::CreatingFeature(s) => s.preset_index,
                    _ => return Ok(()),
                };
                let preset = match &app.mode {
                    AppMode::CreatingFeature(state) => {
                        state.feature_presets.get(preset_index).cloned()
                    }
                    _ => None,
                };
                let mut missing_layout = None;
                if let Some(preset) = preset
                    && let AppMode::CreatingFeature(state) = &mut app.mode
                {
                    // Pre-fill fields from preset.
                    state.mode = preset.mode;
                    state.agent = preset.agent.clone();
                    state.enable_chrome = preset.enable_chrome;
                    // FORK: §19.3 — a preset can start several agents.
                    state.ticked = preset
                        .agents
                        .iter()
                        .filter(|a| state.allowed_agents.contains(a))
                        .cloned()
                        .collect();
                    if state.ticked.len() < 2 {
                        state.ticked.clear();
                    }
                    if let Some(layout) = preset.fan_out {
                        state.fan_out = layout;
                    }
                    // FORK: §19.4 — the preset's layout preset, by name.
                    if let Some(name) = preset.layout.as_deref() {
                        match state.layout_presets.iter().position(|n| n == name) {
                            Some(i) => state.layout_index = i + 1,
                            None => {
                                state.layout_index = 0;
                                state.branch_error = None;
                                missing_layout = Some(format!(
                                    "Preset '{}' names layout '{name}', which is not in layout_presets",
                                    preset.name
                                ));
                            }
                        }
                    }
                    state.sync_primary();
                    if let Some(ref prefix) = preset.branch_prefix
                        && !prefix.is_empty()
                    {
                        state.branch = format!("{}/", prefix);
                    }
                    state.step = CreateFeatureStep::Branch;
                }
                if let Some(message) = missing_layout {
                    app.push_toast_warning(message);
                }
            }
            _ => {}
        },
        CreateFeatureStep::Branch => match key {
            KeyCode::Esc => {
                if let AppMode::CreatingFeature(state) = &mut app.mode {
                    if state.worktrees.is_empty() {
                        app.cancel_create();
                    } else {
                        state.step = CreateFeatureStep::Source;
                    }
                } else {
                    app.cancel_create();
                }
            }
            KeyCode::Enter => {
                if let Some(error) = branch_error(app) {
                    if let AppMode::CreatingFeature(state) = &mut app.mode {
                        state.branch_error = Some(error.clone());
                    }
                    app.push_toast_warning(error);
                } else if let AppMode::CreatingFeature(state) = &mut app.mode {
                    state.branch_error = None;
                    state.step = CreateFeatureStep::Worktree;
                }
            }
            KeyCode::Backspace => {
                if let AppMode::CreatingFeature(state) = &mut app.mode {
                    state.branch.pop();
                    state.branch_error = None;
                }
            }
            KeyCode::Char(c) => {
                if let AppMode::CreatingFeature(state) = &mut app.mode {
                    state.branch.push(c);
                    state.branch_error = None;
                }
            }
            _ => {}
        },
        CreateFeatureStep::Worktree => match key {
            KeyCode::Esc => {
                if let AppMode::CreatingFeature(state) = &mut app.mode {
                    state.step = CreateFeatureStep::Branch;
                }
            }
            KeyCode::Tab | KeyCode::Enter | KeyCode::Char('l') => {
                if let AppMode::CreatingFeature(state) = &mut app.mode {
                    state.step = CreateFeatureStep::Mode;
                }
            }
            KeyCode::Down | KeyCode::Up | KeyCode::Char('j') | KeyCode::Char('k') => {
                if let AppMode::CreatingFeature(state) = &mut app.mode {
                    state.use_worktree = !state.use_worktree;
                }
            }
            _ => {}
        },
        CreateFeatureStep::Mode => match key {
            KeyCode::Esc => {
                if let AppMode::CreatingFeature(state) = &mut app.mode {
                    if state.mode_focus > 0 {
                        state.mode_focus -= 1;
                    } else if state.source_index == 1 && !state.worktrees.is_empty() {
                        state.step = CreateFeatureStep::ExistingWorktree;
                    } else {
                        state.step = CreateFeatureStep::Worktree;
                    }
                }
            }
            KeyCode::Tab | KeyCode::Char('l') => {
                if let AppMode::CreatingFeature(state) = &mut app.mode {
                    // For Claude: 0=agent 1=mode 2=chrome [3=layout]
                    // For others: 0=agent 1=mode [2=layout]
                    let max_focus = state.max_mode_focus();
                    if state.mode_focus < max_focus {
                        state.mode_focus += 1;
                    }
                }
            }
            KeyCode::BackTab | KeyCode::Char('h') => {
                if let AppMode::CreatingFeature(state) = &mut app.mode
                    && state.mode_focus > 0
                {
                    state.mode_focus -= 1;
                }
            }
            KeyCode::Enter => {
                if let AppMode::CreatingFeature(state) = &mut app.mode {
                    let max_focus = state.max_mode_focus();
                    if state.mode_focus < max_focus {
                        state.mode_focus += 1;
                    } else {
                        let is_supervibe = matches!(state.mode, VibeMode::SuperVibe);
                        if is_supervibe {
                            state.step = CreateFeatureStep::ConfirmSuperVibe;
                        } else {
                            // FORK: §19.3 — layout / prompt / session name.
                            state.advance_past_mode();
                        }
                    }
                }
            }
            // FORK: §19.3 — Space ticks the highlighted harness; ticking a
            // second one makes this a fan-out.
            KeyCode::Char(' ') => {
                if let AppMode::CreatingFeature(state) = &mut app.mode
                    && state.mode_focus == 0
                {
                    state.toggle_tick();
                }
            }
            // FORK: §19.7 — `b` on the harness list: Claude (background).
            KeyCode::Char('b') => {
                if let AppMode::CreatingFeature(state) = &mut app.mode
                    && state.mode_focus == 0
                    && state.background_available()
                {
                    state.background = !state.background;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let AppMode::CreatingFeature(state) = &mut app.mode {
                    match state.mode_focus {
                        0 => {
                            state.agent_index =
                                (state.agent_index + 1) % state.allowed_agents.len();
                            state.agent = state.allowed_agents[state.agent_index].clone();
                            state.sync_primary(); // FORK: §19.3
                        }
                        1 => {
                            state.mode_index = (state.mode_index + 1) % VibeMode::ALL.len();
                            state.mode = VibeMode::ALL[state.mode_index].clone();
                        }
                        // FORK: §19.4 — the Layout row.
                        f if Some(f) == state.layout_focus() => state.cycle_layout(true),
                        2 if state.agent == AgentKind::Claude => {
                            state.enable_chrome = !state.enable_chrome;
                        }
                        _ => {}
                    }
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if let AppMode::CreatingFeature(state) = &mut app.mode {
                    match state.mode_focus {
                        0 => {
                            state.agent_index = if state.agent_index == 0 {
                                state.allowed_agents.len() - 1
                            } else {
                                state.agent_index - 1
                            };
                            state.agent = state.allowed_agents[state.agent_index].clone();
                            state.sync_primary(); // FORK: §19.3
                        }
                        1 => {
                            state.mode_index = if state.mode_index == 0 {
                                VibeMode::ALL.len() - 1
                            } else {
                                state.mode_index - 1
                            };
                            state.mode = VibeMode::ALL[state.mode_index].clone();
                        }
                        f if Some(f) == state.layout_focus() => state.cycle_layout(false),
                        2 if state.agent == AgentKind::Claude => {
                            state.enable_chrome = !state.enable_chrome;
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        },
        CreateFeatureStep::SessionName => match key {
            KeyCode::Esc => {
                if let AppMode::CreatingFeature(state) = &mut app.mode {
                    state.step = CreateFeatureStep::Mode;
                }
            }
            KeyCode::Enter => {
                // FORK: §19.3 — the prompt step comes last.
                if let AppMode::CreatingFeature(state) = &mut app.mode {
                    if state.session_name.trim().is_empty() {
                        app.push_toast_warning("Session name cannot be empty");
                    } else {
                        state.step = CreateFeatureStep::Prompt;
                    }
                }
            }
            KeyCode::Backspace => {
                if let AppMode::CreatingFeature(state) = &mut app.mode {
                    state.session_name.pop();
                }
            }
            KeyCode::Char(c) => {
                if let AppMode::CreatingFeature(state) = &mut app.mode {
                    state.session_name.push(c);
                }
            }
            _ => {}
        },
        // FORK: §19.3 — shared worktree or one each.
        CreateFeatureStep::FanOut => match key {
            KeyCode::Esc => {
                if let AppMode::CreatingFeature(state) = &mut app.mode {
                    state.step = CreateFeatureStep::Mode;
                }
            }
            KeyCode::Down | KeyCode::Up | KeyCode::Char('j') | KeyCode::Char('k') => {
                if let AppMode::CreatingFeature(state) = &mut app.mode {
                    state.fan_out = match state.fan_out {
                        FanOutLayout::Shared => FanOutLayout::WorktreeEach,
                        FanOutLayout::WorktreeEach => FanOutLayout::Shared,
                    };
                }
            }
            KeyCode::Enter | KeyCode::Tab | KeyCode::Char('l') => {
                if let AppMode::CreatingFeature(state) = &mut app.mode {
                    state.step = CreateFeatureStep::Prompt;
                }
            }
            _ => {}
        },
        // Keys with modifiers are handled in `handle_create_feature_key_event`.
        CreateFeatureStep::Prompt => handle_prompt_step(app, KeyEvent::from(key))?,
        CreateFeatureStep::ConfirmSuperVibe => match key {
            KeyCode::Enter | KeyCode::Char('y') => {
                if let AppMode::CreatingFeature(state) = &mut app.mode {
                    state.advance_past_mode(); // FORK: §19.3
                }
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                if let AppMode::CreatingFeature(state) = &mut app.mode {
                    state.step = CreateFeatureStep::Mode;
                }
            }
            _ => {}
        },
    }
    Ok(())
}

/// FORK: §19.3 — the dialog's key handler with modifiers: the Prompt step
/// takes `Ctrl+L` (library), `Ctrl+T` (send when ready) and `Alt+Enter`
/// (newline); every other step only needs the key code.
pub fn handle_create_feature_key_event(app: &mut App, key: KeyEvent) -> Result<()> {
    let on_prompt = matches!(
        &app.mode,
        AppMode::CreatingFeature(state) if state.step == CreateFeatureStep::Prompt
    );
    if on_prompt {
        handle_prompt_step(app, key)
    } else {
        handle_create_feature_key(app, key.code)
    }
}

fn handle_prompt_step(app: &mut App, key: KeyEvent) -> Result<()> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);

    // The library picker, while open, takes every key.
    let picker_open = matches!(
        &app.mode,
        AppMode::CreatingFeature(state) if state.prompt_picker.is_some()
    );
    if picker_open {
        if let AppMode::CreatingFeature(state) = &mut app.mode
            && let Some(picker) = state.prompt_picker.as_mut()
        {
            let len = picker.entries.len();
            match key.code {
                KeyCode::Esc => state.prompt_picker = None,
                KeyCode::Down | KeyCode::Char('j') if len > 0 => {
                    picker.selected = (picker.selected + 1) % len;
                }
                KeyCode::Up | KeyCode::Char('k') if len > 0 => {
                    picker.selected = picker.selected.checked_sub(1).unwrap_or(len - 1);
                }
                KeyCode::Enter => {
                    if let Some((_, body)) = picker.entries.get(picker.selected).cloned() {
                        if !state.task_prompt.is_empty() && !state.task_prompt.ends_with('\n') {
                            state.task_prompt.push('\n');
                        }
                        state.task_prompt.push_str(&body);
                    }
                    state.prompt_picker = None;
                }
                _ => {}
            }
        }
        return Ok(());
    }

    match key.code {
        KeyCode::Esc => {
            if let AppMode::CreatingFeature(state) = &mut app.mode {
                state.step = state.step_before_prompt();
            }
        }
        KeyCode::Char('l') | KeyCode::Char('L') if ctrl => {
            let repo = match &app.mode {
                AppMode::CreatingFeature(state) => state.project_repo.clone(),
                _ => return Ok(()),
            };
            let entries = app.prompt_templates_for_repo(&repo);
            if entries.is_empty() {
                app.push_toast_info(
                    "The prompt library is empty — L on the dashboard adds templates",
                );
            } else if let AppMode::CreatingFeature(state) = &mut app.mode {
                state.prompt_picker = Some(FeaturePromptPicker {
                    entries,
                    selected: 0,
                });
            }
        }
        KeyCode::Char('t') | KeyCode::Char('T') if ctrl => {
            if let AppMode::CreatingFeature(state) = &mut app.mode {
                state.send_prompt = !state.send_prompt;
            }
        }
        KeyCode::Char('u') | KeyCode::Char('U') if ctrl => {
            if let AppMode::CreatingFeature(state) = &mut app.mode {
                state.task_prompt.clear();
            }
        }
        KeyCode::Char('j') | KeyCode::Char('J') if ctrl => {
            if let AppMode::CreatingFeature(state) = &mut app.mode {
                state.task_prompt.push('\n');
            }
        }
        KeyCode::Enter if alt || key.modifiers.contains(KeyModifiers::SHIFT) => {
            if let AppMode::CreatingFeature(state) = &mut app.mode {
                state.task_prompt.push('\n');
            }
        }
        KeyCode::Enter => {
            let previous_message = app.message.clone();
            app.create_feature_from_dialog()?;
            if matches!(app.mode, AppMode::CreatingFeature(_))
                && app.message != previous_message
                && let Some(message) = app.message.clone()
            {
                app.push_toast_warning(message);
            }
        }
        KeyCode::Backspace => {
            if let AppMode::CreatingFeature(state) = &mut app.mode {
                state.task_prompt.pop();
            }
        }
        KeyCode::Char(c) if !ctrl => {
            if let AppMode::CreatingFeature(state) = &mut app.mode {
                state.task_prompt.push(c);
            }
        }
        _ => {}
    }
    Ok(())
}
