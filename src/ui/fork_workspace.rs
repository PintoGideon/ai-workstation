//! FORK: drawing for the workspace dashboard layout (DESIGN.md §15): the
//! session sidebar on the left, the selected session live on the right.
//!
//! The sidebar walks the same `visible_items` as the tree and keeps its
//! selection, scroll offset and disclosure state, so every dashboard key
//! works unchanged. The pane is one split-view tile drawn through
//! `fork_split::draw_live_tile` from the single view's snapshot
//! (`app.pane_lines`). Nothing here calls tmux; badges come from
//! `fork::badges`.

use chrono::{DateTime, Utc};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem},
};
use unicode_width::UnicodeWidthStr;

use crate::app::fork_workspace::{FALLBACK_NOTE, WorkspaceFocus, layout};
use crate::app::{App, Selection, VisibleItem};
use crate::context_tracking::ContextBand;
use crate::fork::badges::{self, BadgeCounts, BadgeKind, SessionBadge};
use crate::theme::Theme;

use super::fork_split::{LiveTile, draw_live_tile};
use super::fork_text::fit_line;

/// Draw the workspace layout into the dashboard body. Returns `false` when
/// the tree should be drawn instead (layout off, no projects, or the
/// terminal is too narrow — then the footer gets a one-time note).
pub fn draw(frame: &mut Frame, app: &mut App, area: Rect) -> bool {
    if !app.workspace_layout_enabled() || app.store.projects.is_empty() {
        app.fork_workspace.sidebar_rect = None;
        app.fork_workspace.pane_rect = None;
        return false;
    }
    let Some(layout) = layout(area) else {
        if !app.fork_workspace.fallback_noted {
            app.fork_workspace.fallback_noted = true;
            app.message = Some(FALLBACK_NOTE.to_string());
        }
        app.fork_workspace.sidebar_rect = None;
        app.fork_workspace.pane_rect = None;
        return false;
    };
    if app.fork_workspace.fallback_noted {
        app.fork_workspace.fallback_noted = false;
        // The note was for the narrow window; it is over.
        if app.message.as_deref() == Some(FALLBACK_NOTE) {
            app.message = None;
        }
    }
    // Everything stays inside the frame, whatever the tick laid out.
    let frame_area = frame.area();
    if frame_area.intersection(layout.sidebar) != layout.sidebar
        || frame_area.intersection(layout.pane) != layout.pane
    {
        return false;
    }
    app.fork_workspace.sidebar_rect = Some(layout.sidebar);
    app.fork_workspace.pane_rect = Some(layout.pane);

    draw_sidebar(frame, app, layout.sidebar, layout.compact);
    draw_pane(frame, app, layout.pane);
    true
}

fn counts_spans(counts: &BadgeCounts, theme: &Theme) -> Vec<Span<'static>> {
    let mut spans = Vec::with_capacity(3);
    for (kind, count) in [
        (BadgeKind::NeedsYou, counts.needs_you),
        (BadgeKind::Working, counts.working),
        (BadgeKind::FinishedUnread, counts.done),
    ] {
        if count == 0 {
            continue;
        }
        if !spans.is_empty() {
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(
            format!("{}{count}", kind.glyph()),
            Style::default()
                .fg(kind.tone(theme))
                .add_modifier(Modifier::BOLD),
        ));
    }
    spans
}

fn spans_width(spans: &[Span]) -> usize {
    spans.iter().map(Span::width).sum()
}

/// `left … right`: `right` sits at the row's end when both fit, else it is
/// dropped (the name wins).
fn with_right(
    mut left: Vec<Span<'static>>,
    right: Vec<Span<'static>>,
    width: usize,
) -> Line<'static> {
    let left_width = spans_width(&left);
    let right_width = spans_width(&right);
    if !right.is_empty() && left_width + 1 + right_width <= width {
        left.push(Span::raw(" ".repeat(width - left_width - right_width)));
        left.extend(right);
    }
    Line::from(left)
}

/// The glyph in a two-column cell, so names line up whether the glyph is
/// `⛔` (two columns wide in a terminal) or `●`.
fn glyph_span(badge: &SessionBadge, theme: &Theme) -> Span<'static> {
    let glyph = badge.glyph();
    let cell = if UnicodeWidthStr::width(glyph) < 2 {
        format!("{glyph} ")
    } else {
        glyph.to_string()
    };
    Span::styled(cell, badge.style(theme))
}

fn draw_sidebar(frame: &mut Frame, app: &mut App, area: Rect, compact: bool) {
    let visible_height = usize::from(area.height.saturating_sub(2));
    let visible = app.visible_items();
    app.ensure_selection_visible_for_items(&visible, visible_height);
    let start = if visible.is_empty() {
        0
    } else {
        app.scroll_offset.min(visible.len() - 1)
    };
    let end = app.visible_window_end_for_items(&visible, visible_height);
    let slice = &visible[start..end];
    let theme = &app.theme;
    let now = Utc::now();
    // Border and one column of padding on each side.
    let row_width = usize::from(area.width.saturating_sub(4));
    let dim = super::fork_chrome::dim_style(theme);

    let mut selected_rows: Option<std::ops::Range<usize>> = None;
    let mut offset = 0usize;
    let items: Vec<ListItem> = slice
        .iter()
        .map(|item| {
            let selected = item_is_selected(&app.selection, item);
            let lines = sidebar_item_lines(app, item, selected, compact, row_width, now, dim);
            if selected {
                selected_rows = Some(offset..offset + lines.len());
            }
            offset += lines.len();
            if selected {
                ListItem::new(lines).style(Style::default().bg(theme.effective_selection_bg()))
            } else {
                ListItem::new(lines)
            }
        })
        .collect();

    let focused = app.fork_workspace.focus == WorkspaceFocus::Sidebar;
    let mut block = super::fork_chrome::padded_panel(" Sessions ", theme);
    if focused {
        block = block.border_style(Style::default().fg(theme.border_focus.to_color()));
    }
    frame.render_widget(List::new(items).block(block), area);

    if let Some(rows) = selected_rows {
        let inner_height = usize::from(area.height.saturating_sub(2));
        let bar_style = super::fork_chrome::key_style(theme);
        for row in rows.start..rows.end.min(inner_height) {
            let position = (area.x + 1, area.y + 1 + row as u16);
            if let Some(cell) = frame.buffer_mut().cell_mut(position) {
                cell.set_symbol("▎").set_style(bar_style);
            }
        }
    }
}

fn item_is_selected(selection: &Selection, item: &VisibleItem) -> bool {
    match (selection, item) {
        (Selection::Project(a), VisibleItem::Project(b)) => a == b,
        (Selection::Feature(a1, a2), VisibleItem::Feature(b1, b2)) => a1 == b1 && a2 == b2,
        (Selection::Session(a1, a2, a3), VisibleItem::Session(b1, b2, b3)) => {
            a1 == b1 && a2 == b2 && a3 == b3
        }
        _ => false,
    }
}

fn sidebar_item_lines(
    app: &App,
    item: &VisibleItem,
    selected: bool,
    compact: bool,
    row_width: usize,
    now: DateTime<Utc>,
    dim: Style,
) -> Vec<Line<'static>> {
    let theme = &app.theme;
    let text = if selected {
        Style::default().fg(theme.text.to_color())
    } else {
        dim
    };
    match item {
        VisibleItem::Project(pi) => {
            let project = &app.store.projects[*pi];
            let disclosure = if project.collapsed { "▸ " } else { "▾ " };
            let left = vec![
                Span::styled(disclosure, text),
                Span::styled(
                    project.name.clone(),
                    Style::default()
                        .fg(theme.project_title.to_color())
                        .add_modifier(Modifier::BOLD),
                ),
            ];
            let right = if compact {
                Vec::new()
            } else {
                counts_spans(&badges::project_counts(app, *pi, now), theme)
            };
            vec![fit_line(with_right(left, right, row_width), row_width)]
        }
        VisibleItem::Feature(pi, fi) => {
            let feature = &app.store.projects[*pi].features[*fi];
            let badge = badges::feature_badge(app, *pi, *fi, now);
            let disclosure = if feature.sessions.is_empty() {
                "  "
            } else if feature.collapsed {
                "▸ "
            } else {
                "▾ "
            };
            let name_style = if selected {
                Style::default()
                    .fg(theme.feature_title.to_color())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.feature_title.to_color())
            };
            let name = feature
                .nickname
                .clone()
                .unwrap_or_else(|| feature.name.clone());
            let left = vec![
                Span::styled(" ", text),
                Span::styled(disclosure, text),
                glyph_span(&badge, theme),
                Span::styled(" ", text),
                Span::styled(name, name_style),
            ];
            let right = if compact || matches!(badge.kind, BadgeKind::Idle | BadgeKind::Stopped) {
                Vec::new()
            } else {
                vec![Span::styled(badge.age(now), dim)]
            };
            vec![fit_line(with_right(left, right, row_width), row_width)]
        }
        VisibleItem::Session(pi, fi, si) => {
            let session = &app.store.projects[*pi].features[*fi].sessions[*si];
            let badge = badges::session_badge(app, *pi, *fi, *si, now);
            let label_style = if selected {
                Style::default()
                    .fg(theme.text.to_color())
                    .add_modifier(Modifier::BOLD)
            } else if session.stopped {
                dim
            } else {
                Style::default().fg(theme.text.to_color())
            };
            let left = vec![
                Span::styled("   ", text),
                glyph_span(&badge, theme),
                Span::styled(" ", text),
                Span::styled(session.label.clone(), label_style),
            ];
            let mut right = Vec::new();
            // P4: context use, toned by the configured thresholds.
            if !compact
                && !matches!(badge.kind, BadgeKind::Stopped | BadgeKind::Exited)
                && let Some((pct, band)) = app.session_context_percent(&session.id)
            {
                right.push(Span::styled(
                    format!("{pct}%"),
                    context_style(band, theme, dim),
                ));
            }
            if !compact && !matches!(badge.kind, BadgeKind::Idle | BadgeKind::Stopped) {
                if !right.is_empty() {
                    right.push(Span::raw(" "));
                }
                right.push(Span::styled(badge.age(now), dim));
            }
            let mut lines = vec![fit_line(with_right(left, right, row_width), row_width)];
            if !compact && (selected || badge.kind.needs_you()) {
                let reason_style = if badge.kind.needs_you() {
                    Style::default().fg(badge.tone(theme))
                } else {
                    Style::default().fg(theme.status_detail.to_color())
                };
                lines.push(fit_line(
                    Line::from(vec![
                        Span::raw("     "),
                        Span::styled(badge.label_compact(), reason_style),
                    ]),
                    row_width,
                ));
            }
            lines
        }
    }
}

/// The context percentage's style: dim while normal, the warning / danger
/// tone past the thresholds.
fn context_style(band: ContextBand, theme: &Theme, dim: Style) -> Style {
    match band {
        ContextBand::Normal => dim,
        ContextBand::Warning => Style::default().fg(theme.warning.to_color()),
        ContextBand::Critical => Style::default()
            .fg(theme.danger.to_color())
            .add_modifier(Modifier::BOLD),
    }
}

/// The word in the pane's badge: `⛔ waiting 0:40`, `● working 6m`.
fn badge_word(app: &App, session: &str, badge: &SessionBadge) -> &'static str {
    match badge.kind {
        BadgeKind::NeedsYou => match app.attention_for(session).map(|r| r.state) {
            Some(crate::app::attention::AttentionState::Question) => "question",
            _ => "waiting",
        },
        BadgeKind::Working => "working",
        BadgeKind::FinishedUnread | BadgeKind::FinishedRead => "finished",
        BadgeKind::Idle => "idle",
        BadgeKind::Stopped => "stopped",
        BadgeKind::Exited => "exited",
    }
}

fn draw_pane(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    let focused = app.fork_workspace.focus == WorkspaceFocus::Pane;
    let now = Utc::now();
    let Some(live) = app.fork_workspace.live.as_ref() else {
        let hint = match app.selection {
            Selection::Session(pi, fi, si) => {
                match app.session_badge_at(pi, fi, si).map(|b| b.kind) {
                    Some(BadgeKind::Stopped) => "This session is stopped — c starts it",
                    _ => "Nothing to show for this row",
                }
            }
            Selection::Feature(..) => "No running agent in this feature — c starts it",
            Selection::Project(..) => "No running agent in this project — n adds a feature",
        };
        let empty = [
            Line::from(hint.to_string()),
            Line::from(""),
            Line::from("j/k pick a session · Tab or Enter type into it · b tree layout"),
        ];
        draw_live_tile(
            frame,
            theme,
            area,
            LiveTile {
                title: Line::from(Span::styled(
                    " Live pane ",
                    Style::default().fg(theme.text_muted.to_color()),
                )),
                title_right: None,
                focused: false,
                needs_you: false,
                lines: &[],
                cursor: None,
                dead: false,
                empty: Some(&empty),
            },
        );
        return;
    };

    let badge = app
        .session_badge_for_window(&live.session, &live.window)
        .unwrap_or_else(|| SessionBadge::new(BadgeKind::Stopped, now, None));
    let name_style = if focused {
        Style::default()
            .fg(theme.text.to_color())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.text_muted.to_color())
    };
    let title = fit_line(
        Line::from(vec![
            Span::raw(" "),
            Span::styled(
                live.project_name.clone(),
                Style::default().fg(theme.project_title.to_color()),
            ),
            Span::styled(" › ", name_style),
            Span::styled(
                live.feature_label.clone(),
                Style::default().fg(theme.feature_title.to_color()),
            ),
            Span::styled(" › ", name_style),
            Span::styled(live.label.clone(), name_style),
            Span::raw(" "),
        ]),
        usize::from(area.width.saturating_sub(2)) / 2,
    );
    let badge_text = format!(
        " {} {} {} ",
        badge.glyph(),
        badge_word(app, &live.session, &badge),
        badge.age(now)
    );
    let title_right = (UnicodeWidthStr::width(badge_text.as_str()) + title.width() + 2
        <= usize::from(area.width))
    .then(|| {
        Line::from(Span::styled(
            badge_text,
            Style::default().fg(badge.tone(theme)),
        ))
    });
    let cursor = (focused && !app.leader_active)
        .then_some(app.tmux_cursor)
        .flatten();
    draw_live_tile(
        frame,
        theme,
        area,
        LiveTile {
            title,
            title_right,
            focused,
            needs_you: badge.kind.needs_you(),
            lines: &app.pane_lines,
            cursor,
            dead: false,
            empty: None,
        },
    );
}

/// The footer's key hints for the layout: what applies to the focused half.
pub(crate) fn footer_hints(app: &App) -> Line<'static> {
    let theme = &app.theme;
    let key = super::fork_chrome::key_style(theme);
    let hint = |text: &'static str| Span::raw(text);
    if app.fork_workspace.focus == WorkspaceFocus::Pane {
        let mut spans = vec![
            Span::styled(" Ctrl+Q", key),
            hint(" sidebar  "),
            Span::styled("Ctrl+Space", key),
            hint(" commands  "),
            Span::styled("Ctrl+Space z", key),
            hint(" full view  "),
            Span::styled("Ctrl+Space i", key),
            hint(" next needing you"),
        ];
        if app.leader_active {
            spans = vec![
                Span::styled(" Ctrl+Space", key),
                Span::styled(
                    "  Enter attach · z full view · q sidebar · i needs you · f final review · p prompts · t/T next/prev session · Ctrl+Space send it to the agent",
                    Style::default().fg(theme.warning.to_color()),
                ),
            ];
        }
        return Line::from(spans);
    }
    let on_session = matches!(app.selection, Selection::Session(..));
    let mut spans = if app.pending_permission_for_selection().is_some() {
        super::fork_permissions::row_hint_spans(theme)
    } else {
        Vec::new()
    };
    if app.desired_live_target_exists() {
        spans.extend([Span::styled(" Enter", key), hint(" attach  ")]);
    }
    spans.extend([Span::styled("Tab", key), hint(" type here  ")]);
    if on_session {
        spans.extend([Span::styled("o", key), hint(" full view  ")]);
    } else if !app.desired_live_target_exists() {
        spans.extend([Span::styled("Enter", key), hint(" expand  ")]);
    }
    spans.extend([
        Span::styled("i", key),
        hint(" next needing you  "),
        Span::styled("v", key),
        hint(" tiles  "),
        Span::styled("n", key),
        hint(" feature  "),
        Span::styled("c", key),
        hint(" start  "),
        Span::styled("x", key),
        hint(" stop  "),
        Span::styled("b", key),
        hint(" tree  "),
        Span::styled("?", key),
        hint(" help  "),
        Span::styled("q", key),
        hint(" quit"),
    ]);
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::attention::{AttentionSource, AttentionState};
    use crate::app::fork_workspace::SELECTION_DEBOUNCE;
    use crate::fork::config::DashboardLayout;
    use crate::project::AgentKind;
    use crate::traits::MockTmuxOps;
    use crate::ui::fork_bench::realistic_app;
    use ratatui::{Terminal, backend::TestBackend};

    /// Rows as text. A wide glyph (`⛔`) is followed by its continuation
    /// cell, which the backend reports as a blank; it is dropped here so
    /// assertions read like a terminal.
    fn rows(app: &mut App, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| crate::ui::draw(frame, app)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                let mut row = String::new();
                let mut skip = 0usize;
                for x in 0..width {
                    if skip > 0 {
                        skip -= 1;
                        continue;
                    }
                    let symbol = buffer[(x, y)].symbol();
                    skip = UnicodeWidthStr::width(symbol).saturating_sub(1);
                    row.push_str(symbol);
                }
                row
            })
            .collect()
    }

    fn workspace(width: u16, height: u16) -> App {
        let mut app = realistic_app(MockTmuxOps::new());
        app.config.aiw.dashboard_layout = DashboardLayout::Workspace;
        app.viewport_cols = width;
        app.viewport_total_rows = height;
        app.viewport_rows = height.saturating_sub(3);
        app
    }

    fn settle(app: &mut App) {
        app.workspace_tick();
        std::thread::sleep(SELECTION_DEBOUNCE + std::time::Duration::from_millis(20));
        app.workspace_tick();
    }

    #[test]
    fn the_layout_fits_every_width_and_falls_back_below_72() {
        for (width, height, expect_sidebar) in [
            (60u16, 24u16, false),
            (71, 24, false),
            (72, 24, true),
            (80, 24, true),
            (120, 40, true),
            (200, 60, true),
        ] {
            let mut app = workspace(width, height);
            app.selection = Selection::Session(0, 0, 0);
            settle(&mut app);
            let screen = rows(&mut app, width, height);
            let has_sidebar = screen.iter().any(|row| row.contains("╭ Sessions"));
            assert_eq!(
                has_sidebar,
                expect_sidebar,
                "{width}x{height}:\n{}",
                screen.join("\n")
            );
            for (y, row) in screen.iter().enumerate() {
                assert_eq!(
                    UnicodeWidthStr::width(row.as_str()),
                    usize::from(width),
                    "{width}x{height} row {y} overflows: {row}"
                );
            }
            if expect_sidebar {
                assert!(
                    screen
                        .iter()
                        .any(|row| row.contains("harbor-api › harbor-12")),
                    "{width}x{height}:\n{}",
                    screen.join("\n")
                );
                assert!(screen[height as usize - 1].contains("Tab type here"));
                assert!(app.fork_workspace.pane_rect.is_some());
            } else {
                assert!(screen.iter().any(|row| row.contains("╭ Projects")));
                assert_eq!(app.message.as_deref(), Some(FALLBACK_NOTE));
                assert!(
                    screen[height as usize - 2].contains("72 columns"),
                    "{}",
                    screen[height as usize - 2]
                );
            }
        }
    }

    #[test]
    fn the_fallback_note_is_written_once_per_shrink() {
        let mut app = workspace(60, 24);
        rows(&mut app, 60, 24);
        assert_eq!(app.message.as_deref(), Some(FALLBACK_NOTE));
        app.message = None;
        rows(&mut app, 60, 24);
        assert_eq!(app.message, None, "not repeated while still narrow");
        app.message = Some(FALLBACK_NOTE.to_string());
        app.viewport_cols = 120;
        rows(&mut app, 120, 40);
        assert_eq!(
            app.message, None,
            "the note goes when the layout fits again"
        );
        app.viewport_cols = 60;
        rows(&mut app, 60, 24);
        assert_eq!(
            app.message.as_deref(),
            Some(FALLBACK_NOTE),
            "noted again after growing and shrinking"
        );
    }

    #[test]
    fn rows_show_badges_and_the_pane_border_carries_the_needs_you_tone() {
        let mut app = workspace(120, 40);
        // billing-12 has the question; select its claude session.
        app.selection = Selection::Session(1, 0, 0);
        settle(&mut app);
        let screen = rows(&mut app, 120, 40);
        let text = screen.join("\n");
        assert!(text.contains("⛔ claude"), "{text}");
        assert!(text.contains("question"), "{text}");
        // Narrow glyphs sit in a two-column cell so names align with `⛔`.
        assert!(text.contains("✓  claude"), "harbor-13 finished: {text}");
        assert!(text.contains("●  claude"), "harbor-12 working: {text}");
        assert!(text.contains("■  claude"), "stopped feature rows: {text}");
        // Project rows roll their features up.
        assert!(
            screen
                .iter()
                .any(|row| row.contains("billing-app") && row.contains("⛔1")),
            "{text}"
        );
        // The pane title and badge.
        assert!(text.contains("billing-app › billing-12 › claude"), "{text}");
        assert!(text.contains("⛔ question"), "{text}");
        let title_row = screen
            .iter()
            .find(|row| row.contains("⛔ question"))
            .unwrap();
        assert!(title_row.ends_with('╮'), "{title_row}");
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|frame| crate::ui::draw(frame, &mut app))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let y = screen
            .iter()
            .position(|row| row.contains("⛔ question"))
            .unwrap() as u16;
        assert_eq!(
            buffer[(119, y)].style().fg,
            Some(app.theme.status_waiting.to_color()),
            "pane border takes the needs-you tone"
        );
    }

    #[test]
    fn rows_show_the_context_percentage_in_the_thresholds_tone() {
        let mut app = workspace(120, 40);
        app.selection = Selection::Session(0, 0, 0);
        let id = app.store.projects[0].features[0].sessions[0].id.clone();
        assert!(app.handle_status_line_message(&serde_json::json!({
            "type": "status_line", "aiw_feature_session_id": id, "context_used_pct": 88
        })));
        settle(&mut app);
        let screen = rows(&mut app, 120, 40);
        let row = screen
            .iter()
            .find(|row| row.contains("▎") && row.contains("claude"))
            .unwrap()
            .clone();
        assert!(row.contains("88%"), "{row}");
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|frame| crate::ui::draw(frame, &mut app))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let y = screen.iter().position(|r| r == &row).unwrap() as u16;
        // A cell per char: count chars, not bytes, up to the percentage.
        let x = row[..row.find("88%").unwrap()].chars().count() as u16;
        assert_eq!(
            buffer[(x, y)].style().fg,
            Some(app.theme.danger.to_color()),
            "88% is past the critical threshold"
        );
    }

    #[test]
    fn compact_sidebar_shows_glyph_and_name_only() {
        let mut app = workspace(80, 24);
        app.selection = Selection::Session(1, 0, 0);
        settle(&mut app);
        let screen = rows(&mut app, 80, 24);
        let text = screen.join("\n");
        assert!(text.contains("⛔ claude"), "{text}");
        assert!(
            !text.contains("permission"),
            "no second line in the compact sidebar: {text}"
        );
        assert!(screen.iter().any(|row| row.starts_with("│▎")), "{text}");
    }

    #[test]
    fn focused_pane_shows_the_cursor_and_its_own_hints() {
        let mut app = workspace(120, 40);
        app.selection = Selection::Session(0, 0, 0);
        settle(&mut app);
        app.pane_lines = vec![Line::from("❯ ")];
        app.tmux_cursor = Some((2, 0));
        app.workspace_focus_pane();
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal
            .draw(|frame| crate::ui::draw(frame, &mut app))
            .unwrap();
        let pane = app.fork_workspace.pane_rect.unwrap();
        let cursor = terminal.get_cursor_position().unwrap();
        assert_eq!((cursor.x, cursor.y), (pane.x + 1 + 2, pane.y + 1));
        let screen = rows(&mut app, 120, 40);
        assert!(screen[39].contains("Ctrl+Q sidebar"), "{}", screen[39]);
        // Back on the sidebar: no cursor, sidebar hints.
        app.workspace_focus_sidebar();
        let screen = rows(&mut app, 120, 40);
        assert!(screen[39].contains("Tab type here"), "{}", screen[39]);
    }

    #[test]
    fn an_unread_finished_turn_dims_once_the_pane_is_focused() {
        let mut app = workspace(120, 40);
        app.selection = Selection::Session(0, 1, 0); // harbor-13: finished
        settle(&mut app);
        let bold_check = |app: &mut App| {
            let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
            terminal.draw(|frame| crate::ui::draw(frame, app)).unwrap();
            let buffer = terminal.backend().buffer();
            let screen = rows(app, 120, 40);
            let y = screen
                .iter()
                .position(|row| row.contains("✓  claude"))
                .unwrap() as u16;
            let x = screen[y as usize].find('✓').unwrap();
            let x = UnicodeWidthStr::width(&screen[y as usize][..x]) as u16;
            let style = buffer[(x, y)].style();
            (style.add_modifier.contains(Modifier::BOLD), style.fg)
        };
        let (bold, color) = bold_check(&mut app);
        assert!(bold);
        assert_eq!(color, Some(app.theme.success.to_color()));
        app.workspace_focus_pane();
        let (bold, color) = bold_check(&mut app);
        assert!(!bold);
        assert_eq!(color, Some(app.theme.text_muted.to_color()));
        // The next finished turn is bright again.
        app.record_attention(
            "aiw-harbor-13",
            &AgentKind::Claude,
            AttentionState::Question,
            AttentionSource::Hook,
        );
        app.record_attention(
            "aiw-harbor-13",
            &AgentKind::Claude,
            AttentionState::CompletedAwaitingReview,
            AttentionSource::Hook,
        );
        let (bold, _) = bold_check(&mut app);
        assert!(bold);
    }

    #[test]
    fn pills_tree_and_sidebar_counts_agree() {
        let mut app = workspace(200, 60);
        settle(&mut app);
        let now = Utc::now();
        let total = badges::total_counts(&app, now);
        let pills = crate::ui::fork_chrome::header_pills(&app).unwrap();
        assert_eq!(
            (pills.needs_you, pills.working, pills.done),
            (total.needs_you, total.working, total.done)
        );
        let summed = (0..app.store.projects.len())
            .map(|pi| badges::project_counts(&app, pi, now))
            .fold(BadgeCounts::default(), |mut acc, c| {
                acc.needs_you += c.needs_you;
                acc.working += c.working;
                acc.done += c.done;
                acc
            });
        assert_eq!(summed, total);
        let screen = rows(&mut app, 200, 60);
        assert!(
            screen[1].contains(&format!(
                "⛔ {} · ● {} · ✓ {}",
                total.needs_you, total.working, total.done
            )),
            "{}",
            screen[1]
        );
        // The tree draws the same glyphs for the same rows.
        app.config.aiw.dashboard_layout = DashboardLayout::Tree;
        let tree = rows(&mut app, 200, 60);
        assert!(tree[1].contains(&format!(
            "⛔ {} · ● {} · ✓ {}",
            total.needs_you, total.working, total.done
        )));
        let tree_text = tree.join("\n");
        assert!(tree_text.contains("⛔ ▾ billing-12"), "{tree_text}");
        assert!(tree_text.contains("✓ ▾ harbor-13"), "{tree_text}");
    }

    #[test]
    fn workspace_frame_makes_no_tmux_calls() {
        let mut app = workspace(200, 60);
        app.selection = Selection::Session(0, 0, 0);
        settle(&mut app);
        let mut terminal = Terminal::new(TestBackend::new(200, 60)).unwrap();
        for _ in 0..3 {
            terminal
                .draw(|frame| crate::ui::draw(frame, &mut app))
                .unwrap();
        }
        app.workspace_focus_pane();
        terminal
            .draw(|frame| crate::ui::draw(frame, &mut app))
            .unwrap();
    }
}
