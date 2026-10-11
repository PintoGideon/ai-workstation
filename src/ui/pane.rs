//! What is left of the embedded session view (DESIGN.md §18): the leader
//! command menu the dashboard still draws, and the capture normaliser the
//! desktop GUI's terminal shares. The view itself — a captured tmux screen
//! redrawn by AIW, with its Claude sidebar — is gone: the cockpit shows the
//! agent's real pane beside the sidebar.

use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::theme::Theme;

pub(crate) fn draw_command_menu(
    frame: &mut Frame,
    content_area: Rect,
    title: &str,
    commands: &[(&str, &str)],
    theme: &Theme,
) {
    if content_area.width < 30 || content_area.height < 8 {
        return;
    }

    let longest_label = commands
        .iter()
        .map(|(key, desc)| key.len() + desc.len() + 4)
        .max()
        .unwrap_or(24) as u16;
    let width = (longest_label + 4).clamp(30, content_area.width.saturating_sub(2));
    let height = (commands.len() as u16 + 2).min(content_area.height.saturating_sub(1));
    let x = content_area.x + content_area.width.saturating_sub(width + 1);
    let y = content_area.y + content_area.height.saturating_sub(height + 1);
    let area = Rect::new(x, y, width, height);

    let lines: Vec<Line<'static>> = commands
        .iter()
        .map(|(key, desc)| {
            Line::from(vec![
                Span::styled(
                    format!("{:<6}", key),
                    Style::default()
                        .fg(theme.info.to_color())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::styled(desc.to_string(), Style::default().fg(theme.text.to_color())),
            ])
        })
        .collect();

    let popup = Paragraph::new(lines).block(
        Block::default()
            .title(title)
            .borders(Borders::ALL)
            .style(Style::default().bg(theme.effective_bg()))
            .border_style(Style::default().fg(theme.info.to_color())),
    );

    frame.render_widget(Clear, area);
    frame.render_widget(popup, area);
}

/// Normalize captured tmux pane text for vt100 parsing. capture-pane
/// output terminates the last line with a newline; processing that
/// final newline in a parser whose height equals the pane scrolls the
/// whole screen up one row, so strip it before converting line
/// endings.
pub(crate) fn normalize_captured_pane(raw: &str) -> String {
    let raw = raw.strip_suffix('\n').unwrap_or(raw);
    raw.replace('\n', "\r\n")
}
