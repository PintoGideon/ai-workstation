//! FORK: the watchdog's recovery dialog (DESIGN.md §19.8): why the session
//! is flagged, and the actions that apply, numbered.

use ratatui::{
    Frame,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::app::fork_watchdog::WatchdogDialogState;
use crate::theme::Theme;

pub fn draw_watchdog(frame: &mut Frame, state: &WatchdogDialogState, theme: &Theme) {
    let area = frame.area();
    if area.width < 30 || area.height < 8 {
        return;
    }
    let width = (area.width * 7 / 10).clamp(44, 90).min(area.width);
    let height = (state.actions.len() as u16 + 6).min(area.height);
    let rect = crate::ui::dialog_rect(area, width, height);
    crate::ui::draw_modal_overlay(frame, rect, theme);
    let key = crate::ui::fork_chrome::key_style(theme);
    let hints = Line::from(vec![
        Span::styled(" Enter", key),
        Span::raw(" run  "),
        Span::styled("1-9", key),
        Span::raw(" pick  "),
        Span::styled("Esc", key),
        Span::raw(" close  "),
    ]);
    let block = Block::default()
        .title(format!(" Recover {} · {} ", state.label, state.feature))
        .title_bottom(hints)
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .style(Style::default().bg(theme.effective_bg()))
        .border_style(Style::default().fg(theme.warning.to_color()));
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);

    let dim = Style::default().fg(theme.text_muted.to_color());
    let text = Style::default().fg(theme.text.to_color());
    let warn = Style::default()
        .fg(theme.warning.to_color())
        .add_modifier(Modifier::BOLD);
    let mut lines = Vec::new();
    match &state.reason {
        Some(reason) => lines.push(Line::from(Span::styled(
            format!(" ⚠ {}", reason.trim_end_matches(" · ! recover")),
            warn,
        ))),
        None => lines.push(Line::from(Span::styled(
            " Nothing flagged; these act on the agent now.",
            dim,
        ))),
    }
    lines.push(Line::from(""));
    for (i, action) in state.actions.iter().enumerate() {
        let selected = i == state.selected;
        let style = if selected {
            text.add_modifier(Modifier::BOLD | Modifier::REVERSED)
        } else {
            text
        };
        lines.push(Line::from(vec![
            Span::styled(format!(" {} ", i + 1), key),
            Span::styled(format!(" {} ", action.label()), style),
        ]));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}
