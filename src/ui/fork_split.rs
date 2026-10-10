//! Drawing for split view (DESIGN.md §4.1): one bordered tile per visible
//! session — header `project › feature › agent · status` in the top border,
//! bright border on the focused tile, the theme's warning colour on tiles
//! whose agent needs you — and a one-line key-hint footer.

use ratatui::{
    Frame,
    layout::{Position, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
};

use crate::app::fork_split::{SplitViewState, TileStatusKind};
use crate::app::{App, AppMode};
use crate::fork::split;

pub fn draw(frame: &mut Frame, app: &App) {
    let AppMode::SplitView(state) = &app.mode else {
        return;
    };
    let theme = &app.theme;
    let area = frame.area();
    // No full-screen `Clear`: `Terminal::draw` starts from a reset buffer.
    frame.render_widget(
        Block::default().style(Style::default().bg(theme.effective_bg())),
        area,
    );

    let visible = state.visible();
    // Rects come from the last tick; recompute if the frame changed since.
    let rects = if state.rects.len() == visible.len() {
        state.rects.clone()
    } else {
        split::tile_rects(split::tiles_area(area), visible.len())
    };
    let mut needs_you = 0usize;
    for tile in &state.tiles {
        if app.split_tile_status(tile).kind.needs_attention() {
            needs_you += 1;
        }
    }

    for (slot, index) in visible.clone().enumerate() {
        let Some(rect) = rects.get(slot).copied() else {
            continue;
        };
        draw_tile(frame, app, state, index, rect);
    }

    draw_footer(frame, app, state, needs_you);
    super::draw_toasts(frame, &app.toasts, theme);
}

fn draw_tile(frame: &mut Frame, app: &App, state: &SplitViewState, index: usize, rect: Rect) {
    let theme = &app.theme;
    let tile = &state.tiles[index];
    let focused = index == state.focus;
    let status = app.split_tile_status(tile);

    // The dashboard's status colours (G11): needs you = amber, working =
    // blue, finished = green, idle / stopped = grey.
    let status_color = match status.kind {
        TileStatusKind::NeedsYou => theme.status_waiting.to_color(),
        TileStatusKind::Working => theme.status_working.to_color(),
        TileStatusKind::Finished => theme.success.to_color(),
        TileStatusKind::Idle => theme.status_idle.to_color(),
        TileStatusKind::Stopped => theme.status_stopped.to_color(),
    };
    let status_glyph = match status.kind {
        TileStatusKind::NeedsYou => "!",
        TileStatusKind::Working => "●",
        TileStatusKind::Finished => "✓",
        TileStatusKind::Idle => "○",
        TileStatusKind::Stopped => "×",
    };
    let border_color = if status.kind.needs_attention() {
        theme.status_waiting.to_color()
    } else if focused {
        theme.border_focus.to_color()
    } else {
        theme.border.to_color()
    };
    let mut border_style = Style::default().fg(border_color);
    if focused {
        border_style = border_style.add_modifier(Modifier::BOLD);
    }

    let name_style = if focused {
        Style::default()
            .fg(theme.text.to_color())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.text_muted.to_color())
    };
    let title = Line::from(vec![
        Span::raw(" "),
        Span::styled(
            tile.project_name.clone(),
            Style::default().fg(theme.project_title.to_color()),
        ),
        Span::styled(" › ", name_style),
        Span::styled(
            tile.feature_label.clone(),
            Style::default().fg(theme.feature_title.to_color()),
        ),
        Span::styled(" › ", name_style),
        Span::styled(tile.label.clone(), name_style),
        Span::styled(" · ", name_style),
        Span::styled(
            format!("{status_glyph} {}", status.text),
            Style::default().fg(status_color),
        ),
        Span::raw(" "),
    ]);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if focused {
            BorderType::Thick
        } else {
            BorderType::Rounded
        })
        .border_style(border_style)
        .title(title);
    frame.render_widget(block, rect);

    let content = split::tile_content(rect);
    if content.width == 0 || content.height == 0 {
        return;
    }
    let tile_frame = &state.frames[index];
    let pane_style = Style::default()
        .fg(theme.text.to_color())
        .bg(theme.effective_bg());
    if tile_frame.dead {
        frame.render_widget(
            Paragraph::new("(this session's tmux window is gone)")
                .style(pane_style.fg(theme.text_muted.to_color())),
            content,
        );
        return;
    }
    if tile_frame.lines.is_empty() {
        frame.render_widget(
            Paragraph::new("connecting…").style(pane_style.fg(theme.text_muted.to_color())),
            content,
        );
        return;
    }
    let buf = frame.buffer_mut();
    buf.set_style(content, pane_style);
    for (i, line) in tile_frame
        .lines
        .iter()
        .take(content.height as usize)
        .enumerate()
    {
        buf.set_line(content.x, content.y + i as u16, line, content.width);
    }

    if focused
        && !app.leader_active
        && let Some((x, y)) = tile_frame.cursor
        && x < content.width
        && y < content.height
    {
        frame.set_cursor_position(Position::new(content.x + x, content.y + y));
    }
}

fn draw_footer(frame: &mut Frame, app: &App, state: &SplitViewState, needs_you: usize) {
    let theme = &app.theme;
    let area = frame.area();
    if area.height == 0 {
        return;
    }
    let footer = Rect::new(area.x, area.y + area.height - 1, area.width, 1);
    let key = Style::default()
        .fg(theme.shortcut_text.to_color())
        .bg(theme.shortcut_background.to_color())
        .add_modifier(Modifier::BOLD);
    let text = Style::default().fg(theme.text_muted.to_color());

    let mut spans = vec![
        Span::styled(
            format!(" {} ", state.title),
            Style::default()
                .fg(theme.project_title.to_color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ];
    if app.leader_active {
        spans.push(Span::styled(" Ctrl+Space ", key));
        spans.push(Span::styled(
            "  ←↑→↓ focus · z zoom · v leave · [ ] page · i needs you · Ctrl+Space send it to the agent",
            Style::default().fg(theme.warning.to_color()),
        ));
    } else {
        spans.push(Span::styled(" Ctrl+Space ←↑→↓ ", key));
        spans.push(Span::styled(" focus  ", text));
        spans.push(Span::styled(" Ctrl+Space z ", key));
        spans.push(Span::styled(" zoom  ", text));
        spans.push(Span::styled(" Ctrl+Space v ", key));
        spans.push(Span::styled(" leave  ", text));
        if state.page_count() > 1 {
            spans.push(Span::styled(" Ctrl+Space [ ] ", key));
            spans.push(Span::styled(
                format!(" page {}/{}  ", state.page() + 1, state.page_count()),
                text,
            ));
        }
        if needs_you > 0 {
            spans.push(Span::styled(" Ctrl+Space i ", key));
            spans.push(Span::styled(
                format!(" {needs_you} need you"),
                Style::default().fg(theme.warning.to_color()),
            ));
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), footer);
}
