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

use crate::app::fork_split::SplitViewState;
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
        if app.split_tile_status(tile).kind.needs_you() {
            needs_you += 1;
        }
    }

    for (slot, index) in visible.clone().enumerate() {
        let Some(rect) = rects.get(slot).copied() else {
            continue;
        };
        // A stale layout (resize before the tick recomputed it) must never
        // draw outside the frame: skip any tile that does not fit inside it.
        if rect.is_empty() || area.intersection(rect) != rect {
            continue;
        }
        draw_tile(frame, app, state, index, rect);
    }

    draw_footer(frame, app, state, needs_you);
    super::draw_toasts(frame, frame.area(), &app.toasts, theme);
}

fn draw_tile(frame: &mut Frame, app: &App, state: &SplitViewState, index: usize, rect: Rect) {
    let theme = &app.theme;
    let tile = &state.tiles[index];
    let focused = index == state.focus;
    // The session's badge (DESIGN.md §15): its glyph, tone and label, so a
    // tile title and a sidebar row say the same thing.
    let status = app.split_tile_status(tile);
    let status_label = status.label(chrono::Utc::now());

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
            format!("{} {status_label}", status.glyph()),
            Style::default().fg(status.tone(theme)),
        ),
        Span::raw(" "),
    ]);

    let tile_frame = &state.frames[index];
    let cursor = (focused && !app.leader_active)
        .then_some(tile_frame.cursor)
        .flatten();
    draw_live_tile(
        frame,
        theme,
        rect,
        LiveTile {
            title,
            title_right: None,
            focused,
            needs_you: status.kind.needs_you(),
            lines: &tile_frame.lines,
            cursor,
            dead: tile_frame.dead,
            empty: None,
        },
    );
}

/// What [`draw_live_tile`] draws: one bordered live terminal.
pub(crate) struct LiveTile<'a> {
    /// The top-left title (project › feature › session · status).
    pub title: Line<'static>,
    /// An optional right-aligned top title (the workspace pane's badge).
    pub title_right: Option<Line<'static>>,
    pub focused: bool,
    /// Amber border, as a tile whose agent needs you.
    pub needs_you: bool,
    pub lines: &'a [Line<'static>],
    /// Drawn only when given (the focused tile's).
    pub cursor: Option<(u16, u16)>,
    /// The tmux window is gone.
    pub dead: bool,
    /// Text to show instead of a terminal (an empty state), dim and centred.
    pub empty: Option<&'a [Line<'static>]>,
}

/// The one live-tile renderer: split-view tiles and the workspace layout's
/// pane (DESIGN.md §15) draw through here, so borders, focus and the
/// needs-you tone cannot drift apart.
pub(crate) fn draw_live_tile(
    frame: &mut Frame,
    theme: &crate::theme::Theme,
    rect: Rect,
    tile: LiveTile<'_>,
) {
    let border_color = if tile.needs_you {
        theme.status_waiting.to_color()
    } else if tile.focused {
        theme.border_focus.to_color()
    } else {
        theme.border.to_color()
    };
    let mut border_style = Style::default().fg(border_color);
    if tile.focused {
        border_style = border_style.add_modifier(Modifier::BOLD);
    }
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(if tile.focused {
            BorderType::Thick
        } else {
            BorderType::Rounded
        })
        .border_style(border_style)
        .title(tile.title);
    if let Some(right) = tile.title_right {
        block = block.title_top(right.right_aligned());
    }
    frame.render_widget(block, rect);

    let content = split::tile_content(rect);
    if content.width == 0 || content.height == 0 {
        return;
    }
    let pane_style = Style::default()
        .fg(theme.text.to_color())
        .bg(theme.effective_bg());
    if let Some(empty) = tile.empty {
        let height = (empty.len() as u16).min(content.height);
        let top = content.y + content.height.saturating_sub(height) / 2;
        frame.render_widget(
            Paragraph::new(empty.to_vec())
                .alignment(ratatui::layout::Alignment::Center)
                .style(pane_style.fg(theme.text_muted.to_color())),
            Rect::new(content.x, top, content.width, height),
        );
        return;
    }
    if tile.dead {
        frame.render_widget(
            Paragraph::new("(this session's tmux window is gone)")
                .style(pane_style.fg(theme.text_muted.to_color())),
            content,
        );
        return;
    }
    if tile.lines.is_empty() {
        frame.render_widget(
            Paragraph::new("connecting…").style(pane_style.fg(theme.text_muted.to_color())),
            content,
        );
        return;
    }
    let buf = frame.buffer_mut();
    buf.set_style(content, pane_style);
    for (i, line) in tile.lines.iter().take(content.height as usize).enumerate() {
        buf.set_line(content.x, content.y + i as u16, line, content.width);
    }

    if let Some((x, y)) = tile.cursor
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
    // Drop whole hints from the end before they would be cut mid-word.
    let line = super::fork_text::fit_hints(Line::from(spans), footer.width as usize, |span| {
        span.style == key
    });
    frame.render_widget(Paragraph::new(line), footer);
}
