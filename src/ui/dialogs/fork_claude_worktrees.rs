//! FORK: the `W` list of worktrees Claude created through AIW's hook
//! (DESIGN.md §19.6).

use ratatui::{
    Frame,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::app::App;
use crate::app::fork_claude_worktrees::ClaudeWorktreesState;
use crate::fork::claude_worktrees::ClaudeWorktree;
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

/// `claude-auth  ·  worktree-auth  ·  subagent Explore  ·  AIW_PORT 4110`.
pub fn worktree_line(wt: &ClaudeWorktree) -> String {
    let ports = wt
        .port_start
        .map(|p| format!("  ·  AIW_PORT {p}"))
        .unwrap_or_default();
    format!("{}  ·  {}  ·  {}{ports}", wt.name, wt.branch, wt.origin)
}

pub fn draw_claude_worktrees(frame: &mut Frame, app: &App, state: &ClaudeWorktreesState) {
    let theme = &app.theme;
    let area = frame.area();
    if area.width < 30 || area.height < 8 {
        return;
    }
    let worktrees = app.claude_worktrees_in_dialog();
    let width = (area.width * 8 / 10).clamp(40, 110).min(area.width);
    let height = ((worktrees.len() as u16) * 2 + 7).clamp(8, area.height);
    let rect = crate::ui::dialog_rect(area, width, height);
    crate::ui::draw_modal_overlay(frame, rect, theme);
    let hints: &[(&str, &str)] = if state.confirm_remove {
        &[("y", "remove"), ("any key", "keep")]
    } else {
        &[
            ("j/k", "move"),
            ("t", "terminal"),
            ("y", "copy path"),
            ("d", "remove"),
            ("Esc", "close"),
        ]
    };
    let block = Block::default()
        .title(format!(" Worktrees Claude made · {} ", state.title))
        .title_bottom(key_hints(theme, hints))
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .style(Style::default().bg(theme.effective_bg()))
        .border_style(Style::default().fg(theme.primary.to_color()));
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);

    let dim = Style::default().fg(theme.text_muted.to_color());
    let mut lines = vec![
        Line::from(Span::styled(
            " Created by Claude through AIW's WorktreeCreate hook (--worktree, subagents, workflows).",
            dim,
        )),
        Line::from(""),
    ];
    for (i, wt) in worktrees.iter().enumerate() {
        let selected = i == state.selected;
        let style = if selected {
            Style::default()
                .fg(theme.primary.to_color())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text.to_color())
        };
        let marker = if selected { "▸ " } else { "  " };
        lines.push(Line::from(Span::styled(
            format!(" {marker}{}", worktree_line(wt)),
            style,
        )));
        lines.push(Line::from(Span::styled(
            format!("     {}", wt.path.display()),
            dim,
        )));
    }
    if state.confirm_remove
        && let Some(wt) = worktrees.get(state.selected)
    {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!(
                " Remove {}? Only a clean worktree is removed; its branch goes too unless it has commits.",
                wt.name
            ),
            Style::default().fg(theme.warning.to_color()),
        )));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn a_line_names_the_worktree_branch_origin_and_port() {
        let wt = ClaudeWorktree {
            id: "cwt-1".into(),
            repo: PathBuf::from("/r"),
            path: PathBuf::from("/w/claude-auth"),
            branch: "worktree-auth".into(),
            base: "abc".into(),
            name: "auth".into(),
            origin: "subagent Explore".into(),
            parent_session_id: None,
            claude_session_id: None,
            requested_cwd: PathBuf::from("/r"),
            port_start: Some(4110),
            port_count: Some(10),
            created_at: String::new(),
        };
        assert_eq!(
            worktree_line(&wt),
            "auth  ·  worktree-auth  ·  subagent Explore  ·  AIW_PORT 4110"
        );
    }
}
