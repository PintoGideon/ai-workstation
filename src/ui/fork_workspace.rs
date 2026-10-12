//! FORK: drawing the cockpit sidebar (DESIGN.md §18): the session list,
//! with the info block under it. The agent itself is tmux's business — the
//! real pane beside this one — so nothing here captures or renders a
//! screen.
//!
//! The sidebar walks the same `visible_items` as the old tree and keeps its
//! selection, scroll offset and disclosure state, so every dashboard key
//! works unchanged. Nothing here calls tmux; badges come from
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

use crate::app::{App, Selection, VisibleItem};
use crate::context_tracking::ContextBand;
use crate::fork::badges::{self, BadgeCounts, BadgeKind, SessionBadge};
use crate::fork::organize::{FeatureGroup, FeatureSlot};
use crate::theme::Theme;

use super::fork_text::fit_line;

/// Draw the sidebar into the dashboard body. Returns `false` when there is
/// nothing to draw (no projects — the empty-dashboard note is drawn then).
pub fn draw(frame: &mut Frame, app: &mut App, area: Rect) -> bool {
    if !app.workspace_active() {
        app.fork_workspace.sidebar_rect = None;
        return false;
    }
    app.fork_workspace.sidebar_rect = Some(area);
    let compact = app.workspace_compact();
    // The info block takes the bottom rows when the sidebar is tall enough
    // and a session is selected (or shown for a feature row).
    let target = app.session_info_target();
    let mut info_rows = super::fork_session_info::rows(area.height, target.is_some());
    // FORK: §19.4 — one more row for a note.
    if info_rows > 0 && target.is_some_and(|t| app.session_info_has_note(t)) {
        info_rows += 1;
    }
    let list_area = Rect::new(area.x, area.y, area.width, area.height - info_rows);
    draw_sidebar(frame, app, list_area, compact);
    if let Some(target) = target
        && info_rows > 0
    {
        let info_area = Rect::new(area.x, area.y + list_area.height, area.width, info_rows);
        super::fork_session_info::draw(frame, app, info_area, target);
    }
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

    let block = super::fork_chrome::padded_panel(" Sessions ", theme);
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
            let mut lines = vec![fit_line(with_right(left, right, row_width), row_width)];
            if project.features.is_empty() && !project.collapsed {
                lines.push(fit_line(
                    Line::from(vec![
                        Span::raw("   "),
                        Span::styled("no features yet — n adds one", dim),
                    ]),
                    row_width,
                ));
            }
            if !project.collapsed && !compact {
                lines.extend(external_lines(
                    &app.external_sessions_for_project(&project.name),
                    "  ",
                    row_width,
                    dim,
                ));
                // FORK: §19.6 — Claude worktrees no feature claims.
                lines.extend(claude_worktree_lines(
                    &app.claude_worktrees_for_project(&project.name),
                    "  ",
                    row_width,
                    dim,
                ));
            }
            lines
        }
        VisibleItem::Feature(pi, fi) => {
            let feature = &app.store.projects[*pi].features[*fi];
            // FORK: §19.4 — the feature's user group, if any.
            let slot = app.feature_slot(*pi, *fi);
            let group = app.feature_group(*pi, *fi);
            if slot == FeatureSlot::CollapsedGroup
                && let Some(group) = group
            {
                return vec![group_line(
                    app, *pi, group, true, selected, row_width, now, dim,
                )];
            }
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
            let mut left = vec![
                group_bar(group, text),
                Span::styled(disclosure, text),
                glyph_span(&badge, theme),
                Span::styled(" ", text),
                Span::styled(name, name_style),
            ];
            // FORK: §19.4 — the feature has a note.
            if app.feature_note(&feature.id).is_some() {
                left.push(Span::styled(" ✎", dim));
            }
            let right = if compact || matches!(badge.kind, BadgeKind::Idle | BadgeKind::Stopped) {
                Vec::new()
            } else {
                vec![Span::styled(badge.age(now), dim)]
            };
            let mut lines = Vec::new();
            // FORK: §19.4 — an expanded group's header above its first
            // feature.
            if !compact
                && slot == FeatureSlot::GroupHead
                && let Some(group) = group
            {
                lines.push(group_line(
                    app, *pi, group, false, false, row_width, now, dim,
                ));
            }
            // FORK: §19.3 — a fan-out group's header, with how many of its
            // siblings need you.
            if !compact && let Some((header, needs_you)) = app.fan_out_header(*pi, *fi, now) {
                let right = if needs_you > 0 {
                    vec![Span::styled(
                        format!("{}{needs_you}", BadgeKind::NeedsYou.glyph()),
                        Style::default()
                            .fg(BadgeKind::NeedsYou.tone(theme))
                            .add_modifier(Modifier::BOLD),
                    )]
                } else {
                    Vec::new()
                };
                lines.push(fit_line(
                    with_right(
                        vec![Span::styled(
                            format!(" {header}"),
                            Style::default().fg(theme.feature_title.to_color()),
                        )],
                        right,
                        row_width,
                    ),
                    row_width,
                ));
            }
            lines.push(fit_line(with_right(left, right, row_width), row_width));
            if !feature.collapsed && !compact {
                lines.extend(external_lines(
                    &app.external_sessions_for_feature(&feature.id),
                    "   ",
                    row_width,
                    dim,
                ));
                // FORK: §19.6 — worktrees Claude made for this feature.
                lines.extend(claude_worktree_lines(
                    &app.claude_worktrees_for_feature(&feature.id),
                    "   ",
                    row_width,
                    dim,
                ));
            }
            lines
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
            let mut left = vec![
                group_bar(app.feature_group(*pi, *fi), text),
                Span::styled("  ", text),
                glyph_span(&badge, theme),
                Span::styled(" ", text),
                Span::styled(session.label.clone(), label_style),
            ];
            // FORK: §19.4 — the session has a note.
            if app.session_note(&session.id).is_some() {
                left.push(Span::styled(" ✎", dim));
            }
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
            // FORK: §19.3 — a task prompt is waiting for this agent's pane.
            if !compact && app.pending_prompt_for(&session.id).is_some() {
                if !right.is_empty() {
                    right.push(Span::raw(" "));
                }
                right.push(Span::styled("⏸ prompt", dim));
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

/// FORK: §19.4 — the first cell of a feature or session row: a bar in the
/// group's colour when it is in a group, else a space.
fn group_bar(group: Option<&FeatureGroup>, text: Style) -> Span<'static> {
    match group {
        Some(group) => Span::styled(
            "▏",
            Style::default().fg(super::dialogs::fork_organize::group_color(&group.color)),
        ),
        None => Span::styled(" ", text),
    }
}

/// FORK: §19.4 — a group's header (`▾ ■ Backend`) or, collapsed, the row
/// that stands for it (`▸ ■ Backend · 3 features`), with how many of its
/// features need the user on the right.
#[allow(clippy::too_many_arguments)]
fn group_line(
    app: &App,
    pi: usize,
    group: &FeatureGroup,
    collapsed: bool,
    selected: bool,
    row_width: usize,
    now: DateTime<Utc>,
    dim: Style,
) -> Line<'static> {
    let theme = &app.theme;
    let color = super::dialogs::fork_organize::group_color(&group.color);
    let (count, needs_you) = app.group_rollup(pi, &group.id, now);
    let name_style = if selected {
        Style::default().fg(color).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(color)
    };
    let mut left = vec![
        Span::styled(if collapsed { " ▸ " } else { " ▾ " }, dim),
        Span::styled("■ ", Style::default().fg(color)),
        Span::styled(group.name.clone(), name_style),
    ];
    if collapsed {
        left.push(Span::styled(
            format!(" · {count} feature{}", if count == 1 { "" } else { "s" }),
            dim,
        ));
    }
    let right = if needs_you > 0 {
        vec![Span::styled(
            format!("{}{needs_you}", BadgeKind::NeedsYou.glyph()),
            Style::default()
                .fg(BadgeKind::NeedsYou.tone(theme))
                .add_modifier(Modifier::BOLD),
        )]
    } else {
        Vec::new()
    };
    fit_line(with_right(left, right, row_width), row_width)
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

/// FORK: §19.2 — dimmed, read-only lines for Claude sessions outside AIW:
/// up to `MAX_EXTERNAL_LINES`, then `+N more (E)`.
fn external_lines(
    externals: &[&crate::app::fork_identity::ExternalSession],
    indent: &str,
    row_width: usize,
    dim: Style,
) -> Vec<Line<'static>> {
    use crate::app::fork_identity::MAX_EXTERNAL_LINES;
    let mut lines: Vec<Line<'static>> = externals
        .iter()
        .take(if externals.len() > MAX_EXTERNAL_LINES {
            MAX_EXTERNAL_LINES
        } else {
            externals.len()
        })
        .map(|external| {
            let entry = &external.entry;
            fit_line(
                Line::from(vec![
                    Span::raw(indent.to_string()),
                    Span::styled(
                        format!("↗ {} · {}", entry.display_name(), entry.state_word()),
                        dim.add_modifier(Modifier::ITALIC),
                    ),
                ]),
                row_width,
            )
        })
        .collect();
    if externals.len() > MAX_EXTERNAL_LINES {
        lines.push(fit_line(
            Line::from(vec![
                Span::raw(indent.to_string()),
                Span::styled(
                    format!("↗ +{} more (E)", externals.len() - MAX_EXTERNAL_LINES),
                    dim.add_modifier(Modifier::ITALIC),
                ),
            ]),
            row_width,
        ));
    }
    lines
}

/// FORK: §19.6 — dimmed `⎇ <name> · <origin>` lines for the worktrees Claude
/// made through AIW's hook: up to `MAX_LINES`, then `+N more (W)`.
fn claude_worktree_lines(
    worktrees: &[&crate::fork::claude_worktrees::ClaudeWorktree],
    indent: &str,
    row_width: usize,
    dim: Style,
) -> Vec<Line<'static>> {
    use crate::app::fork_claude_worktrees::MAX_LINES;
    let line = |text: String| {
        fit_line(
            Line::from(vec![
                Span::raw(indent.to_string()),
                Span::styled(text, dim.add_modifier(Modifier::ITALIC)),
            ]),
            row_width,
        )
    };
    let shown = if worktrees.len() > MAX_LINES {
        MAX_LINES
    } else {
        worktrees.len()
    };
    let mut lines: Vec<Line<'static>> = worktrees
        .iter()
        .take(shown)
        .map(|wt| line(wt.row_text()))
        .collect();
    if worktrees.len() > MAX_LINES {
        lines.push(line(format!("⎇ +{} more (W)", worktrees.len() - MAX_LINES)));
    }
    lines
}

/// The footer's key hints: a narrow pane, a short line.
pub(crate) fn footer_hints(app: &App) -> Line<'static> {
    let theme = &app.theme;
    let key = super::fork_chrome::key_style(theme);
    let hint = |text: &'static str| Span::raw(text);
    let mut spans = if app.pending_permission_for_selection().is_some() {
        super::fork_permissions::row_hint_spans(theme)
    } else {
        Vec::new()
    };
    // 34 columns at the default width: the wording bends to the agent hint.
    if app.desired_live_target_exists() {
        spans.extend([
            Span::styled(" Enter", key),
            hint(" agent  "),
            Span::styled("i", key),
            hint(" needs  "),
            Span::styled("v", key),
            hint(" tiles  "),
            Span::styled("?", key),
            hint(" "),
            Span::styled("q", key),
        ]);
    } else {
        spans.extend([
            Span::styled(" i", key),
            hint(" needs you  "),
            Span::styled("v", key),
            hint(" tiles  "),
            Span::styled("?", key),
            hint(" help  "),
            Span::styled("q", key),
        ]);
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::attention::{AttentionSource, AttentionState};
    use crate::fork::cockpit::Link;
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

    /// The dashboard as the cockpit sidebar draws it: `width` columns of a
    /// `height`-row pane.
    fn sidebar(width: u16, height: u16) -> App {
        let mut app = realistic_app(MockTmuxOps::new());
        app.cockpit_attach(
            Link::recording(),
            "/bin/aiw".into(),
            "/s/cockpit.sock".into(),
        );
        app.viewport_cols = width;
        app.viewport_total_rows = height;
        app.viewport_rows = height.saturating_sub(3);
        app
    }

    #[test]
    fn the_sidebar_fits_every_width_and_the_info_block_needs_24_rows() {
        // The block needs 24 rows of *sidebar* (the terminal minus the
        // header and footer rows).
        for (width, height, expect_info) in [
            (28u16, 20u16, false),
            (28, 30, true),
            (36, 29, false),
            (36, 59, true),
            (50, 40, true),
        ] {
            let mut app = sidebar(width, height);
            app.selection = Selection::Session(0, 0, 0);
            app.workspace_commit_now();
            let screen = rows(&mut app, width, height);
            let text = screen.join("\n");
            assert!(text.contains("╭ Sessions"), "{width}x{height}:\n{text}");
            for (y, row) in screen.iter().enumerate() {
                assert_eq!(
                    UnicodeWidthStr::width(row.as_str()),
                    usize::from(width),
                    "{width}x{height} row {y} overflows: {row}"
                );
            }
            assert_eq!(
                text.contains("╭ claude ─"),
                expect_info,
                "{width}x{height}:\n{text}"
            );
            if width >= 36 {
                assert!(
                    screen[height as usize - 1].contains("v tiles"),
                    "{}",
                    screen[height as usize - 1]
                );
            }
            assert!(
                !text.contains("Tab"),
                "nothing is typed through the sidebar"
            );
        }
    }

    #[test]
    fn rows_show_badges_and_the_context_percentage_in_the_thresholds_tone() {
        let mut app = sidebar(36, 59);
        // billing-12 has the question; select its claude session.
        app.selection = Selection::Session(1, 0, 0);
        app.workspace_commit_now();
        let screen = rows(&mut app, 36, 59);
        let text = screen.join("\n");
        assert!(text.contains("⛔ claude"), "{text}");
        assert!(text.contains("question"), "{text}");
        assert!(text.contains("✓  claude"), "harbor-13 finished: {text}");
        assert!(text.contains("●  claude"), "harbor-12 working: {text}");
        assert!(text.contains("■  claude"), "stopped feature rows: {text}");
        assert!(
            screen
                .iter()
                .any(|row| row.contains("billing-app") && row.contains("⛔1")),
            "{text}"
        );
        // Context from the status line, in the critical tone past the threshold.
        app.selection = Selection::Session(0, 0, 0);
        let id = app.store.projects[0].features[0].sessions[0].id.clone();
        assert!(app.handle_status_line_message(&serde_json::json!({
            "type": "status_line", "aiw_feature_session_id": id, "context_used_pct": 88
        })));
        app.workspace_commit_now();
        let screen = rows(&mut app, 36, 59);
        let row = screen
            .iter()
            .find(|row| row.contains("▎") && row.contains("claude"))
            .unwrap()
            .clone();
        assert!(row.contains("88%"), "{row}");
        let mut terminal = Terminal::new(TestBackend::new(36, 59)).unwrap();
        terminal
            .draw(|frame| crate::ui::draw(frame, &mut app))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let y = screen.iter().position(|r| r == &row).unwrap() as u16;
        let x = row[..row.find("88%").unwrap()].chars().count() as u16;
        assert_eq!(
            buffer[(x, y)].style().fg,
            Some(app.theme.danger.to_color()),
            "88% is past the critical threshold"
        );
        // The info block says the same.
        let text = screen.join("\n");
        assert!(text.contains("ctx 88%"), "{text}");
    }

    #[test]
    fn compact_sidebar_shows_glyph_and_name_only() {
        let mut app = sidebar(27, 24);
        app.selection = Selection::Session(1, 0, 0);
        app.workspace_commit_now();
        let screen = rows(&mut app, 27, 24);
        let text = screen.join("\n");
        assert!(text.contains("⛔ claude"), "{text}");
        assert!(
            !text.contains("permission"),
            "no second line in the compact sidebar: {text}"
        );
        assert!(screen.iter().any(|row| row.starts_with("│▎")), "{text}");
    }

    #[test]
    fn an_unread_finished_turn_dims_once_the_agent_pane_was_focused() {
        let mut app = sidebar(36, 59);
        app.selection = Selection::Session(0, 1, 0); // harbor-13: finished
        app.workspace_commit_now();
        let bold_check = |app: &mut App| {
            let mut terminal = Terminal::new(TestBackend::new(36, 59)).unwrap();
            terminal.draw(|frame| crate::ui::draw(frame, app)).unwrap();
            let buffer = terminal.backend().buffer();
            let screen = rows(app, 36, 59);
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
        app.cockpit_focus_agent();
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
    fn pills_and_sidebar_counts_agree() {
        let mut app = sidebar(36, 59);
        app.workspace_commit_now();
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
    }

    #[test]
    fn claude_worktrees_show_under_their_feature_capped_with_a_more_line() {
        use crate::fork::claude_worktrees::ClaudeWorktree;
        let mut app = sidebar(40, 59);
        let feature_id = app.store.projects[0].features[0].id.clone();
        let project = app.store.projects[0].name.clone();
        app.store.projects[0].collapsed = false;
        app.store.projects[0].features[0].collapsed = false;
        let wt = |n: usize| ClaudeWorktree {
            id: format!("cwt-{n}"),
            repo: "/r".into(),
            path: format!("/w/claude-wt{n}").into(),
            branch: format!("worktree-wt{n}"),
            base: "abc".into(),
            name: format!("wt{n}"),
            origin: if n == 0 {
                "worktree".into()
            } else {
                "subagent".into()
            },
            parent_session_id: None,
            claude_session_id: None,
            requested_cwd: "/r".into(),
            port_start: None,
            port_count: None,
            created_at: String::new(),
        };
        for n in 0..5 {
            app.claude_worktrees.items.push(wt(n));
            app.claude_worktrees
                .feature_of
                .insert(format!("cwt-{n}"), feature_id.clone());
            app.claude_worktrees
                .project_of
                .insert(format!("cwt-{n}"), project.clone());
        }
        app.selection = Selection::Feature(0, 0);
        app.workspace_commit_now();
        let rows = rows(&mut app, 40, 59);
        let text = rows.join("\n");
        assert!(text.contains("⎇ wt0 · worktree"), "{text}");
        assert!(text.contains("⎇ wt2 · subagent"), "{text}");
        assert!(!text.contains("⎇ wt3"), "only three are listed: {text}");
        assert!(text.contains("⎇ +2 more (W)"), "{text}");
        // The lines sit between the feature row and its first session.
        let feature_name = app.store.projects[0].features[0].name.clone();
        let feature_row = rows.iter().position(|r| r.contains(&feature_name)).unwrap();
        let wt_row = rows.iter().position(|r| r.contains("⎇ wt0")).unwrap();
        assert_eq!(wt_row, feature_row + 1, "{text}");
    }

    #[test]
    fn sidebar_frame_makes_no_tmux_calls() {
        let mut app = sidebar(36, 59);
        app.selection = Selection::Session(0, 0, 0);
        app.workspace_commit_now();
        let mut terminal = Terminal::new(TestBackend::new(36, 59)).unwrap();
        for _ in 0..3 {
            terminal
                .draw(|frame| crate::ui::draw(frame, &mut app))
                .unwrap();
        }
        app.cockpit_focus_agent();
        terminal
            .draw(|frame| crate::ui::draw(frame, &mut app))
            .unwrap();
    }
}
