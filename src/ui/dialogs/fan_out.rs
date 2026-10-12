//! FORK: the New Feature dialog's fan-out steps (DESIGN.md §17, §19.3):
//! the layout choice and the task prompt.

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

use crate::app::CreateFeatureState;
use crate::fork::fan_out::{FanOutLayout, agent_slug, shared_labels, sibling_branches};
use crate::theme::Theme;

use super::super::dashboard::centered_rect;

fn agents_line(state: &CreateFeatureState) -> String {
    state
        .selected_agents()
        .iter()
        .map(agent_slug)
        .collect::<Vec<_>>()
        .join(" + ")
}

/// What the launch will create, one line per feature or window.
fn plan_lines(state: &CreateFeatureState) -> Vec<String> {
    let agents = state.selected_agents();
    if agents.len() < 2 {
        return vec![format!(
            "feature '{}' with {}",
            state.branch.trim(),
            state.session_name.trim()
        )];
    }
    match state.effective_fan_out() {
        FanOutLayout::WorktreeEach => sibling_branches(&state.branch, &agents)
            .into_iter()
            .zip(agents.iter())
            .map(|(branch, agent)| format!("feature '{branch}' · {}", agent.display_name()))
            .collect(),
        FanOutLayout::Shared => shared_labels(&agents)
            .into_iter()
            .map(|label| format!("window '{label}' in '{}'", state.branch.trim()))
            .collect(),
    }
}

pub fn draw_fan_out_layout(frame: &mut Frame, state: &CreateFeatureState, theme: &Theme) {
    let area = centered_rect(60, 45, frame.area());
    crate::ui::draw_modal_overlay(frame, area, theme);
    let block = Block::default()
        .title(format!(" Fan-out: {} ", agents_line(state)))
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .style(Style::default().bg(theme.effective_bg()))
        .border_style(Style::default().fg(theme.primary.to_color()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines = vec![
        Line::from(Span::styled(
            format!(
                " One prompt, {} agents. Where do they work?",
                state.selected_agents().len()
            ),
            Style::default().fg(theme.text.to_color()),
        )),
        Line::from(""),
    ];
    for layout in FanOutLayout::ALL {
        let selected = layout == state.fan_out;
        let marker = if selected { ">" } else { " " };
        let name_style = if selected {
            Style::default()
                .fg(theme.primary.to_color())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text_muted.to_color())
        };
        lines.push(Line::from(Span::styled(
            format!("   {marker} {}", layout.label()),
            name_style,
        )));
        lines.push(Line::from(Span::styled(
            format!("       {}", layout.description()),
            Style::default().fg(theme.text_muted.to_color()),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        " Creates:",
        Style::default().fg(theme.text_muted.to_color()),
    )));
    for line in plan_lines(state) {
        lines.push(Line::from(Span::styled(
            format!("   {line}"),
            Style::default().fg(theme.text.to_color()),
        )));
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(inner);
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), chunks[0]);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" j/k", Style::default().fg(theme.warning.to_color())),
            Span::raw(" choose  "),
            Span::styled("Enter", Style::default().fg(theme.warning.to_color())),
            Span::raw(" next  "),
            Span::styled("Esc", Style::default().fg(theme.warning.to_color())),
            Span::raw(" back"),
        ])),
        chunks[1],
    );
}

pub fn draw_feature_prompt(frame: &mut Frame, state: &CreateFeatureState, theme: &Theme) {
    let area = centered_rect(70, 70, frame.area());
    crate::ui::draw_modal_overlay(frame, area, theme);
    let fan_out = state.is_fan_out();
    let title = if fan_out {
        format!(" Task prompt for {} ", agents_line(state))
    } else {
        format!(" Task prompt ({}) ", state.project_name)
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .style(Style::default().bg(theme.effective_bg()))
        .border_style(Style::default().fg(theme.primary.to_color()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let plan = plan_lines(state);
    let note_rows = u16::from(state.mode_note.is_some());
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),                     // [0] explanation
            Constraint::Min(4),                        // [1] the prompt
            Constraint::Length(1),                     // [2] send checkbox
            Constraint::Length(plan.len() as u16 + 1), // [3] what is created
            Constraint::Length(note_rows),             // [4] mode note
            Constraint::Length(1),                     // [5] hints
        ])
        .split(inner);

    let explain = if fan_out {
        " Every agent gets this prompt once its pane is ready (empty = none)."
    } else {
        " The agent gets this prompt once its pane is ready (empty = none)."
    };
    frame.render_widget(
        Paragraph::new(Span::styled(
            explain,
            Style::default().fg(theme.text_muted.to_color()),
        )),
        chunks[0],
    );

    let mut text_lines: Vec<Line> = state
        .task_prompt
        .split('\n')
        .map(|l| {
            Line::from(Span::styled(
                l.to_string(),
                Style::default().fg(theme.text.to_color()),
            ))
        })
        .collect();
    if let Some(last) = text_lines.last_mut() {
        last.spans.push(Span::styled(
            "\u{2588}",
            Style::default().fg(theme.primary.to_color()),
        ));
    }
    // Keep the end of a long prompt (where the cursor is) in view.
    let height = chunks[1].height.saturating_sub(2) as usize;
    let skip = text_lines.len().saturating_sub(height.max(1));
    let prompt = Paragraph::new(text_lines.into_iter().skip(skip).collect::<Vec<_>>())
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_type(ratatui::widgets::BorderType::Rounded)
                .border_style(Style::default().fg(theme.border.to_color())),
        );
    frame.render_widget(prompt, chunks[1]);

    let check = if state.send_prompt { "[x]" } else { "[ ]" };
    let send_text = if fan_out {
        "send it to every agent when ready (off: pasted unsent, you press Enter)"
    } else {
        "send it when the agent is ready (off: pasted unsent, you press Enter)"
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Ctrl+T ", Style::default().fg(theme.warning.to_color())),
            Span::styled(
                format!("{check} {send_text}"),
                Style::default().fg(theme.text.to_color()),
            ),
        ])),
        chunks[2],
    );

    let mut plan_lines_out = vec![Line::from(Span::styled(
        " Creates:",
        Style::default().fg(theme.text_muted.to_color()),
    ))];
    for line in plan {
        plan_lines_out.push(Line::from(Span::styled(
            format!("   {line}"),
            Style::default().fg(theme.text_muted.to_color()),
        )));
    }
    frame.render_widget(Paragraph::new(plan_lines_out), chunks[3]);

    if let Some(note) = &state.mode_note {
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(" Note: {note}"),
                Style::default().fg(theme.warning.to_color()),
            )),
            chunks[4],
        );
    }

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Enter", Style::default().fg(theme.warning.to_color())),
            Span::raw(" create  "),
            Span::styled("Alt+Enter", Style::default().fg(theme.warning.to_color())),
            Span::raw(" newline  "),
            Span::styled("Ctrl+L", Style::default().fg(theme.warning.to_color())),
            Span::raw(" library  "),
            Span::styled("Ctrl+U", Style::default().fg(theme.warning.to_color())),
            Span::raw(" clear  "),
            Span::styled("Esc", Style::default().fg(theme.warning.to_color())),
            Span::raw(" back"),
        ])),
        chunks[5],
    );

    if let Some(picker) = &state.prompt_picker {
        let pick_area = centered_rect(50, 50, frame.area());
        frame.render_widget(Clear, pick_area);
        let items: Vec<ListItem> = picker
            .entries
            .iter()
            .map(|(name, body)| {
                let first = body.lines().next().unwrap_or("").to_string();
                ListItem::new(vec![
                    Line::from(Span::styled(
                        name.clone(),
                        Style::default().fg(theme.text.to_color()),
                    )),
                    Line::from(Span::styled(
                        format!("  {first}"),
                        Style::default().fg(theme.text_muted.to_color()),
                    )),
                ])
            })
            .collect();
        let list = List::new(items)
            .block(
                Block::default()
                    .title(" Prompt library — Enter inserts, Esc closes ")
                    .borders(Borders::ALL)
                    .border_type(ratatui::widgets::BorderType::Rounded)
                    .style(Style::default().bg(theme.effective_bg()))
                    .border_style(Style::default().fg(theme.primary.to_color())),
            )
            .highlight_style(Style::default().bg(theme.effective_selection_bg()));
        let mut list_state = ListState::default();
        list_state.select(Some(picker.selected));
        frame.render_stateful_widget(list, pick_area, &mut list_state);
    }
}
