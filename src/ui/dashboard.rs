use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
};

use crate::app::{App, AppMode, CreateFeatureStep};
use crate::project::VibeMode;

// FORK(cockpit): the selected session's commands (DESIGN.md §18) — the
// embedded view's leader menu moved here when the view went.
const DASHBOARD_LEADER_COMMANDS: &[(&str, &str)] = &[
    ("f", "Final review of the selected feature"),
    ("d", "Diff viewer"),
    ("n", "Current plan"),
    ("m", "Markdown files"),
    ("l", "Past / unsent prompts (paste again)"),
    ("N", "Quick-capture a TODO"),
    ("F", "Fresh-context session"),
    ("P", "Back to PR Triage (after P there)"),
    ("B", "Model advice for the selected session"),
    ("z", "Complete the TODO this session came from"),
    ("/", "Command picker"),
    ("a", "Local commands"),
    ("c", "Config wizard"),
    ("C", "Compact review memory"),
    ("h", "Bookmarks"),
    ("H", "Bookmark current session"),
    ("M", "Remove bookmark"),
    ("1-9", "Jump to bookmark"),
    ("r", "Refresh statuses + PR badges"),
];

/// FORK(cockpit): the dashboard itself — header, sidebar, footer. Drawn
/// under the session dialogs too, where the embedded view used to be.
fn draw_dashboard_chrome(frame: &mut Frame, app: &mut App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
        ])
        .split(frame.area());

    // FORK: G11 status pills from the badge model; cwd read once.
    let pills = super::fork_chrome::header_pills(app);
    super::header::draw(
        frame,
        chunks[0],
        super::fork_chrome::cwd_label(),
        env!("CARGO_PKG_VERSION"),
        app.attention_badge_count(),
        pills,
        &app.theme,
    );
    // FORK: the cockpit sidebar; the empty-dashboard note when there are
    // no projects.
    if !super::fork_workspace::draw(frame, app, chunks[1]) {
        super::list::draw(frame, app, chunks[1]);
    }
    super::status::draw(frame, app, chunks[2]);
}

fn draw_view_context_bar(
    frame: &mut Frame,
    view: &crate::app::ViewState,
    theme: &crate::theme::Theme,
) {
    draw_context_bar(
        frame,
        &view.project_name,
        &view.feature_name,
        Some(&view.session_label),
        Some(&view.vibe_mode),
        view.review,
        theme,
    );
}

fn draw_feature_context_bar(
    frame: &mut Frame,
    project_name: &str,
    feature_name: &str,
    theme: &crate::theme::Theme,
) {
    draw_context_bar(frame, project_name, feature_name, None, None, false, theme);
}

fn draw_mode_context_bar(frame: &mut Frame, mode: &AppMode, theme: &crate::theme::Theme) {
    if let Some(view) = mode_view_context(mode) {
        draw_view_context_bar(frame, view, theme);
    }
}

fn mode_view_context(mode: &AppMode) -> Option<&crate::app::ViewState> {
    match mode {
        AppMode::Help(state) => state.from_view.as_ref(),
        AppMode::NotificationPicker(_, from_view) => from_view.as_ref(),
        AppMode::CommandPicker(state) => state.from_view.as_ref(),
        AppMode::BookmarkPicker(state) => state.from_view.as_ref(),
        AppMode::DiffPicker(state) => Some(&state.from_view),
        AppMode::DiffViewerLoading(state) | AppMode::DiffViewer(state) => Some(&state.from_view),
        AppMode::TodoQuickCapture(state) => Some(&state.view),
        AppMode::FreshContextPrompt(state) => Some(&state.view),
        AppMode::SessionPicker(state) => state.from_view.as_ref(),
        AppMode::DiffReviewPrompt(state) => state.return_to_view.as_ref(),
        AppMode::LatestPrompt(state) => Some(&state.view),
        AppMode::PromptLibrary(state) => state.from_view.as_ref(),
        AppMode::PromptOverrides(state) => state.from_view.as_ref(),
        AppMode::PromptPrecall(pending) => mode_view_context(pending.prior_mode.as_ref()),
        AppMode::PromptEditor(state) => mode_view_context(state.return_to.as_ref()),
        AppMode::PlaceholderFill(state) => state.from_view.as_ref(),
        AppMode::SkillPicker(state) => mode_view_context(state.return_to.as_ref()),
        AppMode::DebugLog(state) => state.from_view.as_ref(),
        AppMode::MarkdownLoading(state) => state.from_view.as_ref(),
        AppMode::MarkdownViewer(state) => state.from_view.as_ref(),
        AppMode::MarkdownFilePicker(state) => state.from_view.as_ref(),
        _ => None,
    }
}

fn draw_context_bar(
    frame: &mut Frame,
    project_name: &str,
    feature_name: &str,
    session_label: Option<&str>,
    vibe_mode: Option<&VibeMode>,
    review: bool,
    theme: &crate::theme::Theme,
) {
    let viewport = frame.area();
    if viewport.width == 0 || viewport.height == 0 {
        return;
    }

    let area = Rect::new(viewport.x, viewport.y, viewport.width, 1);
    frame.render_widget(Clear, area);
    frame.render_widget(
        Block::default().style(Style::default().bg(theme.effective_bg())),
        area,
    );

    let mut spans = vec![Span::raw("  ")];
    spans.push(Span::styled(
        project_name,
        Style::default()
            .fg(theme.project_title.to_color())
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled(
        format!(" / {feature_name}"),
        Style::default()
            .fg(theme.warning.to_color())
            .add_modifier(Modifier::BOLD),
    ));
    if let Some(session_label) = session_label {
        spans.push(Span::styled(
            format!(" / {session_label}"),
            Style::default().fg(theme.text.to_color()),
        ));
    }
    if let Some(vibe_mode) = vibe_mode {
        let (label, color) = match vibe_mode {
            VibeMode::Vibeless => (" [vibeless]", theme.mode_vibeless.to_color()),
            VibeMode::Vibe => (" [vibe]", theme.mode_vibe.to_color()),
            VibeMode::SuperVibe => (" [supervibe]", theme.mode_supervibe.to_color()),
        };
        spans.push(Span::styled(label, Style::default().fg(color)));
    }
    if review {
        spans.push(Span::styled(
            " [review]",
            Style::default().fg(theme.mode_review.to_color()),
        ));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

pub fn draw(frame: &mut Frame, app: &mut App) {
    // FORK: no full-screen `Clear` — `Terminal::draw` hands us a freshly reset
    // buffer, so it only cost a pass over every cell (~6% of a frame).
    frame.render_widget(
        Block::default().style(Style::default().bg(app.theme.effective_bg())),
        frame.area(),
    );

    match &app.mode {
        AppMode::IssueBrowser(state) => {
            super::dialogs::draw_issue_browser(frame, state, &app.theme);
            super::draw_toasts(frame, frame.area(), &app.toasts, &app.theme);
            return;
        }
        AppMode::IssueSetup(state) => {
            super::dialogs::draw_issue_setup(frame, state, &app.theme);
            super::draw_toasts(frame, frame.area(), &app.toasts, &app.theme);
            return;
        }
        AppMode::IssueDuplicateWarning(state) => {
            super::dialogs::draw_issue_duplicate_warning(frame, state, &app.theme);
            super::draw_toasts(frame, frame.area(), &app.toasts, &app.theme);
            return;
        }
        _ => {}
    }

    // The PR-review family of full-screen modes (loading/picker/pane/running
    // views) each `return` before reaching the shared `draw_toasts` call near
    // the end of this function, so every one of them draws its own — a
    // success/warning/error toast pushed while any of these is showing
    // (e.g. the AI review's "found N findings" / error result landing while
    // the user is back in the pane) would otherwise never appear, silently
    // swallowed until the mode changes.
    if let AppMode::PrReviewLoading(state) = &app.mode {
        super::dialogs::draw_pr_review_loading(frame, state, &app.throbber_state, &app.theme);
        super::draw_toasts(frame, frame.area(), &app.toasts, &app.theme);
        return;
    }
    if let AppMode::PrInvestigationLoading(state) = &app.mode {
        super::dialogs::draw_pr_investigation_loading(
            frame,
            state,
            &app.throbber_state,
            &app.theme,
        );
        super::draw_toasts(frame, frame.area(), &app.toasts, &app.theme);
        return;
    }
    if matches!(app.mode, AppMode::PrReview(_)) {
        let fix_session_usage = app.pr_review_fix_session_usage();
        let triage_session_usage = app.pr_review_triage_session_usage();
        let dedicated_session_working = app.pr_review_dedicated_session_working();
        // Resolved here, before the `&mut app.mode` borrow: the companion
        // feature's name/harness/mode live on the store, not the pane state.
        let triage_feature_summary = app.pr_review_triage_feature_summary();
        let ai_review_status = match &app.mode {
            AppMode::PrReview(state) => app.ai_review_triage_status(state),
            _ => crate::app::ai_review::AiReviewTriageStatus::NotRun,
        };
        // Same pattern: resolved off `app` before the `&mut app.mode` borrow —
        // but only while the "add to memory" dialog is actually open, since
        // `repo_for_project_path` shells out to git and this runs every frame.
        let memory_paths = match &app.mode {
            AppMode::PrReview(state) if state.memory_add.is_some() => {
                let repo = app.repo_for_project_path(&state.workdir);
                Some(app.review_memory_paths(&repo))
            }
            _ => None,
        };
        if let AppMode::PrReview(state) = &mut app.mode {
            super::dialogs::draw_pr_review(
                frame,
                state,
                &app.theme,
                super::dialogs::PrReviewUsage {
                    cumulative: fix_session_usage.as_ref(),
                    visit: triage_session_usage.as_ref(),
                    pricing: &app.config.token_pricing,
                },
                dedicated_session_working,
                &ai_review_status,
                triage_feature_summary.as_deref(),
                memory_paths.as_ref(),
            );
        }
        super::draw_toasts(frame, frame.area(), &app.toasts, &app.theme);
        return;
    }
    if matches!(app.mode, AppMode::AiReview(_)) {
        let ai_review_running = app.ai_review_run.is_pending();
        let finding_fix_costs = app.ai_review_finding_fix_costs();
        if let AppMode::AiReview(state) = &mut app.mode {
            super::dialogs::draw_ai_review(
                frame,
                state,
                &app.theme,
                ai_review_running,
                &app.throbber_state,
                &finding_fix_costs,
            );
        }
        super::draw_toasts(frame, frame.area(), &app.toasts, &app.theme);
        return;
    }
    if let AppMode::PrPicker(state) = &app.mode {
        // FORK: memoised — this ran `git rev-parse` + config reads per frame.
        let memory_paths = app.render_review_memory_paths(&state.workdir);
        super::dialogs::draw_pr_picker(frame, state, &app.theme, &memory_paths);
        super::draw_toasts(frame, frame.area(), &app.toasts, &app.theme);
        return;
    }
    if let AppMode::PrReviewList(state) = &app.mode {
        super::dialogs::draw_pr_review_list(frame, state, &app.theme);
        super::draw_toasts(frame, frame.area(), &app.toasts, &app.theme);
        return;
    }
    if let AppMode::ReviewMemoryBootstrapRunning(state) = &app.mode {
        super::dialogs::draw_review_memory_bootstrap_running(
            frame,
            state,
            &app.throbber_state,
            &app.theme,
        );
        super::draw_toasts(frame, frame.area(), &app.toasts, &app.theme);
        return;
    }
    if let AppMode::ReviewMemoryCompactRunning(state) = &app.mode {
        super::dialogs::draw_review_memory_compact_running(
            frame,
            state,
            &app.throbber_state,
            &app.theme,
        );
        super::draw_toasts(frame, frame.area(), &app.toasts, &app.theme);
        return;
    }
    if matches!(app.mode, AppMode::ReviewMemoryCompactReview(_)) {
        if let AppMode::ReviewMemoryCompactReview(state) = &mut app.mode {
            super::dialogs::draw_review_memory_compact_review(frame, state, &app.theme);
        }
        super::draw_toasts(frame, frame.area(), &app.toasts, &app.theme);
        return;
    }
    if let AppMode::AiReviewRunning(state) = &app.mode {
        super::dialogs::draw_ai_review_running(frame, state, &app.throbber_state, &app.theme);
        super::draw_toasts(frame, frame.area(), &app.toasts, &app.theme);
        return;
    }

    if let AppMode::Todos(state) = &app.mode {
        super::dialogs::draw_todos_view_with_visibility(
            frame,
            state,
            &app.theme,
            app.config.nerd_font,
            app.todo_project_visible,
            app.todo_global_visible,
        );
        super::draw_toasts(frame, frame.area(), &app.toasts, &app.theme);
        return;
    }

    // FORK(cockpit): the dialogs that used to open over the embedded view
    // open over the dashboard; it is drawn first, the dialog on top.
    let over_dashboard = matches!(
        app.mode,
        AppMode::LatestPrompt(_)
            | AppMode::MarkdownFilePicker(_)
            | AppMode::DiffPicker(_)
            | AppMode::DiffViewer(_)
            | AppMode::DiffViewerLoading(_)
            | AppMode::MarkdownLoading(_)
            | AppMode::MarkdownViewer(_)
            | AppMode::TodoQuickCapture(_)
            | AppMode::FreshContextPrompt(_)
    );
    if over_dashboard {
        draw_dashboard_chrome(frame, app);
    }

    if let AppMode::LatestPrompt(state) = &app.mode {
        super::dialogs::draw_latest_prompt_dialog(frame, state, app.message.as_deref(), &app.theme);
        draw_mode_context_bar(frame, &app.mode, &app.theme);
        return;
    }

    if let AppMode::DiffPicker(state) = &app.mode {
        super::dialogs::draw_diff_picker(frame, state, &app.theme);
        draw_mode_context_bar(frame, &app.mode, &app.theme);
        return;
    }

    if let AppMode::DiffViewer(state) = &mut app.mode {
        super::dialogs::draw_diff_viewer(frame, state, &app.theme);
        draw_mode_context_bar(frame, &app.mode, &app.theme);
        return;
    }

    if let AppMode::DiffViewerLoading(state) = &app.mode {
        super::dialogs::draw_diff_viewer_loading(frame, state, &app.throbber_state, &app.theme);
        draw_mode_context_bar(frame, &app.mode, &app.theme);
        return;
    }

    if let AppMode::MarkdownLoading(state) = &app.mode {
        super::dialogs::draw_markdown_loading(frame, state, &app.throbber_state, &app.theme);
        draw_mode_context_bar(frame, &app.mode, &app.theme);
        return;
    }

    if let AppMode::MarkdownViewer(state) = &mut app.mode {
        super::dialogs::draw_markdown_viewer(frame, state, &app.theme);
        draw_mode_context_bar(frame, &app.mode, &app.theme);
        return;
    }

    if let AppMode::TodoQuickCapture(state) = &app.mode {
        super::dialogs::draw_todo_quick_capture_dialog(frame, state, &app.theme);
        draw_mode_context_bar(frame, &app.mode, &app.theme);
        return;
    }

    if let AppMode::FreshContextPrompt(state) = &app.mode {
        super::dialogs::draw_fresh_context_prompt_dialog(frame, state, &app.theme);
        draw_mode_context_bar(frame, &app.mode, &app.theme);
        return;
    }

    if let AppMode::SyntaxLanguagePicker(state) = &app.mode {
        super::picker::draw_syntax_language_picker(frame, state, &app.throbber_state, &app.theme);
        return;
    }

    if let AppMode::MarkdownFilePicker(state) = &app.mode {
        super::picker::draw_markdown_file_picker(frame, state, &app.theme);
        draw_mode_context_bar(frame, &app.mode, &app.theme);
        return;
    }

    draw_dashboard_chrome(frame, app);

    if app.leader_active {
        super::pane::draw_command_menu(
            frame,
            frame.area(),
            " Ctrl+Space commands ",
            DASHBOARD_LEADER_COMMANDS,
            &app.theme,
        );
    }

    if let AppMode::PlanInterview(state) = &mut app.mode {
        super::dialogs::draw_plan_interview_dialog(
            frame,
            state,
            app.message.as_deref(),
            &app.theme,
            &app.throbber_state,
        );
        return;
    }

    match &app.mode {
        AppMode::CreatingProject(state) => {
            let allowed_agents =
                // FORK: memoised — this ran `git rev-parse` + config reads per frame.
                app.render_allowed_agents_for_project_path(std::path::Path::new(&state.path));
            super::dialogs::draw_create_project_dialog(
                frame,
                state,
                allowed_agents.as_slice(),
                app.message.as_deref(),
                &app.theme,
            );
        }
        AppMode::CreatingFeature(state) => {
            if state.step == CreateFeatureStep::ConfirmSuperVibe {
                super::dialogs::draw_confirm_supervibe_dialog(frame, &app.theme);
            } else {
                let usage = app.usage.get_data();
                super::dialogs::draw_create_feature_dialog(
                    frame,
                    state,
                    state.feature_presets.as_slice(),
                    state.allowed_agents.as_slice(),
                    &usage,
                    &app.theme,
                );
            }
        }
        AppMode::ConfirmResourceStart(state) => {
            super::dialogs::draw_resource_confirm_dialog(frame, state, &app.theme);
        }
        AppMode::Dormant(state) => {
            super::dialogs::draw_dormant_view(
                frame,
                state,
                app.config.dormant_idle_minutes,
                app.config.dormant_last_accessed_hours,
                &app.theme,
            );
        }
        AppMode::PlanInterview(_) => unreachable!("plan interview handled above"),
        AppMode::CreatingBatchFeatures(state) => {
            super::dialogs::draw_create_batch_features_dialog(frame, state, &app.theme);
        }
        AppMode::DeletingProject(name) => {
            super::dialogs::draw_delete_project_confirm(frame, name, &app.theme);
        }
        AppMode::DeletingFeature(project_name, feature_name) => {
            super::dialogs::draw_delete_feature_confirm(
                frame,
                project_name,
                feature_name,
                &app.theme,
            );
        }
        AppMode::BrowsingPath(state) => {
            super::dialogs::draw_browse_path_dialog(frame, state, &app.theme);
        }
        AppMode::PlanInterviewAttachDoc(state) => {
            super::dialogs::draw_plan_interview_attach_doc_dialog(frame, state, &app.theme);
        }
        _ => {}
    }

    if let AppMode::PrNumberPrompt(state) = &app.mode {
        super::dialogs::draw_pr_number_prompt(frame, state, &app.theme);
    }

    if let AppMode::TodosHostReassign(state) = &app.mode {
        super::dialogs::draw_todos_host_reassign_dialog(frame, state, &app.theme);
    }

    // Drawn the same way from either surface: `draw_modal_overlay` clears the
    // viewport, so what the key was pressed in never shows through.
    if let AppMode::TodoImplementChoice(state) = &app.mode {
        super::dialogs::draw_todo_implement_choice_dialog(frame, state, &app.theme);
    }

    if let AppMode::TodoSpawnTarget(state) = &app.mode {
        super::dialogs::draw_todo_spawn_target_dialog(frame, state, &app.theme);
    }

    if let AppMode::TodoDeleteDisposition(state) = &app.mode {
        super::dialogs::draw_todo_delete_disposition_dialog(frame, state, &app.theme);
    }

    if let AppMode::ConfirmTodoReferenceCompletion(state) = &app.mode {
        super::dialogs::draw_todo_reference_completion_dialog(frame, state, &app.theme);
    }

    if let AppMode::RenamingSession(state) = &app.mode {
        super::dialogs::draw_rename_session_dialog(frame, state, &app.theme);
    }

    if let AppMode::RenamingFeature(state) = &app.mode {
        super::dialogs::draw_rename_feature_dialog(frame, state, &app.theme);
    }

    if let AppMode::SessionConfig(state) = &app.mode {
        super::dialogs::draw_session_config_dialog(frame, state, &app.theme);
    }

    if let AppMode::ProjectAgentConfig(state) = &app.mode {
        super::dialogs::draw_project_agent_config_dialog(frame, state, &app.theme);
    }

    if let AppMode::Help(state) = &app.mode
        && state.from_view.is_none()
    {
        let scroll = state.scroll_offset;
        let scroll = super::dialogs::draw_help(frame, scroll, &app.theme);
        if let AppMode::Help(state) = &mut app.mode {
            state.scroll_offset = scroll;
        }
    }

    if let AppMode::NotificationPicker(selected, None) = &app.mode {
        // FORK: rows carry the badge of the feature they belong to.
        let attention_rows = app.attention_rows();
        let badges = super::picker::row_badges(app, &attention_rows);
        super::picker::draw_notification_picker(
            frame,
            &attention_rows,
            &app.pending_inputs,
            &badges,
            *selected,
            app.config.nerd_font,
            &app.theme,
        );
    }

    if let AppMode::CommandPicker(state) = &app.mode {
        super::picker::draw_command_picker(frame, state, &app.theme);
    }

    if let AppMode::Searching(state) = &app.mode {
        super::dialogs::draw_search_dialog(frame, state, &app.theme);
    }

    if let AppMode::ClaudeSessionPicker(state) = &app.mode {
        super::picker::draw_claude_session_picker(frame, state, &app.theme);
    }

    if matches!(app.mode, AppMode::ConfirmingClaudeSession { .. }) {
        super::picker::draw_claude_session_confirm(frame, &app.theme);
    }

    if let AppMode::CodexSessionPicker(state) = &app.mode {
        super::picker::draw_codex_session_picker(frame, state, &app.theme);
    }

    if matches!(app.mode, AppMode::ConfirmingCodexSession { .. }) {
        super::picker::draw_codex_session_confirm(frame, &app.theme);
    }

    if let AppMode::StoppedSessionDialog(state) = &app.mode {
        super::dialogs::draw_stopped_session_dialog(frame, state, &app.theme);
    }

    if let AppMode::SessionPicker(state) = &app.mode {
        let usage = app.usage.get_data();
        super::picker::draw_session_picker(frame, state, &usage, app.config.nerd_font, &app.theme);
    }

    if let AppMode::NamingNewSession(state) = &app.mode {
        super::dialogs::draw_new_session_name_dialog(frame, state, &app.theme);
    }

    if let AppMode::BookmarkPicker(state) = &app.mode {
        let rows = app.bookmark_picker_rows();
        super::picker::draw_bookmark_picker(frame, state, &rows, &app.theme);
    }

    if let AppMode::DiffReviewPrompt(state) = &app.mode {
        super::dialogs::draw_diff_review_dialog(frame, state, &app.throbber_state, &app.theme);
    }

    if let AppMode::RunningHook(state) = &app.mode {
        super::dialogs::draw_running_hook_dialog(frame, state, &app.throbber_state, &app.theme);
    }

    if let AppMode::DeletingFeatureInProgress(state) = &app.mode {
        super::dialogs::draw_deleting_feature_dialog(frame, state, &app.throbber_state, &app.theme);
    }

    if let AppMode::HookPrompt(state) = &app.mode {
        super::dialogs::draw_hook_prompt_dialog(frame, state, &app.theme);
    }

    // FORK: permission detail overlay (P6).
    if let AppMode::PermissionDetail(state) = &app.mode {
        super::fork_permissions::draw_detail(frame, app, state);
    }

    // FORK: the follow-up queue dialog (P7).
    if let AppMode::FollowupQueue(state) = &mut app.mode {
        let queue = app
            .followups
            .get(&state.session_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        super::dialogs::fork_followups::draw_followup_queue(frame, queue, &app.theme, state);
    }

    // FORK: repo hook approval dialog.
    if let AppMode::ApprovingRepoHook(state) = &app.mode {
        crate::fork::hook_approval::draw(frame, state, &app.theme);
    }

    if let AppMode::ForkingFeature(state) = &app.mode {
        let allowed_agents = app.active_extension.allowed_agents();
        super::dialogs::draw_fork_feature_dialog(
            frame,
            state,
            allowed_agents.as_slice(),
            &app.theme,
        );
    }

    if let AppMode::ThemePicker(state) = &app.mode {
        super::dialogs::draw_theme_picker(
            frame,
            state,
            &app.config.theme,
            &app.theme,
            app.config.transparent_background,
        );
    }

    if let AppMode::ReviewHarnessPick(state) = &app.mode {
        super::dialogs::draw_review_harness_pick(frame, state, &app.theme);
    }

    if let AppMode::ReviewIntegrate(state) = &app.mode {
        super::dialogs::draw_review_integrate(frame, state, &app.theme);
    }

    if let AppMode::DebugLog(state) = &app.mode {
        super::dialogs::draw_debug_log(
            frame,
            &app.debug_log,
            state.scroll_offset,
            state.hide_perf_logs,
            &app.theme,
        );
    }

    if let AppMode::HarnessSetup(state) = &app.mode {
        super::dialogs::draw_harness_setup_dialog(frame, state, &app.throbber_state, &app.theme);
    }

    if let AppMode::PromptLibrary(state) = &app.mode {
        super::dialogs::draw_prompt_library(frame, state, app.message.as_deref(), &app.theme);
    }

    if let AppMode::PromptEditor(state) = &app.mode {
        super::dialogs::draw_prompt_editor(frame, state, &app.theme);
    }

    if let AppMode::PromptOverrides(state) = &app.mode {
        super::dialogs::draw_prompt_overrides(frame, state, &app.theme);
    }

    if let AppMode::PromptPrecall(pending) = &app.mode {
        super::dialogs::draw_prompt_precall(frame, pending, &app.theme);
    }

    if let AppMode::ReviewMemoryCompactConfirm(state) = &app.mode {
        // FORK: memoised — this ran `git rev-parse` + config reads per frame.
        let memory_paths = app.render_review_memory_paths(&state.workdir);
        super::dialogs::draw_compact_confirm(
            frame,
            &state.confirm,
            memory_paths.for_scope(state.confirm.scope),
            &app.theme,
        );
    }

    if let AppMode::PlaceholderFill(state) = &app.mode {
        super::dialogs::draw_placeholder_fill(frame, state, &app.theme);
    }

    if let AppMode::SkillPicker(state) = &app.mode {
        super::dialogs::draw_skill_picker(frame, state, &app.theme);
    }

    if let AppMode::ConfigWizard(state) = &mut app.mode {
        super::dialogs::draw_config_wizard_dialog(frame, state, &app.theme);
    }

    if let AppMode::ContextSettings(state) = &app.mode {
        super::dialogs::draw_context_settings_dialog(frame, state, &app.theme);
    }

    draw_mode_context_bar(frame, &app.mode, &app.theme);
    match &app.mode {
        AppMode::DeletingFeatureInProgress(state) => {
            draw_feature_context_bar(frame, &state.project_name, &state.feature_name, &app.theme);
        }
        AppMode::RunningHook(state) => {
            draw_feature_context_bar(frame, &state.project_name, &state.branch, &app.theme);
        }
        AppMode::HookPrompt(state) => match &state.next {
            crate::app::HookNext::WorktreeCreated {
                project_name,
                branch,
                ..
            } => draw_feature_context_bar(frame, project_name, branch, &app.theme),
            crate::app::HookNext::StartFeature { pi, fi, .. }
            | crate::app::HookNext::StopFeature { pi, fi } => {
                if let Some(project) = app.store.projects.get(*pi)
                    && let Some(feature) = project.features.get(*fi)
                {
                    draw_feature_context_bar(frame, &project.name, &feature.name, &app.theme);
                }
            }
        },
        _ => {}
    }

    // FORK: with the panels on screen a toast sits inside the list panel, so
    // it never straddles the panel borders; a dialog has cleared them away.
    let toast_area = match (
        matches!(app.mode, AppMode::Normal),
        app.fork_workspace.sidebar_rect,
    ) {
        (true, Some(body)) => body.inner(ratatui::layout::Margin::new(1, 1)),
        _ => frame.area(),
    };
    super::draw_toasts(frame, toast_area, &app.toasts, &app.theme);
}

pub fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::agent_sidebar::*;
    use crate::project::TokenUsageSourceMatch;

    use crate::app::App;
    use crate::project::FeatureSession;
    use crate::project::{
        AgentKind, Feature, Project, ProjectStatus, ProjectStore, SessionKind, VibeMode,
    };
    use crate::token_tracking::{TokenUsageProvider, TokenUsageSource};
    use crate::traits::{MockTmuxOps, MockWorktreeOps};
    use ratatui::layout::Rect;
    use ratatui::{Terminal, backend::TestBackend};
    use std::collections::HashMap;
    use std::path::PathBuf;

    // ── centered_rect ─────────────────────────────────────────

    #[test]
    fn centered_rect_50_percent() {
        let area = Rect::new(0, 0, 100, 100);
        let result = centered_rect(50, 50, area);
        // Middle slice should be 50% of 100 = 50 in each dim
        assert_eq!(result.width, 50);
        assert_eq!(result.height, 50);
        // Should start at 25% offset
        assert_eq!(result.x, 25);
        assert_eq!(result.y, 25);
    }

    #[test]
    fn centered_rect_80_60_percent() {
        let area = Rect::new(0, 0, 100, 100);
        let result = centered_rect(80, 60, area);
        assert_eq!(result.width, 80);
        assert_eq!(result.height, 60);
        assert_eq!(result.x, 10);
        assert_eq!(result.y, 20);
    }

    #[test]
    fn centered_rect_fits_within_area() {
        let area = Rect::new(10, 5, 80, 40);
        let result = centered_rect(60, 50, area);
        // Result must be inside the original area
        assert!(result.x >= area.x);
        assert!(result.y >= area.y);
        assert!(result.x + result.width <= area.x + area.width);
        assert!(result.y + result.height <= area.y + area.height);
    }

    #[test]
    fn centered_rect_100_percent_fills_area() {
        let area = Rect::new(0, 0, 80, 40);
        let result = centered_rect(100, 100, area);
        assert_eq!(result.width, area.width);
        assert_eq!(result.height, area.height);
    }

    #[test]
    fn sidebar_usage_is_split_into_labeled_lines() {
        assert_eq!(
            format_sidebar_usage("16.0k in · 2.0k out · 21.8k eff · $0.07"),
            "Input: 16.0k tokens\nOutput: 2.0k tokens\nEffective: 21.8k tokens\nCost: $0.07"
        );
    }

    #[test]
    fn sidebar_usage_falls_back_when_format_is_unknown() {
        assert_eq!(
            format_sidebar_usage("tokens unavailable"),
            "Usage: tokens unavailable"
        );
    }

    #[test]
    fn expired_toasts_do_not_leave_stale_cells_behind() {
        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut app = App::new_for_test(
            ProjectStore {
                version: 5,
                projects: vec![],
                session_bookmarks: vec![],
                available_harnesses: vec![],
                prompt_templates: Vec::new(),
                extra: HashMap::new(),
            },
            Box::new(MockTmuxOps::new()),
            Box::new(MockWorktreeOps::new()),
        );
        app.toasts.push(crate::app::Toast::new(
            "toast should disappear",
            crate::app::toast::ToastKind::Info,
        ));

        terminal
            .draw(|frame| draw(frame, &mut app))
            .expect("first draw");
        let first_rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(first_rendered.contains("toast should disappear"));

        app.toasts.clear();
        terminal
            .draw(|frame| draw(frame, &mut app))
            .expect("second draw");
        let second_rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(!second_rendered.contains("toast should disappear"));
    }

    fn codex_feature_session(session_id: &str) -> FeatureSession {
        FeatureSession {
            id: "session-1".into(),
            kind: SessionKind::Codex,
            label: "Codex".into(),
            tmux_window: "codex".into(),
            claude_session_id: None,
            todo_reference: None,
            token_usage_source: Some(TokenUsageSource {
                provider: TokenUsageProvider::Codex,
                id: session_id.into(),
            }),
            token_usage_source_match: Some(TokenUsageSourceMatch::Exact),
            created_at: chrono::Utc::now(),
            command: None,
            on_stop: None,
            pre_check: None,
            status_text: None,
            token_usage: None,
            stopped: false,
        }
    }

    #[test]
    fn select_sidebar_prompt_prefers_session_specific_prompt() {
        assert_eq!(
            select_sidebar_prompt(Some("session prompt"), Some("fallback prompt")),
            Some("session prompt".to_string())
        );
    }

    #[test]
    fn sidebar_prompt_text_falls_back_when_codex_session_prompt_is_missing() {
        let prompt = sidebar_prompt_text(None, Some("fallback prompt"));

        assert!(prompt.contains("fallback prompt"));
        assert!(!prompt.contains("leader l"));
        assert!(!prompt.contains("Preview:"));
    }

    #[test]
    fn sidebar_prompt_text_is_empty_when_no_prompt_is_available() {
        assert_eq!(sidebar_prompt_text(None, None), "");
    }

    #[test]
    fn sidebar_prompt_text_truncates_long_prompt_preview() {
        let prompt = sidebar_prompt_text(
            Some(
                "This is a much longer prompt preview that should be shortened once it crosses the sidebar limit for prompt text.",
            ),
            None,
        );

        assert_eq!(prompt, "This is a much longer prompt preview that shoul…");
    }

    #[test]
    fn compact_sidebar_text_truncates_summary_text() {
        let compacted = compact_sidebar_text(
            "This is a longer summary that should be shortened once it crosses the sidebar limit.",
            40,
        );

        assert_eq!(compacted, "This is a longer summary that should be…");
    }

    #[test]
    fn sidebar_status_activity_text_omits_activity_when_work_is_present() {
        assert_eq!(sidebar_status_activity_text(true, "Ready".into()), None);
        assert_eq!(
            sidebar_status_activity_text(false, "Ready".into()),
            Some("Ready".to_string())
        );
    }

    #[test]
    fn compose_sidebar_status_text_omits_missing_usage_lines() {
        assert_eq!(compose_sidebar_status_text(None, None, None), "");
        assert_eq!(
            compose_sidebar_status_text(Some("Ready".into()), None, None),
            "Activity: Ready"
        );
        assert_eq!(
            compose_sidebar_status_text(None, Some("Input: 1.2K tokens".into()), None),
            "Input: 1.2K tokens"
        );
    }

    #[test]
    fn append_model_status_line_adds_model_when_present() {
        assert_eq!(
            append_model_status_line("Activity: Ready".into(), Some("Model: gpt-5.5")),
            "Activity: Ready\nModel: gpt-5.5"
        );
        assert_eq!(
            append_model_status_line("Activity: Ready".into(), None),
            "Activity: Ready"
        );
    }

    #[test]
    fn compose_sidebar_summary_text_omits_missing_summary() {
        assert_eq!(compose_sidebar_summary_text(None, None), "");
        assert_eq!(
            compose_sidebar_summary_text(None, Some("Short summary".into())),
            "Short summary"
        );
    }

    #[test]
    fn format_codex_usage_source_confidence_omits_exact_match_label() {
        let session = codex_feature_session("sess-current");
        assert_eq!(
            format_codex_usage_source_confidence(&SessionKind::Codex, Some(&session)),
            None
        );
    }

    #[test]
    fn format_codex_usage_source_confidence_uses_inferred_match_label() {
        let mut session = codex_feature_session("sess-current");
        session.token_usage_source_match = Some(TokenUsageSourceMatch::Inferred);

        assert_eq!(
            format_codex_usage_source_confidence(&SessionKind::Codex, Some(&session)),
            Some("Usage source: inferred workdir match".to_string())
        );
    }

    fn store_with_claude_feature() -> (ProjectStore, Feature) {
        let now = chrono::Utc::now();
        let mut feature = Feature {
            id: "feat-1".into(),
            name: "feature".into(),
            branch: "feature".into(),
            workdir: PathBuf::from("/tmp/demo"),
            is_worktree: false,
            tmux_session: "aiw-feature".into(),
            sessions: vec![],
            collapsed: false,
            mode: VibeMode::Vibeless,
            review: false,
            plan_mode: false,
            agent: AgentKind::Claude,
            enable_chrome: false,
            pending_worktree_script: false,
            ready: false,
            status: ProjectStatus::Active,
            created_at: now,
            last_accessed: now,
            summary: None,
            summary_updated_at: None,
            nickname: None,
            selected_plan_path: None,
            triage_source: None,
            review_source: None,
            issue_source: None,
        };
        feature.add_session_named(SessionKind::Claude, "Claude 1".to_string());
        let project = Project {
            id: "proj-1".into(),
            name: "demo".into(),
            repo: PathBuf::from("/tmp/demo"),
            collapsed: false,
            features: vec![feature.clone()],
            created_at: now,
            preferred_agent: AgentKind::Claude,
            is_git: false,
        };
        (
            ProjectStore {
                version: 5,
                projects: vec![project],
                session_bookmarks: vec![],
                available_harnesses: vec![],
                prompt_templates: Vec::new(),
                extra: HashMap::new(),
            },
            feature,
        )
    }

    fn pr_review_state_with_branches(
        pr_head_ref: &str,
        checked_out: &str,
    ) -> crate::app::PrReviewState {
        let pr = crate::github::PrRef {
            number: 321,
            head_sha: "abc123".to_string(),
            url: "https://github.com/o/r/pull/321".to_string(),
            owner: "o".to_string(),
            repo: "r".to_string(),
            head_ref: pr_head_ref.to_string(),
        };
        let review = crate::app::pr_review::normalize(pr, vec![], vec![], vec![], vec![]);
        crate::app::PrReviewState {
            workdir: PathBuf::from("/tmp/demo"),
            review,
            selected: 0,
            detail_scroll: 0,
            detail_content_lines: 0,
            hide_resolved: false,
            sort_mode: crate::app::pr_review::PrSortMode::default(),
            fix_target: crate::app::pr_review::FixTarget::default(),
            fix_target_picked: false,
            usage_baselines: HashMap::new(),
            review_harness: None,
            dedicated_session_label: crate::app::pr_review::TRIAGE_SESSION_LABEL.to_string(),
            harness_pick: None,
            new_feature_setup: None,
            integrate: None,
            fix_confirm: None,
            fix_vim_enabled: false,
            mark_pick: None,
            reply_kind_pick: None,
            reply: None,
            memory_add: None,
            marked: std::collections::HashSet::new(),
            pending_batch: false,
            checked_out_branch: Some(checked_out.to_string()),
            pending_ai_review_findings: 0,
            ai_review_last_run: None,
            investigations: Vec::new(),
            investigation_harness_pick: None,
            investigation_action_pick: None,
            investigation_follow_up: None,
            pending_follow_up: None,
            investigation_context: Default::default(),
        }
    }

    #[test]
    fn pr_review_pane_shows_ai_review_running_badge_for_its_own_workdir() {
        let (store, _feature) = store_with_claude_feature();
        let mut app = App::new_for_test(
            store,
            Box::new(MockTmuxOps::new()),
            Box::new(MockWorktreeOps::new()),
        );
        let mut state = pr_review_state_with_branches("main", "main");
        state.pending_ai_review_findings = 3;
        state.ai_review_last_run = Some(crate::app::ai_review::AiReviewRun {
            ran_at: chrono::Local::now(),
            outcome: crate::app::ai_review::AiReviewRunOutcome::Error("cached failure".to_string()),
        });
        let workdir = state.workdir.clone();
        app.mode = crate::app::AppMode::PrReview(state);

        let pr = crate::github::PrRef {
            number: 321,
            head_sha: "abc123".to_string(),
            url: "https://github.com/o/r/pull/321".to_string(),
            owner: "o".to_string(),
            repo: "r".to_string(),
            head_ref: "main".to_string(),
        };
        let (_tx, rx) = std::sync::mpsc::channel();
        app.ai_review_run.set_receiver_for_test(Some(rx));
        app.ai_review_run
            .set_origin_for_test(Some(crate::app::AiReviewState {
                questions: Default::default(),
                workdir,
                pr,
                findings: Vec::new(),
                summary: None,
                attribution: None,
                selected: 0,
                detail_scroll: 0,
                detail_content_lines: 0,
                last_run: None,
                harness: None,
                harness_pick: None,
                harness_pick_origin: None,
                model: None,
                model_picked: false,
                reasoning: None,
                model_pick: None,
                finding_editor: None,
                post_confirm: None,
                marked: Default::default(),
            }));

        let backend = TestBackend::new(140, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| super::draw(frame, &mut app)).unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();

        assert!(rendered.contains("AI review running"));
        assert!(!rendered.contains("AI review pending"));
        assert!(!rendered.contains("AI review failed"));
    }

    #[test]
    fn pr_review_pane_omits_ai_review_badge_when_nothing_is_running() {
        let (store, _feature) = store_with_claude_feature();
        let mut app = App::new_for_test(
            store,
            Box::new(MockTmuxOps::new()),
            Box::new(MockWorktreeOps::new()),
        );
        app.mode = crate::app::AppMode::PrReview(pr_review_state_with_branches("main", "main"));

        let backend = TestBackend::new(140, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| super::draw(frame, &mut app)).unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();

        assert!(!rendered.contains("AI review running"));
        assert!(!rendered.contains("AI review pending"));
    }

    #[test]
    fn pr_review_pane_shows_completed_pending_ai_review_count() {
        let (store, _feature) = store_with_claude_feature();
        let mut app = App::new_for_test(
            store,
            Box::new(MockTmuxOps::new()),
            Box::new(MockWorktreeOps::new()),
        );
        let mut state = pr_review_state_with_branches("main", "main");
        state.pending_ai_review_findings = 3;
        app.mode = crate::app::AppMode::PrReview(state);

        let backend = TestBackend::new(140, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| super::draw(frame, &mut app)).unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();

        assert!(rendered.contains("AI review pending: 3"));
    }

    #[test]
    fn pr_review_pane_shows_completed_zero_finding_result() {
        let (store, _feature) = store_with_claude_feature();
        let mut app = App::new_for_test(
            store,
            Box::new(MockTmuxOps::new()),
            Box::new(MockWorktreeOps::new()),
        );
        let mut state = pr_review_state_with_branches("main", "main");
        state.ai_review_last_run = Some(crate::app::ai_review::AiReviewRun {
            ran_at: chrono::Local::now(),
            outcome: crate::app::ai_review::AiReviewRunOutcome::Findings(0),
        });
        app.mode = crate::app::AppMode::PrReview(state);

        let backend = TestBackend::new(160, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| super::draw(frame, &mut app)).unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();

        assert!(rendered.contains("[AI review: no findings (now)]"));
    }

    #[test]
    fn pr_review_pane_shows_failed_result_without_error_details() {
        let (store, _feature) = store_with_claude_feature();
        let mut app = App::new_for_test(
            store,
            Box::new(MockTmuxOps::new()),
            Box::new(MockWorktreeOps::new()),
        );
        let mut state = pr_review_state_with_branches("main", "main");
        state.ai_review_last_run = Some(crate::app::ai_review::AiReviewRun {
            ran_at: chrono::Local::now(),
            outcome: crate::app::ai_review::AiReviewRunOutcome::Error(
                "sensitive worker detail".to_string(),
            ),
        });
        app.mode = crate::app::AppMode::PrReview(state);

        let backend = TestBackend::new(160, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| super::draw(frame, &mut app)).unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();

        assert!(rendered.contains("[AI review failed (now)]"));
        assert!(!rendered.contains("sensitive worker detail"));
    }

    #[test]
    fn pending_findings_take_precedence_over_cached_zero_result() {
        let (store, _feature) = store_with_claude_feature();
        let mut app = App::new_for_test(
            store,
            Box::new(MockTmuxOps::new()),
            Box::new(MockWorktreeOps::new()),
        );
        let mut state = pr_review_state_with_branches("main", "main");
        state.pending_ai_review_findings = 2;
        state.ai_review_last_run = Some(crate::app::ai_review::AiReviewRun {
            ran_at: chrono::Local::now(),
            outcome: crate::app::ai_review::AiReviewRunOutcome::Findings(0),
        });
        app.mode = crate::app::AppMode::PrReview(state);

        let backend = TestBackend::new(160, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| super::draw(frame, &mut app)).unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();

        assert!(rendered.contains("AI review pending: 2"));
        assert!(!rendered.contains("no findings"));
    }

    #[test]
    fn pr_review_pane_shows_branch_mismatch_banner_when_workdir_branch_differs() {
        let (store, _feature) = store_with_claude_feature();
        let mut app = App::new_for_test(
            store,
            Box::new(MockTmuxOps::new()),
            Box::new(MockWorktreeOps::new()),
        );
        app.mode =
            crate::app::AppMode::PrReview(pr_review_state_with_branches("main", "other-branch"));

        let backend = TestBackend::new(140, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| super::draw(frame, &mut app)).unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();

        assert!(rendered.contains("reviewing PR for branch"));
        assert!(rendered.contains("other-branch"));
    }

    #[test]
    fn pr_review_pane_hides_branch_mismatch_banner_when_branches_match() {
        let (store, _feature) = store_with_claude_feature();
        let mut app = App::new_for_test(
            store,
            Box::new(MockTmuxOps::new()),
            Box::new(MockWorktreeOps::new()),
        );
        app.mode = crate::app::AppMode::PrReview(pr_review_state_with_branches("main", "main"));

        let backend = TestBackend::new(140, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| super::draw(frame, &mut app)).unwrap();
        let rendered: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();

        assert!(!rendered.contains("reviewing PR for branch"));
    }
}
