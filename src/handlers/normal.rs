use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{App, AppMode, Selection};
use crate::project::{AgentKind, SessionKind};

pub fn handle_normal_key(app: &mut App, key: KeyEvent) -> Result<()> {
    // FORK: workspace layout — the layout toggle, the sidebar's own keys and,
    // with the pane focused, everything (it goes to the agent).
    if super::fork_workspace::handle_key(app, key)? {
        return Ok(());
    }
    // FORK: `a` / `X` answer a permission request waiting on the selected row (P6).
    if super::fork_permissions::handle_dashboard_key(app, key)? {
        return Ok(());
    }
    // FORK: `e` queues a follow-up for the selected agent session (P7).
    if super::fork_followups::handle_dashboard_key(app, key)? {
        return Ok(());
    }

    if app.leader_active {
        return handle_normal_leader_key(app, key);
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char(' ') {
        app.activate_leader();
        return Ok(());
    }

    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Down | KeyCode::Char('j') => {
                app.select_next_feature();
                app.message = None;
                return Ok(());
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.select_prev_feature();
                app.message = None;
                return Ok(());
            }
            // FORK: Ctrl+Q is "back to the dashboard" in every view, so one
            // press too many used to fall through to the plain `q` arm below
            // and quit AIW without confirmation. On the dashboard it is a
            // no-op; `q` still quits.
            KeyCode::Char('q') => return Ok(()),
            _ => {}
        }
    }

    // Apply keybinding remaps from extension config.
    // Build a map from the user-defined key → the canonical
    // action string, then convert pressed char → canonical
    // char using default_key_for_action().
    let raw_key = key.code;
    let remapped_key = if let KeyCode::Char(c) = raw_key {
        let bindings = &app.active_extension.keybindings;
        let canonical_char = bindings
            .iter()
            .find(|&(_, &v)| v == c)
            .and_then(|(action, _)| default_key_for_action(action));
        if let Some(canonical) = canonical_char {
            KeyCode::Char(canonical)
        } else {
            raw_key
        }
    } else {
        raw_key
    };
    let key = remapped_key;
    match key {
        KeyCode::Char('q') => {
            // FORK: asks first when `confirm_quit` is on and agents run.
            crate::fork::quit_confirm::request_quit(app);
        }
        // FORK: Esc used to quit like `q`, so one Esc too many after closing
        // a dialog ended the session. On the dashboard it only takes back a
        // pending quit confirmation.
        KeyCode::Esc => {
            crate::fork::quit_confirm::cancel(app);
        }
        KeyCode::Char('N') => {
            app.start_create_project();
        }
        KeyCode::Char('B') => {
            app.start_create_batch_features();
        }
        KeyCode::Char('O') => {
            app.open_settings_project()?;
        }
        KeyCode::Char('A') => {
            app.open_harness_setup(false);
        }
        KeyCode::Char('n') => {
            if app.selected_project().is_some() {
                app.start_create_feature();
            }
        }
        KeyCode::Char('g') => {
            if let Selection::Project(project_index) = app.selection {
                app.open_issue_browser_for_project(project_index);
            }
        }
        KeyCode::Enter => match &app.selection {
            Selection::Project(_) => {
                app.toggle_collapse();
            }
            Selection::Feature(_, _) => {
                app.toggle_collapse();
            }
            Selection::Session(_, _, _) => {
                if !app.open_stopped_session_dialog()? {
                    app.enter_view()?;
                }
            }
        },
        KeyCode::Char('c') => match &app.selection {
            Selection::Feature(_, _) | Selection::Session(_, _, _) => {
                app.start_feature()?;
            }
            _ => {}
        },
        // FORK: split view — tile the selected feature's (or project's) agents.
        KeyCode::Char('v') => app.cockpit_show_grid(),
        KeyCode::Char('x') => match &app.selection {
            Selection::Session(_, _, _) => {
                app.stop_session()?;
            }
            Selection::Feature(_, _) => {
                app.stop_feature()?;
            }
            _ => {}
        },
        KeyCode::Char('d') => match &app.selection {
            Selection::Project(pi) => {
                if let Some(project) = app.store.projects.get(*pi) {
                    let name = project.name.clone();
                    app.mode = AppMode::DeletingProject(name);
                }
            }
            Selection::Feature(pi, fi) => {
                if let Some(project) = app.store.projects.get(*pi)
                    && let Some(feature) = project.features.get(*fi)
                {
                    let pn = project.name.clone();
                    let fn_ = feature.name.clone();
                    app.mode = AppMode::DeletingFeature(pn, fn_);
                }
            }
            Selection::Session(_, _, _) => {
                app.remove_session()?;
            }
        },
        KeyCode::Char('s') => {
            app.open_session_picker()?;
        }
        KeyCode::Char('S') => {
            // `S` stays on the saved-transcript picker even for a stopped
            // feature: the pickers start the feature themselves, and they are
            // the only way to reach transcripts older than the ID AIW recorded.
            let (is_claude, is_codex) = match &app.selection {
                Selection::Feature(pi, fi) => {
                    let agent = app
                        .store
                        .projects
                        .get(*pi)
                        .and_then(|p| p.features.get(*fi))
                        .map(|f| f.agent.clone());
                    (
                        agent.as_ref().map(|a| *a == AgentKind::Claude),
                        agent.as_ref().map(|a| *a == AgentKind::Codex),
                    )
                }
                Selection::Session(pi, fi, si) => {
                    let kind = app
                        .store
                        .projects
                        .get(*pi)
                        .and_then(|p| p.features.get(*fi))
                        .and_then(|f| f.sessions.get(*si))
                        .map(|s| s.kind.clone());
                    (
                        kind.as_ref().map(|k| *k == SessionKind::Claude),
                        kind.as_ref().map(|k| *k == SessionKind::Codex),
                    )
                }
                _ => (Some(false), Some(false)),
            };

            if is_claude.unwrap_or(false) {
                app.pick_claude_session();
            } else if is_codex.unwrap_or(false) {
                app.pick_codex_session();
            } else {
                app.push_toast_warning("S only works for claude or codex sessions");
            }
        }
        KeyCode::Char('u') => match &app.selection {
            Selection::Project(_) => {
                app.start_project_agent_config()?;
            }
            Selection::Feature(_, _) | Selection::Session(_, _, _) => {
                app.start_session_config()?;
            }
        },
        KeyCode::Char('y') => match &app.selection {
            Selection::Feature(_, _) | Selection::Session(_, _, _) => {
                app.toggle_feature_ready()?;
            }
            _ => {}
        },
        KeyCode::Char('h') | KeyCode::Left => match &app.selection {
            Selection::Project(pi) => {
                if let Some(project) = app.store.projects.get(*pi)
                    && !project.collapsed
                {
                    app.toggle_collapse();
                }
            }
            Selection::Feature(pi, fi) => {
                if let Some(feature) = app
                    .store
                    .projects
                    .get(*pi)
                    .and_then(|p| p.features.get(*fi))
                    && !feature.collapsed
                {
                    app.toggle_collapse();
                }
            }
            Selection::Session(_, _, _) => {
                app.toggle_collapse();
            }
        },
        KeyCode::Char('l') | KeyCode::Right => match &app.selection {
            Selection::Project(pi) => {
                if let Some(project) = app.store.projects.get(*pi)
                    && project.collapsed
                {
                    app.toggle_collapse();
                }
            }
            Selection::Feature(pi, fi) => {
                if let Some(feature) = app
                    .store
                    .projects
                    .get(*pi)
                    .and_then(|p| p.features.get(*fi))
                    && feature.collapsed
                {
                    app.toggle_collapse();
                }
            }
            Selection::Session(_, _, _) => {}
        },
        KeyCode::Char('?') => {
            app.mode = AppMode::Help(crate::app::HelpState {
                from_view: None,
                scroll_offset: 0,
            });
        }
        KeyCode::Char('/') => {
            app.start_search();
        }
        // Only on a TODOs session row: the key means "take the next item off
        // *this* list", and a project without a list has no row to press it on.
        // Inert everywhere else rather than guessing which list was meant.
        KeyCode::Char('I') => {
            app.implement_next_todo_from_dashboard()?;
        }
        KeyCode::Char('i') => {
            if app.attention_rows().is_empty() {
                // FORK: a footer line, worded like the header pills.
                app.message = Some("Nothing needs you".to_string());
            } else {
                app.mode = AppMode::NotificationPicker(0, None);
            }
        }
        KeyCode::Char('r') => match &app.selection {
            Selection::Session(_, _, _) => {
                app.start_rename_session();
            }
            Selection::Feature(_, _) => {
                app.start_rename_feature();
            }
            _ => {
                app.refresh_status_and_notifications();
            }
        },
        KeyCode::Char('R') => {
            app.refresh_status_and_notifications();
        }
        KeyCode::Char('V') => {
            app.check_pending_diff_review()?;
        }
        KeyCode::Down | KeyCode::Char('j') => {
            app.select_next();
            app.message = None;
        }
        KeyCode::Up | KeyCode::Char('k') => {
            app.select_prev();
            app.message = None;
        }
        KeyCode::Char('F') => {
            if matches!(
                app.selection,
                Selection::Feature(_, _) | Selection::Session(_, _, _)
            ) {
                app.start_fork_feature();
            }
        }
        KeyCode::Char('T') => {
            app.start_theme_picker();
        }
        KeyCode::Char('p') => {
            app.start_syntax_language_picker();
        }
        KeyCode::Char('L') => {
            app.open_prompt_library(None);
        }
        KeyCode::Char('f') if crate::app::DASHBOARD_SESSION_FILTER_ENABLED => {
            app.session_filter = app.session_filter.next();
            app.push_toast_info(format!("Filter: {}", app.session_filter.display_name()));
        }
        // FORK: §19.2 — fork the selected session's conversation.
        KeyCode::Char('f') => {
            app.start_fork_session();
        }
        // FORK: §19.2 — Claude sessions outside AIW in the projects.
        KeyCode::Char('E') => {
            app.open_external_sessions();
        }
        // `z` for zzz: features nobody is watching.
        KeyCode::Char('z') => {
            app.open_dormant_view();
        }
        KeyCode::Char('D') => {
            app.open_debug_log(None);
        }
        KeyCode::Char('w') => {
            app.start_context_settings();
        }
        _ => {}
    }
    Ok(())
}

pub(crate) const DASHBOARD_KEYBINDING_ACTIONS: &[(&str, char)] = &[
    ("quit", 'q'),
    ("create_project", 'N'),
    ("create_feature", 'n'),
    ("issue_fixer", 'g'),
    ("start_session", 'c'),
    ("stop_session", 'x'),
    ("delete", 'd'),
    ("sessions", 's'),
    ("help", '?'),
    ("search", '/'),
    ("refresh", 'r'),
    ("syntax_picker", 'p'),
    ("session_config", 'u'),
    ("fork_feature", 'F'),
    ("mark_ready", 'y'),
];

/// Configurable actions that have no default key. These are only active when
/// the user binds a key to them in the config; they are always offered in the
/// config wizard so they can be discovered and bound.
pub(crate) const EXTRA_KEYBINDING_ACTIONS: &[&str] = &["next_feature", "prev_feature"];

/// Returns the default canonical key character for a named action.
/// These correspond to the hardcoded keys in handle_normal_key().
pub(crate) fn default_key_for_action(action: &str) -> Option<char> {
    DASHBOARD_KEYBINDING_ACTIONS
        .iter()
        .find_map(|(known_action, key)| (*known_action == action).then_some(*key))
}

fn handle_normal_leader_key(app: &mut App, key: KeyEvent) -> Result<()> {
    app.deactivate_leader();

    match key.code {
        // FORK(cockpit): the commands the embedded view's leader had, for
        // the selected session (DESIGN.md §18). `i` and `?` are plain
        // dashboard keys and left the menu.
        KeyCode::Char('d') => {
            app.open_diff_viewer()?;
        }
        KeyCode::Char('m') => {
            app.open_markdown_viewer_for_selection()?;
        }
        KeyCode::Char('l') => {
            app.open_latest_prompt_for_selection();
        }
        KeyCode::Char('N') => {
            app.open_todo_quick_capture();
        }
        KeyCode::Char('z') => {
            app.request_todo_reference_completion();
        }
        KeyCode::Char('/') => {
            app.open_command_picker(None);
        }
        KeyCode::Char('a') => {
            app.open_command_picker_with_focus(None, crate::app::CommandPickerFocus::Local);
        }
        KeyCode::Char('c') => {
            app.start_config_wizard();
        }
        KeyCode::Char('h') => {
            app.open_bookmark_picker(None);
        }
        KeyCode::Char('H') => {
            app.bookmark_current_session()?;
        }
        KeyCode::Char('M') => {
            app.unbookmark_current_session()?;
        }
        KeyCode::Char(c @ '1'..='9') => {
            let slot = (c as u8 - b'0') as usize;
            app.jump_to_bookmark(slot)?;
        }
        KeyCode::Char('r') => {
            app.refresh_status_and_notifications();
        }
        // FORK(no-cloud): leader+C (AIW Remote server) and leader+Q (phone
        // pairing) were removed with Remote Control.
        _ => {}
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // FORK: Esc on the dashboard is a no-op (one Esc too many after a dialog
    // used to end the session); `q` still quits as the README says.
    #[test]
    fn esc_on_the_dashboard_does_not_quit() {
        use crate::traits::{MockTmuxOps, MockWorktreeOps};
        let mut app = App::new_for_test(
            crate::project::ProjectStore::empty(),
            Box::new(MockTmuxOps::new()),
            Box::new(MockWorktreeOps::new()),
        );
        handle_normal_key(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)).unwrap();
        assert!(!app.should_quit);
        assert!(matches!(app.mode, AppMode::Normal));
        handle_normal_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        )
        .unwrap();
        assert!(app.should_quit);
    }

    // FORK: a stray Ctrl+Q (the "leave the view" key) on the dashboard must
    // not quit; only `q` does.
    #[test]
    fn ctrl_q_on_the_dashboard_does_not_quit() {
        use crate::traits::{MockTmuxOps, MockWorktreeOps};
        let mut app = App::new_for_test(
            crate::project::ProjectStore::empty(),
            Box::new(MockTmuxOps::new()),
            Box::new(MockWorktreeOps::new()),
        );
        handle_normal_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL),
        )
        .unwrap();
        assert!(!app.should_quit);
        handle_normal_key(
            &mut app,
            KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE),
        )
        .unwrap();
        assert!(app.should_quit);
    }

    // ── default_key_for_action ────────────────────────────────

    #[test]
    fn default_key_all_known_actions() {
        assert_eq!(default_key_for_action("quit"), Some('q'));
        assert_eq!(default_key_for_action("create_project"), Some('N'));
        assert_eq!(default_key_for_action("create_feature"), Some('n'));
        assert_eq!(default_key_for_action("issue_fixer"), Some('g'));
        assert_eq!(default_key_for_action("start_session"), Some('c'));
        assert_eq!(default_key_for_action("stop_session"), Some('x'));
        assert_eq!(default_key_for_action("delete"), Some('d'));
        assert_eq!(default_key_for_action("sessions"), Some('s'));
        assert_eq!(default_key_for_action("help"), Some('?'));
        assert_eq!(default_key_for_action("search"), Some('/'));
        assert_eq!(default_key_for_action("refresh"), Some('r'));
        assert_eq!(default_key_for_action("filter"), None);
        assert_eq!(default_key_for_action("syntax_picker"), Some('p'));
        assert_eq!(default_key_for_action("session_config"), Some('u'));
        assert_eq!(default_key_for_action("mark_ready"), Some('y'));
    }

    /// Two actions sharing a default key would make one of them unreachable,
    /// and the remap lookup in `handle_normal_key` resolves a pressed key to
    /// the *first* matching action — so a collision silently steals a binding.
    #[test]
    fn default_keys_are_unique_across_actions() {
        let mut seen: Vec<char> = Vec::new();
        for (action, key) in DASHBOARD_KEYBINDING_ACTIONS {
            assert!(
                !seen.contains(key),
                "{action} reuses the default key {key:?}"
            );
            seen.push(*key);
        }
    }

    #[test]
    fn default_key_unknown_action_returns_none() {
        assert_eq!(default_key_for_action("unknown_action"), None);
        assert_eq!(default_key_for_action(""), None);
        assert_eq!(default_key_for_action("QUIT"), None);
    }
}
