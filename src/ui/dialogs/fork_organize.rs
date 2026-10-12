//! FORK: the `G` groups dialog and the `#` note editor (DESIGN.md §19.4).

use chrono::Utc;
use ratatui::{
    Frame,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
};

use crate::app::App;
use crate::app::fork_organize::{FeatureGroupsState, GroupEdit, GroupRow, NoteEditorState};
use crate::theme::{ColorDef, Theme};

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

/// The colour a group is drawn in.
pub fn group_color(name: &str) -> ratatui::style::Color {
    ColorDef::named(name).to_color()
}

pub fn draw_feature_groups(frame: &mut Frame, app: &App, state: &FeatureGroupsState) {
    let theme = &app.theme;
    let area = frame.area();
    if area.width < 30 || area.height < 9 {
        return;
    }
    let rows = app.group_dialog_rows(state);
    let pi = app.groups_dialog_project(state);
    let groups = pi.map(|pi| app.project_groups(pi)).unwrap_or_default();
    let width = 64.min(area.width);
    let height = (rows.len() as u16 + 9).clamp(9, area.height);
    let rect = crate::ui::dialog_rect(area, width, height);
    crate::ui::draw_modal_overlay(frame, rect, theme);
    let hints: &[(&str, &str)] = if state.edit.is_some() {
        &[("Enter", "save"), ("Esc", "back")]
    } else {
        &[
            ("Enter", "move here"),
            ("n", "new"),
            ("r", "rename"),
            ("c", "colour"),
            ("d", "delete"),
            ("J/K", "order"),
            ("Esc", "close"),
        ]
    };
    let block = Block::default()
        .title(format!(" Groups · {} ", state.project_name))
        .title_bottom(key_hints(theme, hints))
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .style(Style::default().bg(theme.effective_bg()))
        .border_style(Style::default().fg(theme.primary.to_color()));
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);

    let dim = Style::default().fg(theme.text_muted.to_color());
    let text = Style::default().fg(theme.text.to_color());
    let mut lines = vec![
        Line::from(Span::styled(
            match &state.feature_name {
                Some(name) => format!(" Move '{name}' to:"),
                None => " Select a feature first to move it; manage the groups here.".into(),
            },
            dim,
        )),
        Line::from(""),
    ];
    let current = state
        .feature_id
        .as_ref()
        .and_then(|id| app.organize.members.get(id))
        .cloned();
    let now = Utc::now();
    for (i, row) in rows.iter().enumerate() {
        let selected = i == state.selected;
        let marker = if selected { "▸ " } else { "  " };
        let row_style = if selected {
            Style::default()
                .fg(theme.primary.to_color())
                .add_modifier(Modifier::BOLD)
        } else {
            text
        };
        let line = match row {
            GroupRow::Ungroup => {
                let here = state.feature_id.is_some() && current.is_none();
                Line::from(vec![
                    Span::styled(format!(" {marker}"), row_style),
                    Span::styled("(no group)", row_style),
                    Span::styled(if here { "  · here now" } else { "" }, dim),
                ])
            }
            GroupRow::Group(gi) => match groups.get(*gi) {
                Some(group) => {
                    let (count, needs_you) = pi
                        .map(|pi| app.group_rollup(pi, &group.id, now))
                        .unwrap_or((0, 0));
                    let mut spans = vec![
                        Span::styled(format!(" {marker}"), row_style),
                        Span::styled("■ ", Style::default().fg(group_color(&group.color))),
                        Span::styled(group.name.clone(), row_style),
                        Span::styled(
                            format!(
                                "  {count} feature{}{}",
                                if count == 1 { "" } else { "s" },
                                if group.collapsed { " · collapsed" } else { "" }
                            ),
                            dim,
                        ),
                    ];
                    if needs_you > 0 {
                        spans.push(Span::styled(
                            format!("  ⛔{needs_you}"),
                            Style::default().fg(theme.danger.to_color()),
                        ));
                    }
                    if current.as_deref() == Some(group.id.as_str()) {
                        spans.push(Span::styled("  · here now", dim));
                    }
                    Line::from(spans)
                }
                None => Line::from(""),
            },
            GroupRow::New => Line::from(vec![
                Span::styled(format!(" {marker}"), row_style),
                Span::styled("+ New group…", row_style),
            ]),
        };
        lines.push(line);
    }
    lines.push(Line::from(""));
    if let Some((edit, value)) = &state.edit {
        let label = match edit {
            GroupEdit::New => " New group name: ",
            GroupEdit::Rename(_) => " Rename to: ",
        };
        lines.push(Line::from(vec![
            Span::styled(label, Style::default().fg(theme.primary.to_color())),
            Span::styled(value.clone(), text),
            Span::styled("█", Style::default().fg(theme.primary.to_color())),
        ]));
    }
    if let Some(id) = &state.confirm_delete {
        let name = groups
            .iter()
            .find(|g| g.id == *id)
            .map(|g| g.name.clone())
            .unwrap_or_default();
        lines.push(Line::from(Span::styled(
            format!(" Delete group '{name}'? Its features become ungrouped. y / n"),
            Style::default().fg(theme.warning.to_color()),
        )));
    }
    if let Some(error) = &state.error {
        lines.push(Line::from(Span::styled(
            format!(" {error}"),
            Style::default().fg(theme.danger.to_color()),
        )));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

pub fn draw_note_editor(frame: &mut Frame, state: &NoteEditorState, theme: &Theme) {
    let area = frame.area();
    if area.width < 30 || area.height < 8 {
        return;
    }
    let width = (area.width * 7 / 10).clamp(40, 90).min(area.width);
    let height = (state.text.lines().count() as u16 + 6).clamp(9, area.height.min(24));
    let rect = crate::ui::dialog_rect(area, width, height);
    crate::ui::draw_modal_overlay(frame, rect, theme);
    let block = Block::default()
        .title(format!(" Note · {} ", state.title))
        .title_bottom(key_hints(
            theme,
            &[
                ("Enter", "save"),
                ("Alt+Enter", "newline"),
                ("Ctrl+U", "clear"),
                ("Esc", "cancel"),
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
    let text = Style::default().fg(theme.text.to_color());
    let cursor = Span::styled("█", Style::default().fg(theme.primary.to_color()));
    let mut lines = vec![
        Line::from(Span::styled(
            " Free text; shown in the info block and found by / search. Empty deletes it.",
            dim,
        )),
        Line::from(""),
    ];
    let body: Vec<&str> = state.text.split('\n').collect();
    let last = body.len() - 1;
    for (i, line) in body.iter().enumerate() {
        let mut spans = vec![Span::styled(format!(" {line}"), text)];
        if i == last {
            spans.push(cursor.clone());
        }
        lines.push(Line::from(spans));
    }
    // Keep the end of a long note (where the cursor is) in view.
    let room = usize::from(inner.height);
    if lines.len() > room {
        let skip = lines.len() - room;
        lines.drain(..skip);
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}
