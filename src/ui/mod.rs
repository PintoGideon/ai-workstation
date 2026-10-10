mod dashboard;
mod dialogs;
pub(crate) mod fork_chrome; // FORK: G11 dashboard chrome
pub(crate) mod fork_permissions; // FORK: permission approvals (P6)
mod fork_split; // FORK: split view
pub(crate) mod fork_text; // FORK: `…` fitting, hint cutting, path wrapping
pub(crate) mod fork_workspace; // FORK: workspace layout (DESIGN.md §15)
pub mod header;
pub(crate) mod list;
pub mod pane;
mod picker;
mod status;
pub(crate) mod toast; // FORK: `wrap_message` shared with the help overlay

use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    widgets::{Block, Clear},
};

use crate::app::App;
use crate::theme::Theme;

pub(crate) use pane::SCROLLBAR_WIDTH;
pub(crate) use pane::normalize_captured_pane;
pub(crate) use pane::render_ansi_lines;
pub(crate) use pane::render_vt100_screen;
pub(crate) use pane::viewing_main_width;
pub(crate) use toast::draw_toasts;

pub fn draw(frame: &mut Frame, app: &mut App) {
    // FORK: the workspace layout decides by the terminal width; keep the
    // viewport the main loop records in step with the frame being drawn
    // (tests draw without the loop).
    let area = frame.area();
    if (app.viewport_cols, app.viewport_total_rows) != (area.width, area.height) {
        app.viewport_cols = area.width;
        app.viewport_total_rows = area.height;
        app.viewport_rows = area.height.saturating_sub(3);
    }
    // FORK: split view draws the whole screen itself.
    if matches!(app.mode, crate::app::AppMode::SplitView(_)) {
        fork_split::draw(frame, app);
        return;
    }
    dashboard::draw(frame, app);
}

pub(crate) fn draw_modal_overlay(frame: &mut Frame, area: Rect, theme: &Theme) {
    let viewport = frame.area();
    frame.render_widget(Clear, viewport);
    frame.render_widget(
        Block::default().style(Style::default().bg(theme.effective_bg())),
        viewport,
    );

    let shadow = Rect::new(
        area.x.saturating_add(1),
        area.y.saturating_add(1),
        area.width.saturating_sub(1),
        area.height.saturating_sub(1),
    );
    if shadow.width > 0 && shadow.height > 0 {
        frame.render_widget(
            Block::default().style(Style::default().bg(theme.background.to_color())),
            shadow,
        );
    }

    frame.render_widget(Clear, area);
}

/// Center a content-sized dialog, clamping it to the current terminal.
pub(crate) fn dialog_rect(viewport: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(viewport.width);
    let height = height.min(viewport.height);
    Rect::new(
        viewport.x + (viewport.width - width) / 2,
        viewport.y + (viewport.height - height) / 2,
        width,
        height,
    )
}

#[cfg(test)]
mod usability_tests;

// FORK: dashboard frame benchmark (G11 budget, DESIGN.md §12).
#[cfg(test)]
pub(crate) mod fork_bench;
