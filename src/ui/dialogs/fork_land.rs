//! FORK: the land dialog (DESIGN.md §19.10): the preflight (base, ahead /
//! behind, blockers, warnings, the plan), a running land's stage and log, or
//! the last failure with what to do about it.

use ratatui::{
    Frame,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::app::App;
use crate::app::fork_land::{LandDialogState, LineTone, land_lines};

pub fn draw_land(frame: &mut Frame, app: &App, state: &LandDialogState) {
    let area = frame.area();
    if area.width < 40 || area.height < 10 {
        return;
    }
    let theme = &app.theme;
    let width = (area.width * 8 / 10).clamp(50, 110).min(area.width);
    let height = (area.height * 8 / 10).clamp(12, 34).min(area.height);
    let rect = crate::ui::dialog_rect(area, width, height);
    crate::ui::draw_modal_overlay(frame, rect, theme);
    let key = crate::ui::fork_chrome::key_style(theme);
    let landing = app.land_stage(&state.feature_id).is_some();
    let mut hints = Vec::new();
    let mut hint = |k: &str, label: &str| {
        hints.push(Span::styled(format!(" {k}"), key));
        hints.push(Span::raw(format!(" {label} ")));
    };
    if landing {
        hint("t", "terminal");
        hint("Esc", "close (it keeps going)");
    } else if state.confirm_working {
        hint("y", "land anyway");
        hint("Esc", "back");
    } else {
        hint("Enter", "land");
        hint("r", "recheck");
        hint("a", "abort rebase/merge");
        hint("t", "terminal");
        if app.land_failure(&state.feature_id).is_some() {
            hint("x", "dismiss");
        }
        hint("Esc", "close");
    }
    let block = Block::default()
        .title(format!(" Land {} · {} ", state.feature, state.project))
        .title_bottom(Line::from(hints))
        .borders(Borders::ALL)
        .style(Style::default().bg(theme.effective_bg()))
        .border_style(Style::default().fg(theme.primary.to_color()));
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);

    let log_lines = (inner.height as usize).saturating_sub(4).max(4);
    let mut lines: Vec<Line> = Vec::new();
    if let Some(note) = &state.note {
        lines.push(Line::from(Span::styled(
            format!(" {note}"),
            Style::default()
                .fg(theme.warning.to_color())
                .add_modifier(Modifier::BOLD),
        )));
    }
    for (text, tone) in land_lines(app, state, log_lines) {
        let style = match tone {
            LineTone::Plain => Style::default().fg(theme.text.to_color()),
            LineTone::Dim => Style::default().fg(theme.text_muted.to_color()),
            LineTone::Good => Style::default().fg(theme.success.to_color()),
            LineTone::Bad => Style::default()
                .fg(theme.danger.to_color())
                .add_modifier(Modifier::BOLD),
            LineTone::Warn => Style::default().fg(theme.warning.to_color()),
            LineTone::Working => Style::default()
                .fg(theme.status_working.to_color())
                .add_modifier(Modifier::BOLD),
        };
        lines.push(Line::from(Span::styled(format!(" {text}"), style)));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}
