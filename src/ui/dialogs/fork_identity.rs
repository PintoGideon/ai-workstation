//! FORK: the `f` fork dialog and the `E` external-sessions list
//! (DESIGN.md §19.2).

use ratatui::{
    Frame,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::app::fork_identity::{ExternalSession, ExternalSessionsState, ForkSessionState};
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

pub fn draw_fork_session_dialog(frame: &mut Frame, state: &ForkSessionState, theme: &Theme) {
    let area = frame.area();
    if area.width < 30 || area.height < 9 {
        return;
    }
    let width = 64.min(area.width);
    let height = 11.min(area.height);
    let rect = crate::ui::dialog_rect(area, width, height);
    crate::ui::draw_modal_overlay(frame, rect, theme);
    let block = Block::default()
        .title(format!(" Fork '{}' ", state.source.source_label))
        .title_bottom(key_hints(
            theme,
            &[("j/k", "choose"), ("Enter", "fork"), ("Esc", "cancel")],
        ))
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .style(Style::default().bg(theme.effective_bg()))
        .border_style(Style::default().fg(theme.primary.to_color()));
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);

    let dim = Style::default().fg(theme.text_muted.to_color());
    let option = |index: usize, title: &str, detail: &str| -> Vec<Line<'static>> {
        let selected = state.choice == index;
        let marker = if selected { "▸ " } else { "  " };
        let style = if selected {
            Style::default()
                .fg(theme.primary.to_color())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text.to_color())
        };
        vec![
            Line::from(vec![
                Span::styled(format!(" {marker}{} ", index + 1), style),
                Span::styled(title.to_string(), style),
            ]),
            Line::from(Span::styled(format!("      {detail}"), dim)),
        ]
    };
    let mut lines = vec![
        Line::from(Span::styled(
            format!(
                " A new {} session continues this conversation; the original keeps its own.",
                state.source.agent.display_name()
            ),
            dim,
        )),
        Line::from(""),
    ];
    lines.extend(option(
        0,
        "Same feature, new window",
        "shares the checkout: good for trying another answer or a side question",
    ));
    lines.extend(option(
        1,
        "New feature worktree from this branch",
        "its own branch and checkout, uncommitted changes copied: good for a second approach",
    ));
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

/// One line of an external session: name, state, where.
pub fn external_line(external: &ExternalSession) -> String {
    let entry = &external.entry;
    let kind = if entry.is_background() { "bg" } else { "tty" };
    format!(
        "{}  {}  [{kind}]  {}",
        entry.display_name(),
        entry.state_word(),
        entry.cwd
    )
}

pub fn draw_external_sessions(
    frame: &mut Frame,
    externals: &[ExternalSession],
    state: &ExternalSessionsState,
    theme: &Theme,
) {
    let area = frame.area();
    if area.width < 30 || area.height < 8 {
        return;
    }
    let width = (area.width * 8 / 10).clamp(40, 110).min(area.width);
    let height = ((externals.len() as u16) * 2 + 6).clamp(8, area.height);
    let rect = crate::ui::dialog_rect(area, width, height);
    crate::ui::draw_modal_overlay(frame, rect, theme);
    let block = Block::default()
        .title(" Claude sessions outside AIW ")
        .title_bottom(key_hints(
            theme,
            &[
                ("j/k", "move"),
                ("Enter", "copy attach command"),
                ("Esc", "close"),
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
    let mut lines = vec![
        Line::from(Span::styled(
            " Read-only: sessions Claude's agent view knows about in your projects' checkouts.",
            dim,
        )),
        Line::from(""),
    ];
    for (i, external) in externals.iter().enumerate() {
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
            format!(" {marker}{}", external_line(external)),
            style,
        )));
        let hint = external
            .entry
            .attach_command()
            .unwrap_or_else(|| "interactive in another terminal".to_string());
        lines.push(Line::from(Span::styled(
            format!("     {} · {hint}", external.project_name),
            dim,
        )));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::claude_roster::RosterEntry;

    #[test]
    fn external_lines_name_the_state_and_place() {
        let external = ExternalSession {
            project_name: "RS".into(),
            feature_id: None,
            entry: RosterEntry {
                cwd: "/repo".into(),
                kind: "background".into(),
                id: Some("7c5d".into()),
                state: Some("blocked".into()),
                waiting_for: Some("permission prompt".into()),
                ..Default::default()
            },
        };
        assert_eq!(
            external_line(&external),
            "7c5d  waiting: permission prompt  [bg]  /repo"
        );
    }
}
