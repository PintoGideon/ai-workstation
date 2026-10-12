use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
    },
};
use unicode_width::UnicodeWidthStr;

use super::super::dashboard::centered_rect;
use crate::theme::Theme;

use crate::ui::fork_chrome::{dim_style, key_style};

/// FORK: G11 — the two columns of the help body: a right-aligned key column
/// and the description beside it, sized for the panel's inner width.
#[derive(Clone, Copy)]
struct HelpColumns {
    key: usize,
    desc: usize,
}

impl HelpColumns {
    /// Two columns of margin, the key column, two columns of gap, the text.
    const MARGIN: usize = 2;
    const GAP: usize = 2;
    const KEY: usize = 16;

    fn for_width(width: usize) -> Self {
        // A narrow panel gives the key column up rather than wrapping every
        // description into one word per line.
        let key = if width >= 60 {
            Self::KEY
        } else {
            (width / 3).clamp(4, Self::KEY)
        };
        let desc = width.saturating_sub(Self::MARGIN + key + Self::GAP).max(8);
        Self { key, desc }
    }

    fn indent(&self) -> usize {
        Self::MARGIN + self.key + Self::GAP
    }
}

/// FORK: G11 one key binding: key right-aligned in the accent colour, the
/// description in the text colour, wrapped with a hanging indent so a
/// continuation never starts at the border; a line with no key (a note)
/// reads dim.
fn bind_lines<'a>(
    key: &'a str,
    desc: &'a str,
    columns: HelpColumns,
    theme: &Theme,
) -> Vec<Line<'a>> {
    let desc_style = if key.is_empty() {
        dim_style(theme)
    } else {
        Style::default().fg(theme.text.to_color())
    };
    let key_shown = if key.width() > columns.key {
        crate::ui::fork_text::fit_line(Line::from(key), columns.key).to_string()
    } else {
        key.to_string()
    };
    let mut wrapped = crate::ui::toast::wrap_message(desc, columns.desc, usize::MAX).into_iter();
    let first = wrapped.next().unwrap_or_default();
    let mut lines = vec![Line::from(vec![
        Span::styled(
            format!(
                "{:>width$}",
                key_shown,
                width = HelpColumns::MARGIN + columns.key
            ),
            key_style(theme),
        ),
        Span::raw(" ".repeat(HelpColumns::GAP)),
        Span::styled(first, desc_style),
    ])];
    lines.extend(wrapped.map(|rest| {
        Line::from(vec![
            Span::raw(" ".repeat(columns.indent())),
            Span::styled(rest, dim_style(theme)),
        ])
    }));
    lines
}

/// FORK: G11 a section title in bold text colour, wrapped to the panel.
fn section_title<'a>(title: &'a str, columns: HelpColumns, theme: &Theme) -> Vec<Line<'a>> {
    let width = HelpColumns::MARGIN + columns.key + HelpColumns::GAP + columns.desc;
    crate::ui::toast::wrap_message(
        title.trim_start(),
        width.saturating_sub(HelpColumns::MARGIN),
        usize::MAX,
    )
    .into_iter()
    .map(|text| {
        Line::from(Span::styled(
            format!("{}{text}", " ".repeat(HelpColumns::MARGIN)),
            Style::default()
                .fg(theme.text.to_color())
                .add_modifier(Modifier::BOLD),
        ))
    })
    .collect()
}

/// FORK: G11 a blank line, then the section title.
fn section_header<'a>(title: &'a str, columns: HelpColumns, theme: &Theme) -> Vec<Line<'a>> {
    let mut lines = vec![Line::from("")];
    lines.extend(section_title(title, columns, theme));
    lines
}

pub fn draw_help(frame: &mut Frame, scroll_offset: usize, theme: &Theme) -> usize {
    let area = centered_rect(90, 85, frame.area());
    draw_help_at(frame, area, scroll_offset, theme)
}

fn draw_help_at(frame: &mut Frame, area: Rect, scroll_offset: usize, theme: &Theme) -> usize {
    crate::ui::draw_modal_overlay(frame, area, theme);
    // FORK: G11 two-column layout sized once for this panel.
    let columns = HelpColumns::for_width(usize::from(area.width.saturating_sub(2)));

    let normal_keybinds: Vec<(&str, &str)> = vec![
        ("j/k / \u{2191}/\u{2193}", "Navigate up/down"),
        ("h / \u{2190}", "Collapse project/feature"),
        ("l / \u{2192}", "Expand project/feature"),
        (
            "Enter",
            "Keyboard into the agent pane (running session) / expand / recover",
        ),
        (
            "Ctrl+Q",
            "Keyboard sidebar ⇄ agent pane (tmux; works from either side)",
        ),
        (
            "s",
            "Add session (picker; \"Claude (background)\" runs under claude --bg)",
        ),
        ("S", "Pick session to resume"),
        ("N", "Create new project"),
        // FORK: §19.3
        (
            "n",
            "Create new feature (Space ticks more agents: one prompt, several agents)",
        ),
        ("g", "Browse GitHub issues for the selected project"),
        ("B", "Create batch features"),
        ("O", "Open AIW settings project"),
        ("A", "Manage agent harnesses"),
        ("d", "Delete project/feature/session"),
        ("D", "View debug log"),
        ("p", "Open syntax parser picker"),
        ("L", "Open prompt library"),
        ("T", "Theme picker"),
        ("c", "Start feature / stopped session"),
        // FORK: tiles in the agent pane (DESIGN.md §18).
        (
            "v",
            "Tile this feature's agents (a fan-out group: every sibling) in the agent pane; Alt+q there closes",
        ),
        ("x", "Stop feature / session (session stays listed)"),
        ("r", "Rename session/feature"),
        ("R", "Refresh statuses + PR badges (runs gh)"),
        ("V", "Check pending diff review"),
        ("u", "Preferred harness / worktree config"),
        ("F", "Fork feature (new branch)"),
        // FORK: §19.2
        (
            "f",
            "Fork the session's conversation (same feature or new worktree)",
        ),
        ("E", "Claude sessions outside AIW in your projects"),
        (
            "W",
            "Worktrees Claude made here (--worktree, subagents): terminal, remove",
        ),
        // FORK: §19.4
        (
            "G",
            "Feature groups: move the feature into one, new / rename / colour / delete",
        ),
        (
            "C",
            "Fold / unfold the selected feature's group (h on a folded feature too)",
        ),
        ("#", "Note on the selected session or feature (found by /)"),
        (
            "@",
            "Message an agent session (Claude: its inbox; others: next turn end)",
        ), // FORK: §19.7
        (
            "!",
            "Recover an agent: restart resuming, interrupt, /compact, replay, deliver queued",
        ), // FORK: §19.8
        (
            "Enter / c / x / d",
            "On a background Claude row: attach / restart (respawn) / stop / claude rm",
        ),
        ("y", "Toggle mark feature as ready"),
        // FORK: hook-driven productivity (DESIGN.md §16).
        (
            "a / X",
            "Allow / deny the permission the selected agent is asking for",
        ),
        (
            "e",
            "Queue a follow-up for the selected agent (delivered when its turn ends)",
        ),
        ("z", "Dormant features (idle + unattended)"),
        ("w", "Context window / warning settings"),
        ("i", "Needs attention: questions and finished work"),
        ("I", "On a TODOs row: start the next TODO"),
        ("/", "Search and jump to item (names, notes, group names)"),
        ("?", "Toggle this help"),
        ("q", "Quit the dashboard (the agents keep running)"),
    ];

    // FORK: G11 — section titles instead of a banner line; the close/scroll
    // keys moved into the bottom border.
    let mut lines: Vec<Line> = Vec::with_capacity(256);
    lines.extend(section_title("  On the dashboard:", columns, theme));

    lines.extend(
        normal_keybinds
            .iter()
            .flat_map(|(key, desc)| bind_lines(key, desc, columns, theme)),
    ); // FORK: G11

    // FORK: the cockpit (DESIGN.md §18).
    lines.extend(section_header("  The cockpit:", columns, theme));
    let cockpit_keybinds: Vec<(&str, &str)> = vec![
        ("j/k", "Pick a session; the agent pane follows after 150 ms"),
        (
            "Enter",
            "Keyboard into the agent's real pane: typing is native, as in tmux attach",
        ),
        (
            "Ctrl+Q",
            "Keyboard back to the sidebar (and from the sidebar: to the agent)",
        ),
        (
            "v",
            "aiw grid in the agent pane; Alt+q there brings the agent back",
        ),
        (
            "dialogs",
            "Zoom the sidebar over the agent pane and un-zoom when they close",
        ),
        (
            "prompts",
            "Kickoffs, TODOs, templates, past prompts are pasted unsent; you press Enter",
        ),
        (
            "info block",
            "Title · harness · model · ctx % · cost · age · branch · worktree · queued",
        ),
    ];
    lines.extend(
        cockpit_keybinds
            .iter()
            .flat_map(|(key, desc)| bind_lines(key, desc, columns, theme)),
    );

    lines.extend(section_header(
        "  Ctrl+Space, then a key — commands on the selected session:",
        columns,
        theme,
    ));
    let leader_keybinds: Vec<(&str, &str)> = vec![
        ("d", "Diff viewer (all changes / commit)"),
        ("m", "Markdown file picker/viewer"),
        ("l", "Past and unsent prompts: paste one again (unsent)"),
        ("N", "Quick-capture a TODO for this worktree"),
        ("z", "Complete the TODO this session came from"),
        ("h / H / M / 1-9", "Bookmarks: picker / add / remove / jump"),
        ("/", "Command picker (slash + AIW actions)"),
        ("a", "AIW local actions picker"),
        ("c", "Config wizard"),
        ("r", "Refresh statuses + PR badges (runs gh)"),
    ];

    lines.extend(
        leader_keybinds
            .iter()
            .flat_map(|(key, desc)| bind_lines(key, desc, columns, theme)),
    ); // FORK: G11

    lines.extend(section_header("  In the TODOs view:", columns, theme)); // FORK: G11

    let todos_keybinds: Vec<(&str, &str)> = vec![
        ("j/k / \u{2191}/\u{2193}", "Navigate TODOs"),
        ("a / n", "Add a TODO"),
        ("e", "Edit title"),
        ("o", "Edit notes"),
        ("b", "Edit scratchpad banner"),
        ("Space / x", "Cycle state: todo → in progress → done"),
        ("P", "Cycle priority (High/Med/Low)"),
        ("J / K", "Reorder up/down"),
        ("d", "Delete TODO (y/n confirm)"),
        ("I", "Start the next TODO in the visible lists"),
        ("Enter", "Start work on this TODO"),
        ("", "  (jumps straight to a linked session or feature)"),
        ("", "  (an in-progress TODO cannot launch another agent)"),
        ("p", "Show/hide the project TODO list"),
        ("g", "Show/hide the global TODO list"),
        ("", "  (visibility is shared until AIW exits)"),
        ("Tab / Shift+Tab", "Move between visible TODO lists"),
        ("M / C", "Move / copy the TODO to another list"),
        ("q / Esc / Ctrl+Q", "Exit TODOs view"),
    ];

    lines.extend(
        todos_keybinds
            .iter()
            .flat_map(|(key, desc)| bind_lines(key, desc, columns, theme)),
    ); // FORK: G11

    // Already wrapped with hanging indents, so no `Wrap` (which would restart
    // a continuation at the border).
    let total_lines = lines.len();
    let paragraph = Paragraph::new(lines);
    let visible_height = area.height.saturating_sub(2) as usize;
    let max_scroll = total_lines.saturating_sub(visible_height);
    let scroll = scroll_offset.min(max_scroll) as u16;

    // FORK: G11 rounded panel, bold title, key hints in the bottom border.
    let help = paragraph.scroll((scroll, 0)).block(
        Block::default()
            .title(Span::styled(
                " Keybindings ",
                Style::default()
                    .fg(theme.primary.to_color())
                    .add_modifier(Modifier::BOLD),
            ))
            .title_bottom(
                Line::from(vec![
                    Span::styled(" Esc", key_style(theme)),
                    Span::styled(" close  ", dim_style(theme)),
                    Span::styled("j/k", key_style(theme)),
                    Span::styled(" scroll ", dim_style(theme)),
                ])
                .right_aligned(),
            )
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .style(Style::default().bg(theme.effective_bg()))
            .border_style(Style::default().fg(theme.primary.to_color())),
    );

    frame.render_widget(help, area);

    if total_lines > visible_height && area.width > 0 && area.height > 2 {
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
        let mut scrollbar_state =
            ScrollbarState::new(max_scroll).position(scroll_offset.min(max_scroll));
        let scrollbar_area = Rect {
            x: area.x + area.width - 1,
            y: area.y + 1,
            width: 1,
            height: area.height.saturating_sub(2),
        };
        frame.render_stateful_widget(scrollbar, scrollbar_area, &mut scrollbar_state);
    }
    scroll as usize
}
