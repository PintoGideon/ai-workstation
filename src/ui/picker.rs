use chrono::{Local, TimeZone};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};

use crate::app::attention::AttentionRow;
use crate::app::{
    BookmarkPickerState, ClaudeSessionPickerState, CodexSessionPickerState, CommandAction,
    CommandPickerState, MarkdownFilePickerState, PendingInput, SessionPickerState,
    SyntaxLanguagePickerState, SyntaxOperationAction,
};
use crate::custom_session_icons::resolve_custom_session_icon;
use crate::theme::Theme;
use crate::usage::{UsageData, format_usage_summary, usage_windows_for_session_kind};

use super::dashboard::centered_rect;
use super::fork_text::{fit_text, relative_age};

/// The needs-attention overlay (`i`).
///
/// Rows come pre-ordered from `App::attention_rows`: questions first, then
/// completions, then generic waits, then whatever pending inputs the attention
/// layer could not explain. Each row is labelled with its state rather than
/// with the agent's message — AIW deliberately does not capture the question
/// text, so the label is all there is to say without opening the session.
/// FORK: the badge (DESIGN.md §15) of the feature each row belongs to, so
/// the picker shows the same glyph, reason and tone as the dashboard rows.
pub(crate) fn row_badges(
    app: &crate::app::App,
    rows: &[AttentionRow],
) -> Vec<Option<crate::fork::badges::SessionBadge>> {
    let now = chrono::Utc::now();
    rows.iter()
        .map(|row| match row {
            AttentionRow::Attention { entry, .. } => app
                .store
                .projects
                .iter()
                .enumerate()
                .find_map(|(pi, project)| {
                    project
                        .features
                        .iter()
                        .position(|f| f.tmux_session == entry.tmux_session)
                        .map(|fi| crate::fork::badges::feature_badge(app, pi, fi, now))
                }),
            AttentionRow::Pending(index) => {
                let input = app.pending_inputs.get(*index)?;
                let feature_name = input.feature_name.as_deref()?;
                app.store
                    .projects
                    .iter()
                    .enumerate()
                    .find_map(|(pi, project)| {
                        project
                            .features
                            .iter()
                            .position(|f| f.name == feature_name)
                            .map(|fi| crate::fork::badges::feature_badge(app, pi, fi, now))
                    })
            }
        })
        .collect()
}

pub fn draw_notification_picker(
    frame: &mut Frame,
    rows: &[AttentionRow],
    pending: &[PendingInput],
    badges: &[Option<crate::fork::badges::SessionBadge>],
    selected: usize,
    nerd_font: bool,
    theme: &Theme,
) {
    let now = chrono::Utc::now();
    let area = centered_rect(60, 50, frame.area());
    crate::ui::draw_modal_overlay(frame, area, theme);

    let title = format!(" Needs Attention ({}) ", rows.len());
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .style(Style::default().bg(theme.effective_bg()))
        .border_style(Style::default().fg(theme.warning.to_color()));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if rows.is_empty() {
        let empty = Paragraph::new(Line::from(Span::styled(
            "  Nothing needs attention.",
            Style::default().fg(theme.text_muted.to_color()),
        )));
        frame.render_widget(empty, inner);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)])
        .split(inner);

    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let is_selected = i == selected;
            let input = row.pending_index().and_then(|index| pending.get(index));

            // An attention row knows its own feature; a bare pending input
            // carries the names it was filed under.
            let (proj, feat) = match row {
                AttentionRow::Attention { entry, .. } => {
                    (entry.project_name.as_str(), entry.feature_name.as_str())
                }
                AttentionRow::Pending(_) => (
                    input
                        .and_then(|i| i.project_name.as_deref())
                        .unwrap_or("unknown"),
                    input
                        .and_then(|i| i.feature_name.as_deref())
                        .unwrap_or("unknown"),
                ),
            };

            // What the row is asking for. FORK: the badge says it (glyph,
            // reason, tone) when the row's feature is known; an explained
            // stop without one shows its state; an unexplained one falls
            // back to the pending input's own description.
            let badge = badges.get(i).and_then(|b| b.as_ref());
            let (detail, detail_style) = match (badge, row.state()) {
                (Some(badge), _) => (
                    format!("{} {}", badge.glyph(), badge.label(now)),
                    badge.style(theme).add_modifier(Modifier::BOLD),
                ),
                (None, Some(state)) => (
                    format!("{} {}", state.glyph(nerd_font), state.label()),
                    Style::default()
                        .fg(state.color(theme))
                        .add_modifier(Modifier::BOLD),
                ),
                (None, None) => {
                    let input_detail = input.map(|input| {
                        if input.message.is_empty() {
                            input.notification_type.clone()
                        } else {
                            // Columns, not bytes: a multibyte message must
                            // never be sliced mid-character.
                            fit_text(&input.message, 50)
                        }
                    });
                    (
                        input_detail.unwrap_or_else(|| "pending".to_string()),
                        Style::default().fg(theme.text_muted.to_color()),
                    )
                }
            };

            let line = Line::from(vec![
                Span::styled(
                    format!("  {} ", proj),
                    Style::default()
                        .fg(theme.project_title.to_color())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("/ {} ", feat),
                    Style::default().fg(theme.feature_title.to_color()),
                ),
                Span::styled("- ", Style::default().fg(theme.text_muted.to_color())),
                Span::styled(detail, detail_style),
            ]);

            if is_selected {
                ListItem::new(line).style(Style::default().bg(theme.effective_selection_bg()))
            } else {
                ListItem::new(line)
            }
        })
        .collect();

    let list = List::new(items);
    let mut list_state = ListState::default().with_selected(Some(selected));
    frame.render_stateful_widget(list, chunks[0], &mut list_state);

    let hints = Paragraph::new(Line::from(vec![
        Span::styled(
            "  j/k or \u{2191}/\u{2193}",
            Style::default().fg(theme.warning.to_color()),
        ),
        Span::styled(
            " navigate  ",
            Style::default().fg(theme.text_muted.to_color()),
        ),
        Span::styled("Enter", Style::default().fg(theme.warning.to_color())),
        Span::styled(
            " select  ",
            Style::default().fg(theme.text_muted.to_color()),
        ),
        Span::styled("x", Style::default().fg(theme.warning.to_color())),
        Span::styled(
            " dismiss  ",
            Style::default().fg(theme.text_muted.to_color()),
        ),
        Span::styled("Esc", Style::default().fg(theme.warning.to_color())),
        Span::styled(" cancel", Style::default().fg(theme.text_muted.to_color())),
    ]));
    frame.render_widget(hints, chunks[1]);
}

pub fn draw_command_picker(frame: &mut Frame, state: &CommandPickerState, theme: &Theme) {
    let area = centered_rect(50, 50, frame.area());
    crate::ui::draw_modal_overlay(frame, area, theme);

    let title = format!(" Custom Commands ({}) ", state.commands.len());
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .style(Style::default().bg(theme.effective_bg()))
        .border_style(Style::default().fg(theme.primary.to_color()));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.commands.is_empty() {
        let empty = Paragraph::new(Line::from(Span::styled(
            "  No custom commands found.",
            Style::default().fg(theme.text_muted.to_color()),
        )));
        frame.render_widget(empty, inner);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)])
        .split(inner);

    let mut items: Vec<ListItem> = Vec::new();
    let mut current_source = String::new();
    let mut selected_row = None;

    for (i, cmd) in state.commands.iter().enumerate() {
        if cmd.source != current_source {
            if !current_source.is_empty() {
                items.push(ListItem::new(Line::from("")));
            }
            current_source = cmd.source.clone();
            items.push(ListItem::new(Line::from(Span::styled(
                format!("  {} Commands", cmd.source),
                Style::default()
                    .fg(theme.primary.to_color())
                    .add_modifier(Modifier::BOLD),
            ))));
        }

        let is_selected = i == state.selected;
        let prefix = match cmd.action {
            CommandAction::SlashCommand => "/",
            CommandAction::Local { .. } => "*",
            CommandAction::CodexLiveDemo(_) => "*",
        };
        let line = Line::from(vec![
            Span::styled(
                format!("    {prefix}"),
                Style::default().fg(theme.text_muted.to_color()),
            ),
            Span::styled(
                &cmd.name,
                if is_selected {
                    Style::default()
                        .fg(theme.text.to_color())
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.text.to_color())
                },
            ),
        ]);

        if is_selected {
            selected_row = Some(items.len());
            items.push(
                ListItem::new(line).style(Style::default().bg(theme.effective_selection_bg())),
            );
        } else {
            items.push(ListItem::new(line));
        }
    }

    let list = List::new(items);
    let mut list_state = ListState::default().with_selected(selected_row);
    frame.render_stateful_widget(list, chunks[0], &mut list_state);

    let hints = Paragraph::new(Line::from(vec![
        Span::styled(
            "  j/k or \u{2191}/\u{2193}",
            Style::default().fg(theme.warning.to_color()),
        ),
        Span::styled(
            " navigate  ",
            Style::default().fg(theme.text_muted.to_color()),
        ),
        Span::styled("Enter", Style::default().fg(theme.warning.to_color())),
        Span::styled(" run  ", Style::default().fg(theme.text_muted.to_color())),
        Span::styled("Esc", Style::default().fg(theme.warning.to_color())),
        Span::styled(" cancel", Style::default().fg(theme.text_muted.to_color())),
    ]));
    frame.render_widget(hints, chunks[1]);
}

pub fn draw_syntax_language_picker(
    frame: &mut Frame,
    state: &SyntaxLanguagePickerState,
    throbber_state: &throbber_widgets_tui::ThrobberState,
    theme: &Theme,
) {
    let installed = state
        .languages
        .iter()
        .filter(|row| {
            matches!(
                row.status,
                crate::highlight::HighlightInstallState::Installed
            )
        })
        .count();
    let area = centered_rect(68, 58, frame.area());
    crate::ui::draw_modal_overlay(frame, area, theme);

    let title = format!(" Syntax Parsers ({}/{}) ", installed, state.languages.len());
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .style(Style::default().bg(theme.effective_bg()))
        .border_style(Style::default().fg(theme.info.to_color()));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(2),
        ])
        .split(inner);

    let status_line = if let Some(operation) = &state.operation {
        let verb = match operation.action {
            // FORK(no-cloud): say plainly that this goes to the network.
            SyntaxOperationAction::Install => "Downloading (git clone) + building",
            SyntaxOperationAction::Uninstall => "Removing",
        };
        let throbber = throbber_widgets_tui::Throbber::default()
            .throbber_style(
                Style::default()
                    .fg(theme.warning.to_color())
                    .add_modifier(Modifier::BOLD),
            )
            .throbber_set(throbber_widgets_tui::BRAILLE_EIGHT_DOUBLE)
            .use_type(throbber_widgets_tui::WhichUse::Spin);
        let spinner = throbber.to_symbol_span(throbber_state);
        let detail = operation
            .last_output
            .clone()
            .unwrap_or_else(|| "Working...".to_string());
        let elapsed = operation.started_at.elapsed().as_secs();
        Paragraph::new(vec![
            Line::from(vec![
                Span::styled(
                    format!("  {} {}  ", verb, operation.language.picker_title()),
                    Style::default()
                        .fg(theme.warning.to_color())
                        .add_modifier(Modifier::BOLD),
                ),
                spinner,
                Span::styled(
                    format!("  {}s elapsed", elapsed),
                    Style::default().fg(theme.text_muted.to_color()),
                ),
            ]),
            Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled(detail, Style::default().fg(theme.text_muted.to_color())),
            ]),
        ])
    } else if let Some(notice) = &state.notice {
        let color = if notice.starts_with("Error:") {
            theme.danger.to_color()
        } else {
            theme.success.to_color()
        };
        Paragraph::new(Line::from(Span::styled(
            format!("  {}", notice),
            Style::default().fg(color),
        )))
    } else {
        Paragraph::new(Line::from(Span::styled(
            "  Installing downloads the parser from GitHub (git clone) and builds it locally.",
            Style::default().fg(theme.text_muted.to_color()),
        )))
    };
    frame.render_widget(status_line, chunks[0]);

    let items: Vec<ListItem> = state
        .languages
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let is_selected = i == state.selected;
            let is_operating = state
                .operation
                .as_ref()
                .is_some_and(|op| op.language == row.language);

            let (badge_label, badge_style) = if is_operating {
                (
                    match state.operation.as_ref().map(|op| op.action) {
                        Some(SyntaxOperationAction::Install) => " DOWNLOADING ",
                        Some(SyntaxOperationAction::Uninstall) => " REMOVING ",
                        None => " WORKING ",
                    },
                    Style::default()
                        .fg(theme.effective_bg())
                        .bg(theme.warning.to_color())
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                match row.status {
                    crate::highlight::HighlightInstallState::Installed => (
                        " INSTALLED ",
                        Style::default()
                            .fg(theme.effective_bg())
                            .bg(theme.success.to_color())
                            .add_modifier(Modifier::BOLD),
                    ),
                    crate::highlight::HighlightInstallState::Available => (
                        " AVAILABLE ",
                        Style::default()
                            .fg(theme.effective_bg())
                            .bg(theme.text_muted.to_color())
                            .add_modifier(Modifier::BOLD),
                    ),
                    crate::highlight::HighlightInstallState::Broken => (
                        " BROKEN ",
                        Style::default()
                            .fg(theme.effective_bg())
                            .bg(theme.danger.to_color())
                            .add_modifier(Modifier::BOLD),
                    ),
                }
            };

            let line = Line::from(vec![
                Span::styled(
                    if is_selected { "  > " } else { "    " },
                    Style::default().fg(theme.warning.to_color()),
                ),
                Span::styled(badge_label, badge_style),
                Span::styled(" ", Style::default()),
                Span::styled(
                    row.language.picker_title(),
                    if is_selected {
                        Style::default()
                            .fg(theme.text.to_color())
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme.text.to_color())
                    },
                ),
                Span::styled(
                    format!("  {}  ", row.language.extension_summary()),
                    Style::default().fg(theme.primary.to_color()),
                ),
                Span::styled(
                    row.language.picker_description(),
                    Style::default().fg(theme.text_muted.to_color()),
                ),
            ]);

            if is_selected {
                ListItem::new(line).style(Style::default().bg(theme.effective_selection_bg()))
            } else {
                ListItem::new(line)
            }
        })
        .collect();

    frame.render_widget(List::new(items), chunks[1]);

    let hints = if state.operation.is_some() {
        Paragraph::new(Line::from(vec![
            Span::styled("  wait", Style::default().fg(theme.warning.to_color())),
            Span::styled(
                " for the current parser operation to finish",
                Style::default().fg(theme.text_muted.to_color()),
            ),
        ]))
    } else {
        Paragraph::new(Line::from(vec![
            Span::styled(
                "  j/k or \u{2191}/\u{2193}",
                Style::default().fg(theme.warning.to_color()),
            ),
            Span::styled(
                " navigate  ",
                Style::default().fg(theme.text_muted.to_color()),
            ),
            Span::styled("Enter/i", Style::default().fg(theme.warning.to_color())),
            Span::styled(
                " download + install  ",
                Style::default().fg(theme.text_muted.to_color()),
            ),
            Span::styled("x", Style::default().fg(theme.warning.to_color())),
            Span::styled(
                " uninstall  ",
                Style::default().fg(theme.text_muted.to_color()),
            ),
            Span::styled("r", Style::default().fg(theme.warning.to_color())),
            Span::styled(
                " refresh  ",
                Style::default().fg(theme.text_muted.to_color()),
            ),
            Span::styled("Esc", Style::default().fg(theme.warning.to_color())),
            Span::styled(" cancel", Style::default().fg(theme.text_muted.to_color())),
        ]))
    };
    frame.render_widget(hints, chunks[2]);
}

pub fn draw_markdown_file_picker(
    frame: &mut Frame,
    state: &MarkdownFilePickerState,
    theme: &Theme,
) {
    let mut visible_matches: Vec<(usize, usize)> = state
        .files
        .iter()
        .enumerate()
        .filter(|(_, path)| {
            !state.plan_only
                || crate::markdown::markdown_view_relative_label(
                    path,
                    &state.workdir,
                    state.repo_root.as_deref(),
                )
                .to_ascii_lowercase()
                .contains("plan")
        })
        .filter_map(|(idx, path)| {
            crate::app::util::markdown_file_picker_score(
                path,
                &state.workdir,
                state.repo_root.as_deref(),
                &state.query,
            )
            .map(|score| (idx, score))
        })
        .collect();
    visible_matches.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    let visible_indices: Vec<usize> = visible_matches.into_iter().map(|(idx, _)| idx).collect();
    let visible_count = visible_indices.len();
    let selected_visible = visible_indices
        .iter()
        .position(|&idx| idx == state.selected)
        .or_else(|| (!visible_indices.is_empty()).then_some(0));
    let showing_repo_root = state.repo_root.is_some();
    let area = if showing_repo_root {
        centered_rect(70, 60, frame.area())
    } else {
        centered_rect(62, 52, frame.area())
    };
    crate::ui::draw_modal_overlay(frame, area, theme);

    let selecting_plan = matches!(
        state.purpose,
        crate::app::MarkdownFilePickerPurpose::SelectPlan { .. }
    );
    let title = if selecting_plan {
        format!(
            " Select Current Plan — Worktree Markdown ({}/{}) ",
            visible_count,
            state.files.len()
        )
    } else if showing_repo_root {
        format!(
            " Markdown Files: Worktree + Repo Root ({}/{}) ",
            visible_count,
            state.files.len()
        )
    } else {
        format!(" Markdown Files ({}/{}) ", visible_count, state.files.len())
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .style(Style::default().bg(theme.effective_bg()))
        .border_style(Style::default().fg(theme.info.to_color()));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(if showing_repo_root {
            vec![
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(2),
            ]
        } else {
            vec![
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(2),
            ]
        })
        .split(inner);

    if showing_repo_root {
        let legend = Paragraph::new(Line::from(vec![
            Span::styled(
                "  WORKTREE",
                Style::default()
                    .fg(theme.warning.to_color())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " = current feature dir    ",
                Style::default().fg(theme.text_muted.to_color()),
            ),
            Span::styled(
                "REPO ROOT",
                Style::default()
                    .fg(theme.info.to_color())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                " = main repo dir",
                Style::default().fg(theme.text_muted.to_color()),
            ),
        ]));
        frame.render_widget(legend, chunks[0]);
    }

    let query_chunk = if showing_repo_root {
        chunks[1]
    } else {
        chunks[0]
    };
    let list_chunk = if showing_repo_root {
        chunks[2]
    } else {
        chunks[1]
    };
    let hint_chunk = if showing_repo_root {
        chunks[3]
    } else {
        chunks[2]
    };

    let query_line = if state.search_active {
        let query_text = if state.query.is_empty() {
            "Type to filter markdown files"
        } else {
            &state.query
        };
        let query_style = if state.query.is_empty() {
            Style::default().fg(theme.text_muted.to_color())
        } else {
            Style::default().fg(theme.text.to_color())
        };
        Paragraph::new(Line::from(vec![
            Span::styled(
                "  / ",
                Style::default()
                    .fg(theme.warning.to_color())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(query_text, query_style),
        ]))
    } else {
        let text = if state.query.is_empty() {
            "  Press / to search".to_string()
        } else {
            format!("  Search: {}", state.query)
        };
        Paragraph::new(Line::from(Span::styled(
            text,
            Style::default().fg(theme.text_muted.to_color()),
        )))
    };
    frame.render_widget(query_line, query_chunk);

    let items: Vec<ListItem> = visible_indices
        .iter()
        .map(|&idx| {
            let path = &state.files[idx];
            let is_selected = idx == state.selected;
            let scope = crate::markdown::markdown_view_scope(
                path,
                &state.workdir,
                state.repo_root.as_deref(),
            );
            let scope_label = match scope {
                crate::markdown::MarkdownViewScope::Worktree => " WORKTREE ",
                crate::markdown::MarkdownViewScope::RepoRoot => " REPO ROOT ",
                crate::markdown::MarkdownViewScope::Other => " PATH ",
            };
            let scope_style = match scope {
                crate::markdown::MarkdownViewScope::Worktree => Style::default()
                    .fg(theme.effective_bg())
                    .bg(theme.warning.to_color())
                    .add_modifier(Modifier::BOLD),
                crate::markdown::MarkdownViewScope::RepoRoot => Style::default()
                    .fg(theme.effective_bg())
                    .bg(theme.info.to_color())
                    .add_modifier(Modifier::BOLD),
                crate::markdown::MarkdownViewScope::Other => Style::default()
                    .fg(theme.effective_bg())
                    .bg(theme.text_muted.to_color())
                    .add_modifier(Modifier::BOLD),
            };
            let label = crate::markdown::markdown_view_relative_label(
                path,
                &state.workdir,
                state.repo_root.as_deref(),
            );
            let line = Line::from(vec![
                Span::styled(
                    if is_selected { "  > " } else { "    " },
                    Style::default().fg(theme.warning.to_color()),
                ),
                Span::styled(scope_label, scope_style),
                Span::styled(" ", Style::default().fg(theme.text_muted.to_color())),
                Span::styled(
                    label,
                    if is_selected {
                        Style::default()
                            .fg(theme.text.to_color())
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme.text.to_color())
                    },
                ),
            ]);
            if is_selected {
                ListItem::new(line).style(Style::default().bg(theme.effective_selection_bg()))
            } else {
                ListItem::new(line)
            }
        })
        .collect();

    if items.is_empty() {
        let empty = Paragraph::new(Line::from(Span::styled(
            "  No markdown files match the current filter.",
            Style::default().fg(theme.text_muted.to_color()),
        )));
        frame.render_widget(empty, list_chunk);
    } else {
        let list = List::new(items);
        let mut list_state = ListState::default();
        list_state.select(selected_visible);
        frame.render_stateful_widget(list, list_chunk, &mut list_state);
    }

    let hints = if state.search_active {
        Paragraph::new(Line::from(vec![
            Span::styled(
                "  \u{2191}/\u{2193} or Tab/Shift+Tab",
                Style::default().fg(theme.warning.to_color()),
            ),
            Span::styled(
                " navigate  ",
                Style::default().fg(theme.text_muted.to_color()),
            ),
            Span::styled("Enter", Style::default().fg(theme.warning.to_color())),
            Span::styled(
                if selecting_plan {
                    " select  "
                } else {
                    " open  "
                },
                Style::default().fg(theme.text_muted.to_color()),
            ),
            Span::styled("Backspace", Style::default().fg(theme.warning.to_color())),
            Span::styled(
                " clear filter  ",
                Style::default().fg(theme.text_muted.to_color()),
            ),
            Span::styled("Esc", Style::default().fg(theme.warning.to_color())),
            Span::styled(
                " back to selection mode",
                Style::default().fg(theme.text_muted.to_color()),
            ),
        ]))
    } else {
        let mut spans = vec![
            Span::styled(
                "  j/k or \u{2191}/\u{2193}",
                Style::default().fg(theme.warning.to_color()),
            ),
            Span::styled(
                " navigate  ",
                Style::default().fg(theme.text_muted.to_color()),
            ),
            Span::styled("Enter", Style::default().fg(theme.warning.to_color())),
            Span::styled(
                if selecting_plan {
                    " select  "
                } else {
                    " open  "
                },
                Style::default().fg(theme.text_muted.to_color()),
            ),
            Span::styled("/", Style::default().fg(theme.warning.to_color())),
            Span::styled(
                " search  ",
                Style::default().fg(theme.text_muted.to_color()),
            ),
        ];
        if !selecting_plan {
            spans.push(Span::styled(
                "p",
                Style::default().fg(theme.warning.to_color()),
            ));
            spans.push(Span::styled(
                if state.plan_only {
                    " all-files  "
                } else {
                    " plan-only  "
                },
                Style::default().fg(theme.text_muted.to_color()),
            ));
        }
        spans.push(Span::styled(
            "Esc",
            Style::default().fg(theme.warning.to_color()),
        ));
        spans.push(Span::styled(
            " cancel",
            Style::default().fg(theme.text_muted.to_color()),
        ));
        Paragraph::new(Line::from(spans))
    };
    frame.render_widget(hints, hint_chunk);
}

pub fn draw_bookmark_picker(
    frame: &mut Frame,
    state: &BookmarkPickerState,
    rows: &[String],
    theme: &Theme,
) {
    let area = centered_rect(56, 42, frame.area());
    crate::ui::draw_modal_overlay(frame, area, theme);

    let block = Block::default()
        .title(" Session Bookmarks ")
        .borders(Borders::ALL)
        .style(Style::default().bg(theme.effective_bg()))
        .border_style(Style::default().fg(theme.primary.to_color()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)])
        .split(inner);

    if rows.is_empty() {
        let empty = Paragraph::new(Line::from(Span::styled(
            "  No bookmarks. Use Ctrl+Space H in a session.",
            Style::default().fg(theme.text_muted.to_color()),
        )));
        frame.render_widget(empty, chunks[0]);
        let hints = Paragraph::new(Line::from(vec![
            Span::styled("  Esc", Style::default().fg(theme.warning.to_color())),
            Span::styled(" close", Style::default().fg(theme.text_muted.to_color())),
        ]));
        frame.render_widget(hints, chunks[1]);
        return;
    }

    let items: Vec<ListItem> = rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let is_selected = i == state.selected;
            let line = Line::from(vec![
                Span::styled(
                    if is_selected { "  > " } else { "    " },
                    Style::default().fg(theme.warning.to_color()),
                ),
                Span::styled(
                    row.clone(),
                    if is_selected {
                        Style::default()
                            .fg(theme.text.to_color())
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme.text.to_color())
                    },
                ),
            ]);

            if is_selected {
                ListItem::new(line).style(Style::default().bg(theme.effective_selection_bg()))
            } else {
                ListItem::new(line)
            }
        })
        .collect();

    let mut list_state = ListState::default().with_selected(Some(state.selected));
    frame.render_stateful_widget(List::new(items), chunks[0], &mut list_state);

    let hints = Paragraph::new(Line::from(vec![
        Span::styled(
            "  j/k or \u{2191}/\u{2193}",
            Style::default().fg(theme.warning.to_color()),
        ),
        Span::styled(
            " navigate  ",
            Style::default().fg(theme.text_muted.to_color()),
        ),
        Span::styled("Enter", Style::default().fg(theme.warning.to_color())),
        Span::styled(" jump  ", Style::default().fg(theme.text_muted.to_color())),
        Span::styled("d", Style::default().fg(theme.warning.to_color())),
        Span::styled(
            " remove  ",
            Style::default().fg(theme.text_muted.to_color()),
        ),
        Span::styled("1-9", Style::default().fg(theme.warning.to_color())),
        Span::styled(
            " quick jump  ",
            Style::default().fg(theme.text_muted.to_color()),
        ),
        Span::styled("Esc", Style::default().fg(theme.warning.to_color())),
        Span::styled(" close", Style::default().fg(theme.text_muted.to_color())),
    ]));
    frame.render_widget(hints, chunks[1]);
}

pub fn draw_claude_session_picker(
    frame: &mut Frame,
    state: &ClaudeSessionPickerState,
    theme: &Theme,
) {
    let area = centered_rect(60, 50, frame.area());
    crate::ui::draw_modal_overlay(frame, area, theme);

    let title = format!(" Claude Sessions ({}) ", state.sessions.len());
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .style(Style::default().bg(theme.effective_bg()))
        .border_style(Style::default().fg(theme.success.to_color()));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.sessions.is_empty() {
        let empty = Paragraph::new(Line::from(Span::styled(
            "  No sessions for this worktree.",
            Style::default().fg(theme.text_muted.to_color()),
        )));
        frame.render_widget(empty, inner);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)])
        .split(inner);
    let row_width = inner.width as usize;

    let items: Vec<ListItem> = state
        .sessions
        .iter()
        .enumerate()
        .map(|(i, session)| {
            session_picker_item(
                theme,
                theme.success.to_color(),
                row_width,
                i == state.selected,
                session.updated,
                &short_session_id(&session.id),
                &session.title,
                theme.info.to_color(),
            )
        })
        .collect();

    let list = List::new(items);
    let mut list_state = ListState::default().with_selected(Some(state.selected));
    frame.render_stateful_widget(list, chunks[0], &mut list_state);

    let hints = Paragraph::new(Line::from(vec![
        Span::styled(
            "  j/k or \u{2191}/\u{2193}",
            Style::default().fg(theme.warning.to_color()),
        ),
        Span::styled(
            " navigate  ",
            Style::default().fg(theme.text_muted.to_color()),
        ),
        Span::styled("Enter", Style::default().fg(theme.warning.to_color())),
        Span::styled(
            " select  ",
            Style::default().fg(theme.text_muted.to_color()),
        ),
        Span::styled("Esc", Style::default().fg(theme.warning.to_color())),
        Span::styled(" cancel", Style::default().fg(theme.text_muted.to_color())),
    ]));
    frame.render_widget(hints, chunks[1]);
}

pub fn draw_codex_session_picker(
    frame: &mut Frame,
    state: &CodexSessionPickerState,
    theme: &Theme,
) {
    let area = centered_rect(60, 50, frame.area());
    crate::ui::draw_modal_overlay(frame, area, theme);

    let title = format!(" Codex Sessions ({}) ", state.sessions.len());
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .style(Style::default().bg(theme.effective_bg()))
        .border_style(Style::default().fg(theme.session_icon_codex.to_color()));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.sessions.is_empty() {
        let empty = Paragraph::new(Line::from(Span::styled(
            "  No sessions for this worktree.",
            Style::default().fg(theme.text_muted.to_color()),
        )));
        frame.render_widget(empty, inner);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)])
        .split(inner);
    let row_width = inner.width as usize;

    let items: Vec<ListItem> = state
        .sessions
        .iter()
        .enumerate()
        .map(|(i, session)| {
            session_picker_item(
                theme,
                theme.session_icon_codex.to_color(),
                row_width,
                i == state.selected,
                session.updated,
                &short_session_id(&session.id),
                &session.title,
                theme.info.to_color(),
            )
        })
        .collect();

    let list = List::new(items);
    let mut list_state = ListState::default().with_selected(Some(state.selected));
    frame.render_stateful_widget(list, chunks[0], &mut list_state);

    let hints = Paragraph::new(Line::from(vec![
        Span::styled(
            "  j/k or \u{2191}/\u{2193}",
            Style::default().fg(theme.warning.to_color()),
        ),
        Span::styled(
            " navigate  ",
            Style::default().fg(theme.text_muted.to_color()),
        ),
        Span::styled("Enter", Style::default().fg(theme.warning.to_color())),
        Span::styled(
            " select  ",
            Style::default().fg(theme.text_muted.to_color()),
        ),
        Span::styled("Esc", Style::default().fg(theme.warning.to_color())),
        Span::styled(" cancel", Style::default().fg(theme.text_muted.to_color())),
    ]));
    frame.render_widget(hints, chunks[1]);
}

fn draw_session_restore_confirm(frame: &mut Frame, theme: &Theme, agent_name: &str) {
    let area = centered_rect(62, 35, frame.area());
    crate::ui::draw_modal_overlay(frame, area, theme);

    let text = Paragraph::new(vec![
        Line::from(""),
        Line::from(Span::styled(
            "  Feature is already running.",
            Style::default().fg(theme.warning.to_color()),
        )),
        Line::from(""),
        Line::from(Span::styled(
            format!("  Restart with selected {agent_name} session?"),
            Style::default().fg(theme.text.to_color()),
        )),
        Line::from(Span::styled(
            "  This will kill the current tmux session and start",
            Style::default().fg(theme.text_muted.to_color()),
        )),
        Line::from(Span::styled(
            "  a new one with the session restored.",
            Style::default().fg(theme.text_muted.to_color()),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("  y", Style::default().fg(theme.warning.to_color())),
            Span::styled(
                " restart  ",
                Style::default().fg(theme.text_muted.to_color()),
            ),
            Span::styled("n/Esc", Style::default().fg(theme.warning.to_color())),
            Span::styled(" cancel", Style::default().fg(theme.text_muted.to_color())),
        ]),
    ])
    .block(
        Block::default()
            .title(" Confirm Restart ")
            .borders(Borders::ALL)
            .style(Style::default().bg(theme.effective_bg()))
            .border_style(Style::default().fg(theme.warning.to_color())),
    )
    .wrap(Wrap { trim: false });

    frame.render_widget(text, area);
}

// One picker row's display inputs; kept flat to match the other *_item
// renderers in this file.
#[allow(clippy::too_many_arguments)]
fn session_picker_item(
    theme: &Theme,
    accent: Color,
    row_width: usize,
    is_selected: bool,
    updated: i64,
    identifier: &str,
    title: &str,
    timestamp_color: Color,
) -> ListItem<'static> {
    let marker = if is_selected { "  > " } else { "    " };
    let age = format_session_age(updated);
    let ident = fit_text(identifier, 12);
    let age_str = format!("{age:>6}");
    let ident_str = format!("  {ident:<12}  ");
    let meta_chars = age_str.chars().count() + ident_str.chars().count();
    let title_width = row_width.saturating_sub(marker.chars().count() + meta_chars);
    let title_style = if is_selected {
        Style::default()
            .fg(theme.text.to_color())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.text.to_color())
    };
    let continuation_prefix = format!(
        "{}{}",
        " ".repeat(marker.chars().count()),
        " ".repeat(meta_chars)
    );
    let continuation_width = row_width.saturating_sub(continuation_prefix.chars().count());

    let mut wrapped = wrap_text_to_width(title, title_width.max(1));
    if wrapped.len() > 2 {
        let overflow = wrapped.split_off(1);
        let joined = overflow.join(" ");
        let second_width = continuation_width.max(1);
        wrapped.push(fit_text(&joined, second_width));
    }
    if wrapped.len() == 2 {
        let second_width = continuation_width.max(1);
        wrapped[1] = fit_text(&wrapped[1], second_width);
    }

    let first_line = Line::from(vec![
        Span::styled(marker.to_string(), Style::default().fg(accent)),
        Span::styled(age_str, Style::default().fg(timestamp_color)),
        Span::styled(ident_str, Style::default().fg(theme.text_muted.to_color())),
        Span::styled(
            wrapped
                .first()
                .cloned()
                .unwrap_or_else(|| fit_text(title, title_width.max(1))),
            title_style,
        ),
    ]);

    let mut lines = vec![first_line];
    if let Some(second_line) = wrapped.get(1) {
        lines.push(Line::from(vec![
            Span::raw(continuation_prefix),
            Span::styled(second_line.clone(), title_style),
        ]));
    }

    if is_selected {
        ListItem::new(lines).style(Style::default().bg(theme.effective_selection_bg()))
    } else {
        ListItem::new(lines)
    }
}

fn format_session_age(updated: i64) -> String {
    normalize_timestamp(updated)
        .and_then(|updated| Local.timestamp_opt(updated, 0).single())
        .map(relative_age)
        .unwrap_or_else(|| "--".to_string())
}

fn normalize_timestamp(updated: i64) -> Option<i64> {
    if updated <= 0 {
        None
    } else if updated > 1_000_000_000_000 {
        Some(updated / 1000)
    } else {
        Some(updated)
    }
}

fn short_session_id(id: &str) -> String {
    let base = id
        .rsplit('-')
        .next()
        .filter(|part| part.len() >= 6)
        .unwrap_or(id);
    if base.chars().count() <= 10 {
        base.to_string()
    } else {
        base.chars().take(10).collect()
    }
}

fn wrap_text_to_width(text: &str, max_chars: usize) -> Vec<String> {
    if max_chars == 0 {
        return vec![String::new()];
    }

    let mut lines = Vec::new();
    let mut current = String::new();

    for word in text.split_whitespace() {
        let word_len = word.chars().count();

        if current.is_empty() {
            if word_len <= max_chars {
                current.push_str(word);
            } else {
                let mut chunk = String::new();
                for ch in word.chars() {
                    chunk.push(ch);
                    if chunk.chars().count() == max_chars {
                        lines.push(std::mem::take(&mut chunk));
                    }
                }
                current = chunk;
            }
            continue;
        }

        let projected_len = current.chars().count() + 1 + word_len;
        if projected_len <= max_chars {
            current.push(' ');
            current.push_str(word);
            continue;
        }

        lines.push(std::mem::take(&mut current));
        if word_len <= max_chars {
            current.push_str(word);
        } else {
            let mut chunk = String::new();
            for ch in word.chars() {
                chunk.push(ch);
                if chunk.chars().count() == max_chars {
                    lines.push(std::mem::take(&mut chunk));
                }
            }
            current = chunk;
        }
    }

    if !current.is_empty() {
        lines.push(current);
    }

    if lines.is_empty() {
        vec![String::new()]
    } else {
        lines
    }
}

pub fn draw_claude_session_confirm(frame: &mut Frame, theme: &Theme) {
    draw_session_restore_confirm(frame, theme, "claude");
}

pub fn draw_codex_session_confirm(frame: &mut Frame, theme: &Theme) {
    draw_session_restore_confirm(frame, theme, "codex");
}

pub fn draw_session_picker(
    frame: &mut Frame,
    state: &SessionPickerState,
    usage: &UsageData,
    nerd_font: bool,
    theme: &Theme,
) {
    let area = centered_rect(55, 60, frame.area());
    crate::ui::draw_modal_overlay(frame, area, theme);

    let total = state.builtin_sessions.len() + state.custom_sessions.len();
    let title = format!(" Start Session ({}) ", total);
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .style(Style::default().bg(theme.effective_bg()))
        .border_style(Style::default().fg(theme.primary.to_color()));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if total == 0 {
        let empty = Paragraph::new(Line::from(Span::styled(
            "  No sessions available.",
            Style::default().fg(theme.text_muted.to_color()),
        )));
        frame.render_widget(empty, inner);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)])
        .split(inner);

    let mut items: Vec<ListItem> = Vec::new();
    let mut selected_item_idx: Option<usize> = None;

    if !state.builtin_sessions.is_empty() {
        items.push(ListItem::new(Line::from(Span::styled(
            "  Built-in Sessions",
            Style::default()
                .fg(theme.primary.to_color())
                .add_modifier(Modifier::BOLD),
        ))));

        for (i, session) in state.builtin_sessions.iter().enumerate() {
            let idx = i;
            let is_selected = idx == state.selected;
            let is_disabled = session.disabled.is_some();

            let icon = match session.kind {
                crate::project::SessionKind::Claude => Span::styled(
                    "  * ",
                    Style::default().fg(theme.session_icon_claude.to_color()),
                ),
                crate::project::SessionKind::Codex => Span::styled(
                    "  * ",
                    Style::default().fg(theme.session_icon_codex.to_color()),
                ),
                crate::project::SessionKind::Terminal => Span::styled(
                    "  > ",
                    Style::default().fg(theme.session_icon_terminal.to_color()),
                ),
                crate::project::SessionKind::Nvim => {
                    let icon = if nerd_font { "  \u{e6ae} " } else { "  ~ " };
                    Span::styled(
                        icon,
                        Style::default().fg(theme.session_icon_nvim.to_color()),
                    )
                }
                crate::project::SessionKind::Vscode => Span::styled(
                    "  V ",
                    Style::default().fg(theme.session_icon_vscode.to_color()),
                ),
                crate::project::SessionKind::Todos => {
                    let icon = if nerd_font { "  \u{f0ae} " } else { "  = " };
                    Span::styled(
                        icon,
                        Style::default().fg(theme.session_icon_custom.to_color()),
                    )
                }
                _ => Span::styled("    ", Style::default().fg(theme.text_muted.to_color())),
            };

            let (label_style, msg) = if is_disabled {
                (
                    Style::default().fg(theme.text_muted.to_color()),
                    session.disabled.as_ref(),
                )
            } else if is_selected {
                (
                    Style::default()
                        .fg(theme.text.to_color())
                        .add_modifier(Modifier::BOLD),
                    None,
                )
            } else {
                (Style::default().fg(theme.text.to_color()), None)
            };

            let mut spans = vec![
                if is_selected && !is_disabled {
                    Span::styled("  > ", Style::default().fg(theme.warning.to_color()))
                } else {
                    Span::styled("    ", Style::default().fg(theme.text_muted.to_color()))
                },
                icon,
                Span::styled(&session.label, label_style),
            ];

            if let Some(reason) = msg {
                spans.push(Span::styled(
                    format!(" ({})", reason),
                    Style::default().fg(theme.danger.to_color()),
                ));
            }

            let line = Line::from(spans);

            let usage_summary =
                format_usage_summary(&usage_windows_for_session_kind(&session.kind, usage));
            let text: Text = match usage_summary {
                Some(summary) => {
                    // `text_muted` and the selection background are the
                    // same "darkgray" in the default theme, so the muted
                    // color would be invisible on a highlighted row.
                    let usage_style = if is_selected && !is_disabled {
                        Style::default().fg(theme.text.to_color())
                    } else {
                        Style::default().fg(theme.text_muted.to_color())
                    };
                    Text::from(vec![
                        line,
                        Line::from(Span::styled(format!("      {summary}"), usage_style)),
                    ])
                }
                None => Text::from(line),
            };

            if is_selected && !is_disabled {
                selected_item_idx = Some(items.len());
                items.push(
                    ListItem::new(text).style(Style::default().bg(theme.effective_selection_bg())),
                );
            } else {
                items.push(ListItem::new(text));
            }
        }
    }

    if !state.custom_sessions.is_empty() {
        if !items.is_empty() {
            items.push(ListItem::new(Line::from("")));
        }

        items.push(ListItem::new(Line::from(Span::styled(
            "  Custom Sessions",
            Style::default()
                .fg(theme.secondary.to_color())
                .add_modifier(Modifier::BOLD),
        ))));

        let builtin_len = state.builtin_sessions.len();
        for (i, cfg) in state.custom_sessions.iter().enumerate() {
            let idx = builtin_len + i;
            let is_selected = idx == state.selected;

            let name_style = if is_selected {
                Style::default()
                    .fg(theme.text.to_color())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text.to_color())
            };

            let raw_icon = if nerd_font {
                cfg.icon_nerd
                    .as_deref()
                    .map(resolve_custom_session_icon)
                    .or(cfg.icon.as_deref())
                    .unwrap_or("$")
            } else {
                cfg.icon.as_deref().unwrap_or("$")
            };
            let icon_str = format!("  {} ", raw_icon);

            let mut lines: Vec<Line> = vec![Line::from(vec![
                if is_selected {
                    Span::styled("  > ", Style::default().fg(theme.warning.to_color()))
                } else {
                    Span::styled("    ", Style::default().fg(theme.text_muted.to_color()))
                },
                Span::styled(icon_str, Style::default().fg(theme.secondary.to_color())),
                Span::styled(&cfg.name, name_style),
            ])];

            let subtitle = cfg.description.as_deref().or(cfg.command.as_deref());
            if let Some(text) = subtitle {
                let preview = if text.len() > 50 {
                    format!("{}...", &text[..47])
                } else {
                    text.to_string()
                };
                lines.push(Line::from(Span::styled(
                    format!("      {}", preview),
                    Style::default().fg(if is_selected {
                        theme.text.to_color()
                    } else {
                        theme.text_muted.to_color()
                    }),
                )));
            }

            let item = ListItem::new(lines);
            if is_selected {
                selected_item_idx = Some(items.len());
                items.push(item.style(Style::default().bg(theme.effective_selection_bg())));
            } else {
                items.push(item);
            }
        }
    }

    let list = List::new(items);
    let mut list_state = ListState::default();
    list_state.select(selected_item_idx);
    frame.render_stateful_widget(list, chunks[0], &mut list_state);

    let hints = Paragraph::new(Line::from(vec![
        Span::styled(
            "  j/k or \u{2191}/\u{2193}",
            Style::default().fg(theme.warning.to_color()),
        ),
        Span::styled(
            " navigate  ",
            Style::default().fg(theme.text_muted.to_color()),
        ),
        Span::styled("Enter", Style::default().fg(theme.warning.to_color())),
        Span::styled(" start  ", Style::default().fg(theme.text_muted.to_color())),
        Span::styled("Esc", Style::default().fg(theme.warning.to_color())),
        Span::styled(" cancel", Style::default().fg(theme.text_muted.to_color())),
    ]));
    frame.render_widget(hints, chunks[1]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    use std::path::PathBuf;

    fn pending_input(message: &str) -> PendingInput {
        PendingInput {
            session_id: "aiw-feat".to_string(),
            cwd: String::new(),
            message: message.to_string(),
            notification_type: "input-request".to_string(),
            file_path: PathBuf::new(),
            target_file_path: None,
            relative_path: None,
            change_id: None,
            tool: None,
            old_snippet: None,
            new_snippet: None,
            original_file: None,
            proposed_file: None,
            is_new_file: None,
            reason: None,
            response_file: None,
            project_name: Some("proj".to_string()),
            feature_name: Some("feat".to_string()),
            proceed_signal: None,
            request_id: None,
            reply_socket: None,
        }
    }

    /// Regression: the row used to slice the message at byte 47, which
    /// panics inside a multibyte character.
    #[test]
    fn a_long_multibyte_pending_message_is_cut_on_columns_not_bytes() {
        let message = "Привет, агенту нужно разрешение на запуск команды в терминале";
        assert!(message.len() > 50 && !message.is_char_boundary(47));
        let pending = vec![pending_input(message)];
        let rows = vec![AttentionRow::Pending(0)];
        let theme = Theme::default();
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        terminal
            .draw(|frame| {
                draw_notification_picker(frame, &rows, &pending, &[None], 0, false, &theme)
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let screen: String = (0..30)
            .map(|y| {
                (0..120)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(screen.contains("Привет, агенту"), "{screen}");
        assert!(screen.contains('…'), "{screen}");
    }
}
