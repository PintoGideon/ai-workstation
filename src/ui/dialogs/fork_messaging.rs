//! FORK: the `@` message dialog (DESIGN.md §19.5).

use ratatui::{
    Frame,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::app::fork_messaging::MessageDialogState;
use crate::theme::Theme;

fn key_hints(theme: &Theme, pairs: &[(&'static str, &'static str)]) -> Line<'static> {
    let key = crate::ui::fork_chrome::key_style(theme);
    let mut spans = Vec::new();
    for (i, (k, label)) in pairs.iter().enumerate() {
        spans.push(Span::styled(
            if i == 0 {
                format!(" {k}")
            } else {
                k.to_string()
            },
            key,
        ));
        spans.push(Span::raw(format!(" {label}  ")));
    }
    Line::from(spans)
}

/// Draw the dialog: the target (with how the message will travel) and the
/// text being typed.
pub fn draw_message_dialog(frame: &mut Frame, state: &MessageDialogState, theme: &Theme) {
    let area = frame.area();
    if area.width < 30 || area.height < 9 {
        return;
    }
    let width = (area.width * 7 / 10).clamp(44, 96).min(area.width);
    let height = (state.text.lines().count() as u16 + 8).clamp(10, area.height.min(26));
    let rect = crate::ui::dialog_rect(area, width, height);
    crate::ui::draw_modal_overlay(frame, rect, theme);
    let block = Block::default()
        .title(" Message ")
        .title_bottom(key_hints(
            theme,
            &[
                ("Enter", "send"),
                ("↑/↓", "target"),
                ("Alt+Enter", "newline"),
                ("Ctrl+U", "clear"),
                ("Esc", "cancel"),
            ],
        ))
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .style(Style::default().bg(theme.effective_bg()))
        .border_style(Style::default().fg(theme.primary.to_color()));
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);

    let dim = Style::default().fg(theme.text_muted.to_color());
    let text = Style::default().fg(theme.text.to_color());
    let strong = Style::default()
        .fg(theme.primary.to_color())
        .add_modifier(Modifier::BOLD);
    let cursor = Span::styled("█", Style::default().fg(theme.primary.to_color()));
    let mut lines = Vec::new();
    match state.target() {
        Some(target) => {
            let position = if state.targets.len() > 1 {
                format!("  ({}/{})", state.target_index + 1, state.targets.len())
            } else {
                String::new()
            };
            lines.push(Line::from(vec![
                Span::styled(" To ", dim),
                Span::styled(target.title.clone(), strong),
                Span::styled(position, dim),
            ]));
            lines.push(Line::from(Span::styled(
                format!(" via {}", target.route),
                dim,
            )));
        }
        None => lines.push(Line::from(Span::styled(" No agent sessions", dim))),
    }
    lines.push(Line::from(""));
    let body: Vec<&str> = state.text.split('\n').collect();
    let last = body.len() - 1;
    for (i, line) in body.iter().enumerate() {
        let mut spans = vec![Span::styled(format!(" {line}"), text)];
        if i == last {
            spans.push(cursor.clone());
        }
        lines.push(Line::from(spans));
    }
    // Keep the end of a long message (where the cursor is) in view, but
    // never scroll the target lines away.
    let room = usize::from(inner.height);
    if lines.len() > room && room > 3 {
        let skip = lines.len() - room;
        lines.drain(3..3 + skip.min(lines.len() - 3));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}
