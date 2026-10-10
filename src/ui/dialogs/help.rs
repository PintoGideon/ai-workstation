use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, BorderType, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
        Wrap,
    },
};

use super::super::dashboard::centered_rect;
use crate::theme::Theme;

use crate::ui::fork_chrome::{dim_style, key_style};

/// FORK: G11 one key binding: key right-aligned in the accent colour, the
/// description in the text colour; a continuation line (no key) reads dim.
fn bind_line<'a>(key: &'a str, desc: &'a str, theme: &Theme) -> Line<'a> {
    let desc_style = if key.is_empty() {
        dim_style(theme)
    } else {
        Style::default().fg(theme.text.to_color())
    };
    Line::from(vec![
        Span::styled(format!("  {:>16}", key), key_style(theme)),
        Span::raw("  "),
        Span::styled(desc, desc_style),
    ])
}

/// FORK: G11 a section title in bold text colour.
fn section_title<'a>(title: &'a str, theme: &Theme) -> Line<'a> {
    Line::from(Span::styled(
        title,
        Style::default()
            .fg(theme.text.to_color())
            .add_modifier(Modifier::BOLD),
    ))
}

/// FORK: G11 a blank line, then the section title.
fn section_header<'a>(title: &'a str, theme: &Theme) -> [Line<'a>; 2] {
    [Line::from(""), section_title(title, theme)]
}

pub fn draw_help(frame: &mut Frame, scroll_offset: usize, theme: &Theme) -> usize {
    let area = centered_rect(90, 85, frame.area());
    draw_help_at(frame, area, scroll_offset, theme)
}

fn draw_help_at(frame: &mut Frame, area: Rect, scroll_offset: usize, theme: &Theme) -> usize {
    crate::ui::draw_modal_overlay(frame, area, theme);

    let normal_keybinds: Vec<(&str, &str)> = vec![
        ("j/k / \u{2191}/\u{2193}", "Navigate up/down"),
        ("h / \u{2190}", "Collapse project/feature"),
        ("l / \u{2192}", "Expand project/feature"),
        ("Enter", "Toggle expand / view or recover session"),
        ("s", "Add session (picker)"),
        ("m", "Model / reasoning advice for selected agent session"),
        ("S", "Pick session to resume"),
        ("N", "Create new project"),
        ("n", "Create new feature"),
        ("g", "Browse GitHub issues for the selected project"),
        ("B", "Create batch features"),
        ("O", "Open AIW settings project"),
        ("A", "Manage agent harnesses"),
        ("d", "Delete project/feature/session"),
        ("D", "View debug log"),
        ("p", "Open syntax parser picker"),
        ("L", "Open prompt library"),
        ("E", "Edit headless AI prompt templates (overrides)"),
        ("G", "Open PR Triage (experimental)"),
        ("", "(on a project row: Review a PR instead)"),
        ("W", "Open AI Review for this feature (experimental)"),
        (
            "K",
            "Open Learning Mode (read this codebase, ask questions)",
        ),
        ("P", "Run a plan interview for this feature"),
        ("Q", "Run a Quick Plan interview for this feature"),
        ("T", "Theme picker"),
        (
            "t",
            "Companion review feature: land its commits on the source branch",
        ),
        ("c", "Start feature / stopped session"),
        // FORK: split view (DESIGN.md §4.1).
        (
            "v",
            "Split view: tile this feature's agents (project row: one per feature)",
        ),
        (
            "",
            "  in split view: Ctrl+Space ←↑→↓ focus · z zoom · v leave · [ ] page · i needs you",
        ),
        ("x", "Stop feature / session (session stays listed)"),
        ("r", "Rename session/feature"),
        ("R", "Refresh statuses + PR badges (runs gh)"),
        ("V", "Check pending diff review"),
        ("u", "Preferred harness / worktree config"),
        ("F", "Fork feature (new branch)"),
        ("y", "Toggle mark feature as ready"),
        ("Z", "Generate session summary"),
        ("z", "Dormant features (idle + unattended)"),
        ("w", "Context window / warning settings"),
        ("i", "Needs attention: questions and finished work"),
        ("I", "On a TODOs row: start the next TODO"),
        ("/", "Search and jump to item"),
        ("Ctrl+Space c", "Config wizard"),
        ("Ctrl+Space m", "Compact review memory"),
        ("?", "Toggle this help"),
        ("q / Esc", "Quit"),
    ];

    // FORK: G11 — section titles instead of a banner line; the close/scroll
    // keys moved into the bottom border.
    let mut lines: Vec<Line> = Vec::with_capacity(256);
    lines.push(section_title("  On the dashboard:", theme));

    lines.extend(
        normal_keybinds
            .iter()
            .map(|(key, desc)| bind_line(key, desc, theme)),
    ); // FORK: G11

    lines.extend(section_header("  During a plan interview:", theme)); // FORK: G11

    let plan_interview_keybinds: Vec<(&str, &str)> = vec![
        ("r / d", "Resume or discard a saved draft (on entry)"),
        (
            "Enter",
            "Save answer and continue; at the AI prompt, review raw plan (no tokens)",
        ),
        ("Shift+Enter", "Insert a newline (free-text answers)"),
        ("j/k / \u{2191}/\u{2193}", "Choose a select-option answer"),
        (
            "Backspace",
            "Clear the picked option (back to nothing picked; lets you answer with custom text alone)",
        ),
        (
            "e",
            "Type your own answer to a select question (submitted with any picked option; Enter still submits)",
        ),
        ("Ctrl+B", "Return to the previous question"),
        ("Ctrl+S", "Skip an optional question"),
        ("Ctrl+R", "Restore the previous interview's answer (re-run)"),
        (
            "a",
            "Ask AI follow-ups at the optional AI prompt (uses tokens)",
        ),
        ("Ctrl+F", "Draft plan now from saved answers (uses tokens)"),
        (
            "Esc",
            "Cancel (launch without plan, or leave plan unchanged)",
        ),
        (
            "y / n",
            "Open a still-running session with the kickoff prompt seeded but unsent, or leave it alone",
        ),
    ];

    lines.extend(
        plan_interview_keybinds
            .iter()
            .map(|(key, desc)| bind_line(key, desc, theme)),
    ); // FORK: G11

    lines.extend(section_header("  During plan review:", theme)); // FORK: G11

    let plan_review_keybinds: Vec<(&str, &str)> = vec![
        ("j/k / PgUp/PgDn", "Scroll the rendered plan"),
        ("e", "Edit raw plan markdown"),
        (
            "m",
            "Model / reasoning advice for implementing the reviewed plan",
        ),
        (
            "a",
            "Expert review with an explicit frontier model, or re-open one already held (uses tokens for a new review)",
        ),
        (
            "f",
            "Give free-form feedback; the agent may inspect the repository read-only (uses tokens)",
        ),
        (
            "i",
            "Research focused questions in isolated read-only contexts, then merge findings (uses tokens)",
        ),
        ("r", "Regenerate the plan (uses tokens)"),
        (
            "Enter",
            "Accept plan (on creation: launch feature and seed the kickoff prompt)",
        ),
        ("Ctrl+S", "Save edit and return to preview"),
        ("Ctrl+S (feedback)", "Send feedback and revise the plan"),
        (
            "Ctrl+S (research)",
            "Run isolated investigators and merge their findings",
        ),
        ("Esc", "Discard edit or confirm abort"),
    ];

    lines.extend(
        plan_review_keybinds
            .iter()
            .map(|(key, desc)| bind_line(key, desc, theme)),
    ); // FORK: G11

    lines.extend(section_header(
        "  During an Expert review of the plan:",
        theme,
    )); // FORK: G11

    let plan_critique_keybinds: Vec<(&str, &str)> = vec![
        ("j/k / PgUp/PgDn", "Scroll the review"),
        ("r", "Revise the plan with this feedback (uses tokens)"),
        ("Esc / Enter", "Back to the plan, unchanged"),
    ];

    lines.extend(
        plan_critique_keybinds
            .iter()
            .map(|(key, desc)| bind_line(key, desc, theme)),
    ); // FORK: G11

    lines.extend(section_header(
        "  While viewing: Ctrl+Q exits; Ctrl+Space opens leader commands.",
        theme,
    )); // FORK: G11

    let view_keybinds: Vec<(&str, &str)> = vec![
        ("Ctrl+Q", "Exit view"),
        ("Ctrl+Space", "Open leader command menu"),
        // FORK: typing goes straight to the agent unless the compose box is on.
        (
            "any text key",
            "Typed straight into the agent (compose box: Ctrl+Space e)",
        ),
        ("", "Commands below require Ctrl+Space first:"),
        ("s", "Steering coach (experimental)"),
        ("e", "Toggle compose/direct input (agent sessions)"),
        ("d", "Diff viewer (all changes / commit)"),
        ("m", "Markdown file picker/viewer"),
        ("B", "Model / reasoning advice for this agent session"),
        ("n", "Open current plan"),
        (
            "F",
            "Start a fresh-context session (asks for your prompt first)",
        ),
        ("b", "Show/hide sidebar"),
        ("v", "Expand/collapse todos"),
        // FORK: in a tile zoomed from split view, `z` returns to the tiles.
        (
            "z",
            "Complete this session's referenced TODO (zoomed split tile: back to split view)",
        ),
        ("N", "Quick-capture a TODO for this worktree"),
        ("t / T", "Cycle next/prev session"),
        ("w", "Session switcher"),
        ("h", "Bookmark picker popup"),
        ("H / M", "Bookmark / unbookmark session"),
        ("1-9", "Jump to bookmark slot"),
        ("/", "Command picker (slash + AIW actions)"),
        ("a", "AIW local actions picker"),
        ("p", "Prompt library (inject saved prompt)"),
        ("E", "Edit headless AI prompt templates (overrides)"),
        ("R", "Refresh pane sizing"),
        ("D", "Debug log"),
        ("A", "Manage agent harnesses"),
        ("P", "Back to PR Triage (if one is stashed)"),
        ("G", "Open PR Triage for this feature"),
        ("W", "Open AI Review for this feature"),
    ];

    lines.extend(
        view_keybinds
            .iter()
            .map(|(key, desc)| bind_line(key, desc, theme)),
    ); // FORK: G11

    lines.extend(section_header("  In the TODOs view:", theme)); // FORK: G11

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
        ("Enter", "Start work: agent now, or plan it first"),
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
            .map(|(key, desc)| bind_line(key, desc, theme)),
    ); // FORK: G11

    lines.extend(section_header("  In PR Triage:", theme)); // FORK: G11

    let pr_review_keybinds: Vec<(&str, &str)> = vec![
        ("j/k", "Navigate comments"),
        ("Ctrl+D/U", "Scroll detail down/up"),
        ("h", "Hide/show resolved comments"),
        (
            "o",
            "Cycle sort (fetch/file/author/humans-first/conversations-last)",
        ),
        ("f", "Inject scoped fix into agent session"),
        ("", "(e edit · Tab inject · Ctrl+T vim)"),
        ("", "(a finished investigation's findings are appended)"),
        ("", "(first fix/batch picks the fix target: existing"),
        ("", "live session, a named dedicated session + harness, or"),
        ("", "New feature… — an isolated worktree with its own"),
        ("", "harness + vibe mode, set up in one compact form)"),
        ("v", "Investigate this comment — read-only headless pass"),
        ("", "(pick a harness; the answer persists and shows in"),
        ("", "the detail panel)"),
        ("e", "Attach optional context for the next Investigate —"),
        ("", "a hypothesis the run verifies, not a fact to assume"),
        ("a", "Act on a finished investigation: post reply,"),
        ("", "ask follow-up, dismiss, keep as TODO"),
        ("Space", "Mark comment for batch fix"),
        ("B", "Inject one combined prompt for all marked"),
        ("P", "Jump to the linked fix session"),
        ("", "(Ctrl+Space P there jumps back)"),
        ("R", "Reply — pick Done / not-needed, then post"),
        ("M", "Add finding to review-memory doc"),
        ("", "(e edit · Tab category · g project/global)"),
        (
            "m",
            "Mark — Done (local) / Skip (local) / Resolve on GitHub",
        ),
        ("i", "Download + install syntax parser for file (git clone)"),
        ("r", "Refresh comments from GitHub"),
        ("g", "Pick a different PR to triage"),
        ("A", "Open the AI Review pane for this PR"),
        ("I", "Land the triage feature's commits on the PR"),
        ("", "(push to the PR branch, or cherry-pick into the"),
        ("", "source worktree — never while it's dirty)"),
        ("q / Esc", "Close PR Triage"),
    ];

    lines.extend(
        pr_review_keybinds
            .iter()
            .map(|(key, desc)| bind_line(key, desc, theme)),
    ); // FORK: G11

    lines.extend(section_header("  In AI Review:", theme)); // FORK: G11

    let ai_review_keybinds: Vec<(&str, &str)> = vec![
        ("j/k", "Navigate findings"),
        ("Ctrl+D/U", "Scroll detail down/up"),
        ("s", "Skip/unskip finding (excludes it from W)"),
        ("e", "Edit finding body"),
        ("A", "Generate/regenerate the AI review"),
        ("", "(first run picks harness + model)"),
        ("W", "Post kept findings to GitHub as a review"),
        ("", "(e edit summary)"),
        (
            "q / Esc",
            "Close AI Review (back to PR Triage if opened from there)",
        ),
    ];

    lines.extend(
        ai_review_keybinds
            .iter()
            .map(|(key, desc)| bind_line(key, desc, theme)),
    ); // FORK: G11

    lines.extend(section_header("  In the PR picker:", theme)); // FORK: G11

    let pr_picker_keybinds: Vec<(&str, &str)> = vec![
        ("j/k", "Navigate PRs"),
        ("Enter", "Open highlighted PR in PR Triage"),
        ("W", "Open highlighted PR in AI Review"),
        ("a", "Include/hide closed & merged PRs"),
        ("#", "Enter a PR number instead"),
        ("b", "Bootstrap review memory from recent PRs"),
        ("", "(g picks the project or global doc)"),
        ("c", "Compact review memory (merge dupes, prune stale)"),
        ("", "(g picks the project or global doc)"),
        ("Tab", "Switch to the Review tab (review a PR yourself)"),
        ("q / Esc", "Close picker"),
    ];

    lines.extend(
        pr_picker_keybinds
            .iter()
            .map(|(key, desc)| bind_line(key, desc, theme)),
    ); // FORK: G11

    lines.extend(section_header("  In the PR picker's Review tab:", theme)); // FORK: G11

    let review_tab_keybinds: Vec<(&str, &str)> = vec![
        ("j/k", "Navigate open PRs (yours and drafts included)"),
        ("Enter", "Review the highlighted PR in the diff viewer"),
        ("", "(no agent, no feature; your checkout is untouched)"),
        ("r", "Reload the list"),
        ("Tab", "Back to the Triage tab"),
        ("q / Esc", "Close (or stop a PR that is still opening)"),
        ("", "In the review: q submits to GitHub, Esc pauses"),
        ("", "(draft saved; ? lists every key there)"),
    ];

    lines.extend(
        review_tab_keybinds
            .iter()
            .map(|(key, desc)| bind_line(key, desc, theme)),
    ); // FORK: G11

    let paragraph = Paragraph::new(lines).wrap(Wrap { trim: false });
    let total_lines = paragraph.line_count(area.width.saturating_sub(2));
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
