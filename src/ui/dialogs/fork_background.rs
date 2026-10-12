//! FORK: dialogs for background Claude sessions (DESIGN.md §19.7): the
//! first prompt after naming one in the `s` picker, and the `claude rm`
//! confirmation (with Claude's own refusal when it refuses).

use ratatui::{
    Frame,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::app::fork_background::{BackgroundPromptState, BackgroundRemoveState};
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

fn frame_block(title: String, hints: Line<'static>, theme: &Theme) -> Block<'static> {
    Block::default()
        .title(title)
        .title_bottom(hints)
        .borders(Borders::ALL)
        .style(Style::default().bg(theme.effective_bg()))
        .border_style(Style::default().fg(theme.primary.to_color()))
}

/// The first prompt of a background Claude session.
pub fn draw_background_prompt(frame: &mut Frame, state: &BackgroundPromptState, theme: &Theme) {
    let area = frame.area();
    if area.width < 30 || area.height < 9 {
        return;
    }
    let width = (area.width * 7 / 10).clamp(44, 96).min(area.width);
    let height = (state.text.lines().count() as u16 + 8).clamp(10, area.height.min(24));
    let rect = crate::ui::dialog_rect(area, width, height);
    crate::ui::draw_modal_overlay(frame, rect, theme);
    let block = frame_block(
        format!(" Background: {} ", state.label),
        key_hints(
            theme,
            &[
                ("Enter", "start"),
                ("Alt+Enter", "newline"),
                ("Ctrl+U", "clear"),
                ("Esc", "cancel"),
            ],
        ),
        theme,
    );
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);

    let dim = Style::default().fg(theme.text_muted.to_color());
    let text = Style::default().fg(theme.text.to_color());
    let cursor = Span::styled("█", Style::default().fg(theme.primary.to_color()));
    let mut lines = vec![
        Line::from(Span::styled(
            " Runs under Claude's own supervisor (claude --bg): it keeps working",
            dim,
        )),
        Line::from(Span::styled(
            " when AIW or the terminal closes. Enter on its row attaches.",
            dim,
        )),
        Line::from(Span::styled(
            " First prompt (empty: it waits for you):",
            dim,
        )),
        Line::from(""),
    ];
    let body: Vec<&str> = state.text.split('\n').collect();
    let last = body.len() - 1;
    for (i, line) in body.iter().enumerate() {
        let mut spans = vec![Span::styled(format!(" {line}"), text)];
        if i == last {
            spans.push(cursor.clone());
        }
        lines.push(Line::from(spans));
    }
    let room = usize::from(inner.height);
    if lines.len() > room && room > 5 {
        let skip = lines.len() - room;
        lines.drain(4..4 + skip.min(lines.len() - 4));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

/// The `claude rm` confirmation.
pub fn draw_background_remove(frame: &mut Frame, state: &BackgroundRemoveState, theme: &Theme) {
    let area = frame.area();
    if area.width < 30 || area.height < 8 {
        return;
    }
    let width = (area.width * 7 / 10).clamp(44, 96).min(area.width);
    let height = if state.refusal.is_some() { 14 } else { 9 }.min(area.height);
    let rect = crate::ui::dialog_rect(area, width, height);
    crate::ui::draw_modal_overlay(frame, rect, theme);
    let hints = if state.running {
        key_hints(theme, &[("Esc", "close")])
    } else if state.refusal.is_some() {
        key_hints(theme, &[("y", "discard and remove"), ("n", "keep")])
    } else {
        key_hints(theme, &[("y", "remove"), ("n", "keep")])
    };
    let block = frame_block(format!(" Remove {} ", state.label), hints, theme);
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);

    let dim = Style::default().fg(theme.text_muted.to_color());
    let text = Style::default().fg(theme.text.to_color());
    let warn = Style::default()
        .fg(theme.warning.to_color())
        .add_modifier(Modifier::BOLD);
    let mut lines = Vec::new();
    match (&state.refusal, state.running) {
        (_, true) => lines.push(Line::from(Span::styled(" Removing …", dim))),
        (None, false) => {
            lines.push(Line::from(Span::styled(
                " Runs claude rm: Claude drops the session from its list.",
                text,
            )));
            lines.push(Line::from(Span::styled(
                " Its transcript stays on disk (claude --resume finds it).",
                dim,
            )));
        }
        (Some(refusal), false) => {
            lines.push(Line::from(Span::styled(" Claude refused:", warn)));
            lines.push(Line::from(Span::styled(format!(" {}", refusal.text), text)));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!(" y runs: claude rm … {}", refusal.flags.join(" ")),
                warn,
            )));
            lines.push(Line::from(Span::styled(
                " That discards the work Claude named above.",
                dim,
            )));
        }
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}
