//! FORK: the "Queue a follow-up" dialog (P7, DESIGN.md §16): the queued
//! items above, a multi-line editor below, like the compose box.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use super::editor_view::{count_wrapped_editor_lines, editor_lines, sync_editor_scroll};
use crate::app::fork_followups::{FollowupFocus, FollowupQueueState};
use crate::fork::followups::Followup;
use crate::theme::Theme;

/// The footer line while the dialog is up.
pub(crate) fn dialog_hints(theme: &Theme, focus: FollowupFocus) -> Line<'static> {
    let key = crate::ui::fork_chrome::key_style(theme);
    match focus {
        FollowupFocus::Editor => Line::from(vec![
            Span::styled(" Enter", key),
            Span::raw(" queue  "),
            Span::styled("Shift+Enter", key),
            Span::raw(" newline  "),
            Span::styled("Tab", key),
            Span::raw(" the queue  "),
            Span::styled("Esc", key),
            Span::raw(" close"),
        ]),
        FollowupFocus::List => Line::from(vec![
            Span::styled(" j/k", key),
            Span::raw(" pick  "),
            Span::styled("J/K", key),
            Span::raw(" move  "),
            Span::styled("d", key),
            Span::raw(" drop  "),
            Span::styled("Enter", key),
            Span::raw(" edit  "),
            Span::styled("Tab", key),
            Span::raw(" the editor  "),
            Span::styled("Esc", key),
            Span::raw(" close"),
        ]),
    }
}

pub fn draw_followup_queue(
    frame: &mut Frame,
    queue: &[Followup],
    theme: &Theme,
    state: &mut FollowupQueueState,
) {
    let area = frame.area();
    if area.width < 20 || area.height < 10 {
        return;
    }
    let width = (area.width * 7 / 10).clamp(40, 100).min(area.width);
    let height = (area.height * 7 / 10).clamp(12, 30).min(area.height);
    let rect = crate::ui::dialog_rect(area, width, height);
    crate::ui::draw_modal_overlay(frame, rect, theme);

    let block = Block::default()
        .title(format!(" Queue a follow-up → {} ", state.title))
        .title_bottom(dialog_hints(theme, state.focus))
        .borders(Borders::ALL)
        .style(Style::default().bg(theme.effective_bg()))
        .border_style(Style::default().fg(theme.primary.to_color()));
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);

    let list_rows = (queue.len() as u16).clamp(1, 6) + 1;
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(list_rows),
            Constraint::Min(3),
        ])
        .split(inner);

    let muted = Style::default().fg(theme.text_muted.to_color());
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                "Delivered one at a time, when the agent finishes its current turn (its Stop hook asks for the next). Nothing is sent on its own.",
                muted,
            )),
        ])
        .wrap(Wrap { trim: false }),
        chunks[0],
    );

    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        if queue.is_empty() {
            "Queued: nothing yet".to_string()
        } else {
            format!("Queued ({}):", queue.len())
        },
        Style::default()
            .fg(theme.text.to_color())
            .add_modifier(Modifier::BOLD),
    ))];
    let list_focused = state.focus == FollowupFocus::List;
    let selected = state.selected.min(queue.len().saturating_sub(1));
    let visible = usize::from(list_rows.saturating_sub(1));
    let start = selected.saturating_sub(visible.saturating_sub(1));
    for (index, followup) in queue.iter().enumerate().skip(start).take(visible) {
        let is_selected = list_focused && index == selected;
        let marker = if is_selected { "▸ " } else { "  " };
        let text_style = if is_selected {
            Style::default()
                .fg(theme.text.to_color())
                .bg(theme.effective_selection_bg())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text.to_color())
        };
        let width = usize::from(chunks[1].width).saturating_sub(8);
        lines.push(Line::from(vec![
            Span::styled(format!("{marker}{}. ", index + 1), muted),
            Span::styled(followup.preview(width), text_style),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), chunks[1]);

    draw_editor(frame, state, theme, chunks[2]);
}

fn draw_editor(frame: &mut Frame, state: &mut FollowupQueueState, theme: &Theme, area: Rect) {
    let editor_focused = state.focus == FollowupFocus::Editor;
    let block = Block::default()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(if editor_focused {
            theme.border_focus.to_color()
        } else {
            theme.border.to_color()
        }))
        .title(Span::styled(
            " Next instruction ",
            Style::default().fg(theme.text_muted.to_color()),
        ));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }
    let lines = editor_lines(
        &state.editor,
        theme,
        "Type the next instruction — Enter queues it, Shift+Enter for a newline.",
    );
    let wrap_width = usize::from(inner.width).max(1);
    let total = count_wrapped_editor_lines(&lines, wrap_width);
    sync_editor_scroll(
        &state.editor,
        &mut state.scroll_offset,
        &mut state.sync_scroll_to_cursor,
        usize::from(inner.height),
        wrap_width,
        total,
    );
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((state.scroll_offset.min(u16::MAX as usize) as u16, 0)),
        inner,
    );
}

#[cfg(test)]
mod tests {
    use crate::app::{AppMode, Selection};
    use crate::project::SessionKind;

    #[test]
    fn the_dialog_lists_the_queue_and_names_the_session() {
        let mut app = crate::app::fork_workspace::workspace_app(&[SessionKind::Claude]);
        app.selection = Selection::Session(0, 0, 0);
        let id = app.store.projects[0].features[0].sessions[0].id.clone();
        app.queue_followup(&id, "rebase on main").unwrap();
        app.queue_followup(&id, "then run the tests\nand report")
            .unwrap();
        assert!(app.open_followup_queue_for_selection());
        let backend = ratatui::backend::TestBackend::new(120, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                if let AppMode::FollowupQueue(state) = &mut app.mode {
                    let queue = app.followups.get(&id).map(Vec::as_slice).unwrap_or(&[]);
                    super::draw_followup_queue(frame, queue, &app.theme, state);
                }
            })
            .unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(
            text.contains("Queue a follow-up → RS › hx-12 › Claude 1"),
            "{text}"
        );
        assert!(text.contains("1. rebase on main"));
        assert!(text.contains("2. then run the tests…"));
        assert!(text.contains("Nothing is sent on its own"));
    }
}
