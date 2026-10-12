//! FORK: the schedules list and the add / edit dialog (DESIGN.md §19.9).

use ratatui::{
    Frame,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::app::App;
use crate::app::fork_schedules::{EditorField, ScheduleEditorState, ScheduleListState};
use crate::fork::schedules::{CatchUp, ScheduleKind};

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

pub fn draw_schedules(frame: &mut Frame, app: &App, state: &ScheduleListState) {
    let theme = &app.theme;
    let area = frame.area();
    if area.width < 36 || area.height < 8 {
        return;
    }
    let rows = app.schedule_rows(state);
    let history = state
        .show_history
        .then(|| {
            rows.get(state.selected)
                .map(|row| app.schedule_history(&row.id))
        })
        .flatten();
    let history_lines = history
        .as_ref()
        .map(|h| h.len().clamp(1, 12) + 2)
        .unwrap_or(0);
    let width = (area.width * 8 / 10).clamp(50, 110).min(area.width);
    let body = (rows.len().max(1) * 2 + 2 + history_lines) as u16;
    let height = (body + 2).min(area.height);
    let rect = crate::ui::dialog_rect(area, width, height);
    crate::ui::draw_modal_overlay(frame, rect, theme);
    let key = crate::ui::fork_chrome::key_style(theme);
    let hints = if state.confirm_delete {
        Line::from(vec![
            Span::styled(" y", key),
            Span::raw(" delete  "),
            Span::styled("any key", key),
            Span::raw(" keep "),
        ])
    } else {
        Line::from(vec![
            Span::styled(" n", key),
            Span::raw(" new  "),
            Span::styled("e", key),
            Span::raw(" edit  "),
            Span::styled("Space", key),
            Span::raw(" pause  "),
            Span::styled("r", key),
            Span::raw(" run now  "),
            Span::styled("d", key),
            Span::raw(" delete  "),
            Span::styled("h", key),
            Span::raw(" history  "),
            Span::styled("Esc", key),
            Span::raw(" close "),
        ])
    };
    let block = Block::default()
        .title(format!(" Schedules · {} ", state.title))
        .title_bottom(hints)
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .style(Style::default().bg(theme.effective_bg()))
        .border_style(Style::default().fg(theme.border_focus.to_color()));
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);

    let dim = Style::default().fg(theme.text_muted.to_color());
    let text = Style::default().fg(theme.text.to_color());
    let accent = Style::default().fg(theme.primary.to_color());
    let warn = Style::default().fg(theme.warning.to_color());
    let wide = usize::from(inner.width).saturating_sub(4);
    let mut lines = Vec::new();
    if rows.is_empty() {
        lines.push(Line::from(Span::styled(
            " No schedules here. n adds one: a prompt for an agent, or a command for a terminal.",
            dim,
        )));
    }
    for (i, row) in rows.iter().enumerate() {
        let selected = i == state.selected;
        let glyph = if row.finished {
            "✓"
        } else if row.paused {
            "⏸"
        } else {
            "⏲"
        };
        let mut head = vec![
            Span::styled(if selected { "▸ " } else { "  " }, accent),
            Span::styled(format!("{glyph} "), accent),
            Span::styled(
                row.spec.clone(),
                if selected {
                    text.add_modifier(Modifier::BOLD)
                } else {
                    text
                },
            ),
            Span::styled(format!("  {}", row.target), accent),
        ];
        if matches!(
            state.scope,
            crate::app::fork_schedules::ScheduleScope::Project { .. }
        ) {
            head.push(Span::styled(format!("  · {}", row.feature), dim));
        }
        if row.from_config {
            head.push(Span::styled("  (aiw.json)", dim));
        }
        lines.push(Line::from(head));
        let mut detail = vec![Span::styled("    ", dim)];
        detail.push(Span::styled(
            format!("\"{}\"", truncate(&row.text, wide / 2)),
            dim,
        ));
        let state_text = if row.finished {
            "  · finished".to_string()
        } else if row.paused {
            "  · paused".to_string()
        } else {
            row.next
                .as_ref()
                .map(|n| format!("  · next {n}"))
                .unwrap_or_default()
        };
        detail.push(Span::styled(state_text, dim));
        if row.deferred {
            detail.push(Span::styled("  · waiting on you", warn));
        }
        if let Some(last) = &row.last {
            detail.push(Span::styled(format!("  · last {last}"), dim));
        }
        lines.push(Line::from(detail));
    }
    if let Some(history) = history {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(" History (newest first)", accent)));
        if history.is_empty() {
            lines.push(Line::from(Span::styled("   no runs yet", dim)));
        }
        let now = chrono::Utc::now();
        for run in history.iter().take(12) {
            lines.push(Line::from(vec![
                Span::styled(
                    format!(
                        "   {}  ",
                        crate::fork::schedules::short_when(run.at, now, &chrono::Local)
                    ),
                    dim,
                ),
                Span::styled(run.outcome.clone(), text),
                Span::styled(
                    if run.detail.is_empty() {
                        String::new()
                    } else {
                        format!("  ({})", run.detail)
                    },
                    dim,
                ),
            ]));
        }
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

pub fn draw_schedule_editor(frame: &mut Frame, app: &App, state: &ScheduleEditorState) {
    let theme = &app.theme;
    let area = frame.area();
    if area.width < 40 || area.height < 12 {
        return;
    }
    let text_lines = state.text.lines().count().clamp(1, 6) as u16;
    let width = (area.width * 8 / 10).clamp(56, 100).min(area.width);
    let height = (EditorField::ALL.len() as u16 + text_lines + 6).min(area.height);
    let rect = crate::ui::dialog_rect(area, width, height);
    crate::ui::draw_modal_overlay(frame, rect, theme);
    let key = crate::ui::fork_chrome::key_style(theme);
    let hints = Line::from(vec![
        Span::styled(" Enter", key),
        Span::raw(" save  "),
        Span::styled("Tab", key),
        Span::raw(" next  "),
        Span::styled("←→", key),
        Span::raw(" change  "),
        Span::styled("Alt+Enter", key),
        Span::raw(" new line  "),
        Span::styled("Esc", key),
        Span::raw(" back "),
    ]);
    let title = match &state.editing {
        Some(_) => format!(" Edit schedule · {} ", state.feature),
        None => format!(" New schedule · {} ", state.feature),
    };
    let block = Block::default()
        .title(title)
        .title_bottom(hints)
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .style(Style::default().bg(theme.effective_bg()))
        .border_style(Style::default().fg(theme.border_focus.to_color()));
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);

    let dim = Style::default().fg(theme.text_muted.to_color());
    let text = Style::default().fg(theme.text.to_color());
    let accent = Style::default().fg(theme.primary.to_color());
    let danger = Style::default().fg(theme.danger.to_color());
    let mut lines = Vec::new();
    for field in EditorField::ALL {
        let focused = field == state.focus;
        let label = Span::styled(
            format!("{} {:<12}", if focused { "▸" } else { " " }, field.label()),
            if focused {
                accent.add_modifier(Modifier::BOLD)
            } else {
                dim
            },
        );
        let cursor = if focused { "▏" } else { "" };
        let value: Vec<Span> = match field {
            EditorField::Kind => vec![Span::styled(
                match state.kind {
                    ScheduleKind::Prompt => "‹ a prompt for an agent ›",
                    ScheduleKind::Shell => "‹ a command in a terminal ›",
                },
                text,
            )],
            EditorField::Target => match state.kind {
                ScheduleKind::Prompt => vec![Span::styled(
                    format!(
                        "‹ {} ›",
                        state
                            .targets
                            .get(state.target_index)
                            .map(|(_, l)| l.as_str())
                            .unwrap_or("the feature's first agent")
                    ),
                    text,
                )],
                ScheduleKind::Shell => vec![
                    Span::styled(format!("terminal {}{cursor}", state.terminal), text),
                    Span::styled("  (created when missing)", dim),
                ],
            },
            EditorField::When => {
                let mut v = vec![Span::styled(format!("{}{cursor}", state.spec), text)];
                if state.spec.trim().is_empty() {
                    v.push(Span::styled(
                        "every 30m · daily 09:00 · cron 0 9 * * 1-5 · at 2026-10-12 15:00",
                        dim,
                    ));
                }
                v
            }
            EditorField::Text => {
                let first = state.text.lines().next().unwrap_or_default().to_string();
                vec![Span::styled(
                    if state.text.contains('\n') {
                        first
                    } else {
                        format!("{}{cursor}", state.text)
                    },
                    text,
                )]
            }
            EditorField::StartIfStopped => vec![Span::styled(
                if state.start_if_stopped {
                    "‹ start it (resource limits permitting) ›"
                } else {
                    "‹ skip the run ›"
                },
                text,
            )],
            EditorField::CatchUp => vec![Span::styled(
                match state.catch_up {
                    CatchUp::Once => "‹ run once when aiw starts ›",
                    CatchUp::None => "‹ skip them ›",
                },
                text,
            )],
            EditorField::MaxRuns => vec![
                Span::styled(format!("{}{cursor}", state.max_runs), text),
                Span::styled(
                    if state.max_runs.is_empty() {
                        "no limit"
                    } else {
                        ""
                    },
                    dim,
                ),
            ],
            EditorField::End => vec![
                Span::styled(format!("{}{cursor}", state.end), text),
                Span::styled(
                    if state.end.is_empty() {
                        "never (YYYY-MM-DD HH:MM)"
                    } else {
                        ""
                    },
                    dim,
                ),
            ],
        };
        let mut spans = vec![label];
        spans.extend(value);
        lines.push(Line::from(spans));
        // The rest of a multi-line text.
        if field == EditorField::Text && state.text.contains('\n') {
            for (i, extra) in state.text.lines().skip(1).take(5).enumerate() {
                let last = i + 2 == state.text.lines().count().min(6);
                lines.push(Line::from(vec![
                    Span::raw(" ".repeat(14)),
                    Span::styled(
                        format!(
                            "{extra}{}",
                            if last && focused_text(state) {
                                "▏"
                            } else {
                                ""
                            }
                        ),
                        text,
                    ),
                ]));
            }
        }
    }
    lines.push(Line::from(""));
    match &state.error {
        Some(error) => lines.push(Line::from(Span::styled(format!(" ✗ {error}"), danger))),
        None => match app.schedule_editor_preview(state) {
            Ok(preview) => lines.push(Line::from(Span::styled(format!(" ✓ {preview}"), accent))),
            Err(e) if state.spec.trim().is_empty() => {
                let _ = e;
                lines.push(Line::from(Span::styled(
                    " When is it due? Prompts are queued while the agent works.",
                    dim,
                )))
            }
            Err(e) => lines.push(Line::from(Span::styled(format!(" ✗ {e}"), danger))),
        },
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn focused_text(state: &ScheduleEditorState) -> bool {
    state.focus == EditorField::Text
}
