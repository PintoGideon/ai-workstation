use anyhow::Result;
use crossterm::event::{MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use std::time::Instant;

use crate::app::{App, AppMode, CreateFeatureStep, Selection, VisibleItem};
use crate::project::AgentKind;

static mut LAST_CLICK_TIME: Option<Instant> = None;
static mut LAST_CLICK_ROW: Option<u16> = None;
const VIEW_MOUSE_SCROLL_LINES: usize = 3;
const DEBUG_LOG_MOUSE_SCROLL_LINES: usize = 3;
const MARKDOWN_MOUSE_SCROLL_LINES: usize = 3;
const HELP_MOUSE_SCROLL_LINES: usize = 3;
const PLAN_INTERVIEW_MOUSE_SCROLL_LINES: usize = 3;

pub fn handle_mouse(app: &mut App, mouse: MouseEvent, visible_rows: u16) -> Result<()> {
    if app.review_questions().is_some_and(|q| q.open) {
        if let Some(q) = app.review_questions_mut() {
            match mouse.kind {
                MouseEventKind::ScrollDown => q.scroll += VIEW_MOUSE_SCROLL_LINES,
                MouseEventKind::ScrollUp => {
                    q.scroll = q.scroll.saturating_sub(VIEW_MOUSE_SCROLL_LINES)
                }
                _ => {}
            }
        }
        return Ok(());
    }
    match mouse.kind {
        MouseEventKind::ScrollUp => {
            handle_scroll_up(app);
        }
        MouseEventKind::ScrollDown => {
            handle_scroll_down(app);
        }
        MouseEventKind::Down(_) => {
            handle_click(app, mouse.column, mouse.row, visible_rows)?;
        }
        MouseEventKind::Moved => {
            handle_move(app, mouse.row, visible_rows);
        }
        _ => {}
    }
    Ok(())
}

fn handle_move(app: &mut App, row: u16, visible_rows: u16) {
    let AppMode::CreatingFeature(state) = &mut app.mode else {
        return;
    };
    if state.step != CreateFeatureStep::Mode {
        return;
    }

    let full_height = visible_rows.saturating_add(3);
    let dialog_top = full_height.saturating_mul(5) / 100;
    let inner_top = dialog_top.saturating_add(1);
    let relative_row = row.saturating_sub(inner_top);

    state.mode_focus = match relative_row {
        8..=11 => 0,
        13..=17 => 1,
        19 => 2,
        20 => 3,
        21 if state.agent == AgentKind::Claude => 4,
        22 if state.agent == AgentKind::Claude => 5,
        21 => 4,
        23 if state.agent == AgentKind::Claude => 6,
        _ => state.mode_focus,
    };
}

fn handle_scroll_up(app: &mut App) {
    if matches!(app.mode, AppMode::DiffViewer(_)) {
        super::handle_diff_viewer_wheel(app, VIEW_MOUSE_SCROLL_LINES, false);
        return;
    }
    if matches!(
        app.mode,
        AppMode::DiffPicker(_) | AppMode::DiffViewerLoading(_)
    ) {
        return;
    }
    if matches!(app.mode, AppMode::MarkdownLoading(_)) {
        return;
    }
    if matches!(app.mode, AppMode::DiffReviewPrompt(_)) {
        app.diff_review_scroll_patch_up(VIEW_MOUSE_SCROLL_LINES);
        return;
    }
    if let AppMode::DebugLog(state) = &mut app.mode {
        state.scroll_offset = state
            .scroll_offset
            .saturating_sub(DEBUG_LOG_MOUSE_SCROLL_LINES);
        return;
    }
    if let AppMode::MarkdownViewer(state) = &mut app.mode {
        state.scroll_offset = state
            .scroll_offset
            .saturating_sub(MARKDOWN_MOUSE_SCROLL_LINES);
        return;
    }
    if let AppMode::Help(state) = &mut app.mode {
        state.scroll_offset = state.scroll_offset.saturating_sub(HELP_MOUSE_SCROLL_LINES);
        return;
    }
    if let AppMode::PlanInterview(state) = &mut app.mode {
        if let Some(offset) = state.scroll_offset_mut() {
            *offset = offset.saturating_sub(PLAN_INTERVIEW_MOUSE_SCROLL_LINES);
        }
        return;
    }
    app.select_prev();
}

fn handle_scroll_down(app: &mut App) {
    if matches!(app.mode, AppMode::DiffViewer(_)) {
        super::handle_diff_viewer_wheel(app, VIEW_MOUSE_SCROLL_LINES, true);
        return;
    }
    if matches!(
        app.mode,
        AppMode::DiffPicker(_) | AppMode::DiffViewerLoading(_)
    ) {
        return;
    }
    if matches!(app.mode, AppMode::MarkdownLoading(_)) {
        return;
    }
    if matches!(app.mode, AppMode::DiffReviewPrompt(_)) {
        app.diff_review_scroll_patch_down(VIEW_MOUSE_SCROLL_LINES);
        return;
    }
    if let AppMode::DebugLog(state) = &mut app.mode {
        state.scroll_offset = state
            .scroll_offset
            .saturating_add(DEBUG_LOG_MOUSE_SCROLL_LINES);
        return;
    }
    if let AppMode::MarkdownViewer(state) = &mut app.mode {
        state.scroll_offset = state
            .scroll_offset
            .saturating_add(MARKDOWN_MOUSE_SCROLL_LINES);
        return;
    }
    if let AppMode::Help(state) = &mut app.mode {
        state.scroll_offset = state.scroll_offset.saturating_add(HELP_MOUSE_SCROLL_LINES);
        return;
    }
    if let AppMode::PlanInterview(state) = &mut app.mode {
        if let Some(offset) = state.scroll_offset_mut() {
            *offset = offset.saturating_add(PLAN_INTERVIEW_MOUSE_SCROLL_LINES);
        }
        return;
    }
    app.select_next();
}

fn handle_click(app: &mut App, col: u16, row: u16, visible_rows: u16) -> Result<()> {
    if matches!(app.mode, AppMode::Help(_)) {
        app.mode = AppMode::Normal;
        return Ok(());
    }

    if row == 1 {
        let cwd = crate::ui::fork_chrome::cwd_label(); // FORK: cached cwd
        // The header owns the top three rows at full width; ask it where the
        // badge landed rather than re-deriving the title's layout here.
        let header_area = Rect::new(0, 0, app.viewport_cols, 3);
        if let Some(badge) = crate::ui::header::badge_hit_columns(
            header_area,
            cwd,
            env!("CARGO_PKG_VERSION"),
            app.attention_badge_count(),
            // FORK: G11 pills take header columns too.
            crate::ui::fork_chrome::header_pills(app),
        ) && badge.contains(&col)
        {
            app.mode = AppMode::NotificationPicker(0, None);
            return Ok(());
        }
    }

    if matches!(
        app.mode,
        AppMode::CreatingProject(_)
            | AppMode::CreatingFeature(_)
            | AppMode::DeletingProject(_)
            | AppMode::DeletingFeature(_, _)
            | AppMode::BrowsingPath(_)
            | AppMode::PlanInterviewAttachDoc(_)
            | AppMode::CommandPicker(_)
            | AppMode::Searching(_)
            | AppMode::ClaudeSessionPicker(_)
            | AppMode::ConfirmingClaudeSession { .. }
            | AppMode::CodexSessionPicker(_)
            | AppMode::ConfirmingCodexSession { .. }
            | AppMode::StoppedSessionDialog(_)
            | AppMode::SessionPicker(_)
            | AppMode::NamingNewSession(_)
            | AppMode::BookmarkPicker(_)
            | AppMode::DiffPicker(_)
            | AppMode::DiffViewerLoading(_)
            | AppMode::DiffViewer(_)
            | AppMode::MarkdownLoading(_)
            | AppMode::RenamingSession(_)
            | AppMode::RenamingFeature(_)
            | AppMode::NotificationPicker(_, _)
            | AppMode::PermissionDetail(_) // FORK: P6
            | AppMode::FollowupQueue(_) // FORK: P7
            | AppMode::DiffReviewPrompt(_)
            | AppMode::RunningHook(_)
            | AppMode::ConfirmResourceStart(_)
            | AppMode::PlanInterview(_)
    ) {
        return Ok(());
    }

    let list_start_row = 4;
    let list_visible_height = visible_rows.saturating_sub(5);
    let list_end_row = list_start_row + list_visible_height;

    if row >= list_start_row && row < list_end_row {
        let clicked_in_list = row - list_start_row;
        if let Some(item_index) =
            app.item_index_at_visible_row(clicked_in_list as usize, list_visible_height as usize)
        {
            let visible = app.visible_items();
            let clicked_item = visible[item_index].clone();

            let is_double_click = unsafe {
                let now = Instant::now();
                let is_double = LAST_CLICK_TIME
                    .map(|t| now.duration_since(t).as_millis() < 400)
                    .unwrap_or(false)
                    && LAST_CLICK_ROW == Some(row);
                LAST_CLICK_TIME = Some(now);
                LAST_CLICK_ROW = Some(row);
                is_double
            };

            if is_double_click {
                handle_double_click(app, &clicked_item, col)?;
            } else {
                app.selection = match clicked_item {
                    VisibleItem::Project(pi) => Selection::Project(pi),
                    VisibleItem::Feature(pi, fi) => Selection::Feature(pi, fi),
                    VisibleItem::Session(pi, fi, si) => Selection::Session(pi, fi, si),
                };
                app.reload_extension_config();
            }
        }
    }

    Ok(())
}

fn handle_double_click(app: &mut App, item: &VisibleItem, col: u16) -> Result<()> {
    match item {
        VisibleItem::Project(pi) => {
            if let Some(project) = app.store.projects.get_mut(*pi) {
                project.collapsed = !project.collapsed;
            }
        }
        VisibleItem::Feature(_pi, _fi) => {
            if col < 10 {
                if let Some(project) = app.store.projects.get_mut(*_pi)
                    && let Some(feature) = project.features.get_mut(*_fi)
                {
                    feature.collapsed = !feature.collapsed;
                }
            } else {
                app.enter_view()?;
            }
        }
        VisibleItem::Session(_pi, _fi, _si) => {
            app.selection = match item {
                VisibleItem::Session(pi, fi, si) => Selection::Session(*pi, *fi, *si),
                _ => unreachable!(),
            };
            app.enter_view()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, AppMode, DebugLogState, PlanInterviewPhase, PlanInterviewState};
    use crate::project::ProjectStore;
    use crate::traits::{MockTmuxOps, MockWorktreeOps};
    use crossterm::event::KeyModifiers;
    use std::collections::HashMap;

    fn test_app() -> App {
        App::new_for_test(
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
        )
    }

    #[test]
    fn mouse_scroll_down_moves_debug_log() {
        let mut app = test_app();
        app.mode = AppMode::DebugLog(DebugLogState {
            scroll_offset: 1,
            from_view: None,
            hide_perf_logs: false,
        });

        handle_mouse(
            &mut app,
            MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            },
            20,
        )
        .unwrap();

        match &app.mode {
            AppMode::DebugLog(state) => {
                assert_eq!(state.scroll_offset, 1 + DEBUG_LOG_MOUSE_SCROLL_LINES)
            }
            _ => panic!("expected debug log to stay open"),
        }
    }

    fn plan_interview_app(phase: PlanInterviewPhase) -> App {
        let mut app = test_app();
        let mut state = PlanInterviewState::for_feature(
            "feature".into(),
            "feat-1".into(),
            Vec::new(),
            std::path::PathBuf::from("/tmp/does-not-matter"),
            AgentKind::Claude,
        );
        state.synthesized_plan = Some("# Plan\n".into());
        state.phase = phase;
        app.mode = AppMode::PlanInterview(state);
        app
    }

    fn scroll(app: &mut App, kind: MouseEventKind) {
        handle_mouse(
            app,
            MouseEvent {
                kind,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            },
            20,
        )
        .unwrap();
    }

    #[test]
    fn mouse_scroll_moves_the_plan_review_pane() {
        let mut app = plan_interview_app(PlanInterviewPhase::Review);

        scroll(&mut app, MouseEventKind::ScrollDown);
        scroll(&mut app, MouseEventKind::ScrollDown);
        scroll(&mut app, MouseEventKind::ScrollUp);

        match &app.mode {
            AppMode::PlanInterview(state) => {
                assert_eq!(
                    state.review_scroll_offset,
                    PLAN_INTERVIEW_MOUSE_SCROLL_LINES
                );
            }
            _ => panic!("expected the interview to stay open"),
        }
    }

    /// The advisory review is a second markdown pane behind the same dialog, so
    /// the wheel has to reach its own offset rather than the plan's.
    #[test]
    fn mouse_scroll_moves_the_critique_pane_not_the_plan() {
        let mut app = plan_interview_app(PlanInterviewPhase::Critique);

        scroll(&mut app, MouseEventKind::ScrollDown);

        match &app.mode {
            AppMode::PlanInterview(state) => {
                assert_eq!(
                    state.critique_scroll_offset,
                    PLAN_INTERVIEW_MOUSE_SCROLL_LINES
                );
                assert_eq!(state.review_scroll_offset, 0);
            }
            _ => panic!("expected the interview to stay open"),
        }
    }

    /// A phase with nothing to scroll must still swallow the wheel: falling
    /// through would move the dashboard selection hidden behind the dialog.
    #[test]
    fn mouse_scroll_is_swallowed_by_an_unscrollable_phase() {
        let mut app = plan_interview_app(PlanInterviewPhase::Brief);
        let before = format!("{:?}", app.selection);

        scroll(&mut app, MouseEventKind::ScrollDown);

        assert_eq!(format!("{:?}", app.selection), before);
        match &app.mode {
            AppMode::PlanInterview(state) => {
                assert_eq!(state.review_scroll_offset, 0);
                assert_eq!(state.edit_scroll_offset, 0);
            }
            _ => panic!("expected the interview to stay open"),
        }
    }

    #[test]
    fn mouse_scroll_up_clamps_debug_log_at_top() {
        let mut app = test_app();
        app.mode = AppMode::DebugLog(DebugLogState {
            scroll_offset: 1,
            from_view: None,
            hide_perf_logs: false,
        });

        handle_mouse(
            &mut app,
            MouseEvent {
                kind: MouseEventKind::ScrollUp,
                column: 0,
                row: 0,
                modifiers: KeyModifiers::NONE,
            },
            20,
        )
        .unwrap();

        match &app.mode {
            AppMode::DebugLog(state) => assert_eq!(state.scroll_offset, 0),
            _ => panic!("expected debug log to stay open"),
        }
    }
}
