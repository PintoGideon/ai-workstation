use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph, // FORK: G11 panel from `fork_chrome`
};

use crate::app::{
    App, AppMode, ConfigWizardStep, DASHBOARD_SESSION_FILTER_ENABLED, Selection, SessionFilter,
};
use crate::project::SessionKind;
use crate::theme::Theme;
use crate::usage::Model;

/// Accent for a usage percentage (used, not remaining). Shared with the
/// session sidebar's Usage bars so both surfaces change colour together.
pub(crate) fn utilization_color(pct: f64, theme: &Theme) -> Color {
    if pct >= 80.0 {
        theme.usage_high.to_color()
    } else if pct >= 50.0 {
        theme.usage_medium.to_color()
    } else {
        theme.usage_low.to_color()
    }
}

fn usage_bar_spans<'a>(label: &'a str, pct: f64, bar_width: usize, theme: &Theme) -> Vec<Span<'a>> {
    let color = utilization_color(pct, theme);
    let filled = ((pct / 100.0) * bar_width as f64).round() as usize;
    let empty = bar_width.saturating_sub(filled);

    vec![
        Span::styled(
            format!("{} ", label),
            Style::default().fg(theme.text_muted.to_color()),
        ),
        Span::styled("┃".repeat(filled), Style::default().fg(color)),
        Span::styled(
            "░".repeat(empty),
            Style::default().fg(theme.scrollbar.to_color()),
        ),
        Span::styled(
            format!(" {:.0}%", pct),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
    ]
}

// FORK: one copy (`app::util::shorten_path`, home looked up once).
use crate::app::util::shorten_path;
use crate::token_tracking::format_token_count;

pub fn draw(frame: &mut Frame, app: &App, area: Rect) {
    let theme = &app.theme;
    // FORK: G11 keys in the single accent colour (bold), labels dim.
    let key_style = || super::fork_chrome::key_style(theme);
    let hint_style = || Style::default().fg(theme.text_muted.to_color());

    let filter_spans =
        if DASHBOARD_SESSION_FILTER_ENABLED && app.session_filter != SessionFilter::All {
            vec![
                Span::styled(" [", hint_style()),
                Span::styled(
                    app.session_filter.display_name(),
                    Style::default().fg(theme.primary.to_color()),
                ),
                Span::styled("] ", hint_style()),
            ]
        } else {
            vec![]
        };

    let keybinds = match &app.mode {
        // FORK: workspace layout — hints for the focused half.
        AppMode::Normal if app.workspace_active() => super::fork_workspace::footer_hints(app),
        AppMode::Normal => {
            let on_session = matches!(app.selection, Selection::Session(_, _, _));
            let on_feature = matches!(app.selection, Selection::Feature(_, _));
            let pending_worktree_script = match app.selection {
                Selection::Feature(pi, fi) => app.feature_is_pending_worktree_script(pi, fi),
                _ => false,
            };
            if on_session {
                // Offer whichever of start/stop applies to this session —
                // neither for one with no tmux window (TODOs), which `c` and
                // `x` do nothing to.
                let session_running = match app.selection {
                    Selection::Session(pi, fi, si) => app.session_window_running(pi, fi, si),
                    _ => None,
                };
                let start_stop = session_running.map(|running| {
                    if running {
                        ("x", " stop  ")
                    } else {
                        ("c", " start  ")
                    }
                });
                let mut spans = filter_spans;
                // FORK: a row with a permission request waiting (P6).
                if app.pending_permission_for_selection().is_some() {
                    spans.extend(super::fork_permissions::row_hint_spans(theme));
                }
                spans.extend(vec![
                    Span::styled(" Enter", key_style()),
                    Span::raw(" view  "),
                    Span::styled("r", key_style()),
                    Span::raw(" rename  "),
                ]);
                if let Some((key, label)) = start_stop {
                    spans.extend([Span::styled(key, key_style()), Span::raw(label)]);
                }
                spans.extend(vec![
                    Span::styled("d", key_style()),
                    Span::raw(" delete  "),
                    Span::styled("s", key_style()),
                    Span::raw(" switch  "),
                    Span::styled("S", key_style()),
                    Span::raw(" resume  "),
                    Span::styled("u", key_style()),
                    Span::raw(" config  "),
                    Span::styled("V", key_style()),
                    Span::raw(" pending review  "),
                    Span::styled("q", key_style()),
                    Span::raw(" quit"),
                ]);
                Line::from(spans)
            } else if on_feature && pending_worktree_script {
                let mut spans = filter_spans;
                spans.extend(vec![
                    Span::styled("Enter", key_style()),
                    Span::raw(" expand  "),
                    Span::styled("d", key_style()),
                    Span::raw(" delete  "),
                    Span::styled("V", key_style()),
                    Span::raw(" pending review  "),
                    Span::styled("q", key_style()),
                    Span::raw(" quit  "),
                    Span::styled(
                        "worktree script still running",
                        Style::default().fg(theme.info.to_color()),
                    ),
                ]);
                Line::from(spans)
            } else if on_feature {
                let mut spans = filter_spans;
                spans.extend(vec![
                    Span::styled(" n", key_style()),
                    Span::raw(" feature  "),
                    Span::styled("Enter", key_style()),
                    Span::raw(" expand  "),
                    Span::styled("c", key_style()),
                    Span::raw(" start  "),
                    Span::styled("x", key_style()),
                    Span::raw(" stop  "),
                    Span::styled("y", key_style()),
                    Span::raw(" ready  "),
                    Span::styled("s", key_style()),
                    Span::raw(" switch  "),
                    Span::styled("S", key_style()),
                    Span::raw(" resume  "),
                    Span::styled("u", key_style()),
                    Span::raw(" config  "),
                    Span::styled("d", key_style()),
                    Span::raw(" delete  "),
                    Span::styled("V", key_style()),
                    Span::raw(" pending review  "),
                ]);
                if !app.active_extension.custom_sessions.is_empty() {
                    spans.push(Span::styled("p", key_style()));
                    spans.push(Span::raw(" sessions  "));
                }
                spans.extend(vec![Span::styled("q", key_style()), Span::raw(" quit")]);
                Line::from(spans)
            } else if app.store.projects.is_empty() {
                // FORK: G11 — only what applies before the first project.
                Line::from(vec![
                    Span::styled(" N", key_style()),
                    Span::raw(" project  "),
                    Span::styled("?", key_style()),
                    Span::raw(" help  "),
                    Span::styled("q", key_style()),
                    Span::raw(" quit"),
                ])
            } else {
                let mut spans = filter_spans;
                spans.extend(vec![
                    Span::styled(" n", key_style()),
                    Span::raw(" feature  "),
                    Span::styled("g", key_style()),
                    Span::raw(" issues  "),
                    Span::styled("N", key_style()),
                    Span::raw(" project  "),
                    Span::styled("Enter", key_style()),
                    Span::raw(" expand  "),
                    Span::styled("u", key_style()),
                    Span::raw(" preferred harness  "),
                    Span::styled("d", key_style()),
                    Span::raw(" delete  "),
                    Span::styled("R", key_style()),
                    Span::raw(" refresh  "),
                    Span::styled("q", key_style()),
                    Span::raw(" quit"),
                ]);
                Line::from(spans)
            }
        }
        AppMode::CreatingProject(_)
        | AppMode::CreatingFeature(_)
        | AppMode::CreatingBatchFeatures(_)
        | AppMode::RenamingSession(_)
        | AppMode::RenamingFeature(_)
        | AppMode::BrowsingPath(_) => Line::from(vec![
            Span::styled("Enter", key_style()),
            Span::raw(" confirm  "),
            Span::styled("Esc", key_style()),
            Span::raw(" cancel"),
        ]),
        AppMode::DeletingProject(_) | AppMode::DeletingFeature(_, _) => Line::from(vec![
            Span::styled("y", key_style()),
            Span::raw(" confirm  "),
            Span::styled("n/Esc", key_style()),
            Span::raw(" cancel"),
        ]),
        AppMode::ConfigWizard(state) => match state.step {
            ConfigWizardStep::CategoryPicker => Line::from(vec![
                Span::styled("j/k", key_style()),
                Span::raw(" navigate  "),
                Span::styled("Enter", key_style()),
                Span::raw(" select  "),
                Span::styled("Esc", key_style()),
                Span::raw(" cancel  "),
                Span::styled("Ctrl+Q", key_style()),
                Span::raw(" close"),
            ]),
            ConfigWizardStep::ScopePicker => Line::from(vec![
                Span::styled("j/k", key_style()),
                Span::raw(" navigate  "),
                Span::styled("Enter", key_style()),
                Span::raw(" select  "),
                Span::styled("Esc", key_style()),
                Span::raw(" back  "),
                Span::styled("Ctrl+Q", key_style()),
                Span::raw(" close"),
            ]),
            ConfigWizardStep::ItemList => Line::from(vec![
                Span::styled("j/k", key_style()),
                Span::raw(" navigate  "),
                Span::styled("Enter", key_style()),
                Span::raw(" select  "),
                Span::styled("Ctrl+S", key_style()),
                Span::raw(" save  "),
                Span::styled("Esc", key_style()),
                Span::raw(" back  "),
                Span::styled("Ctrl+Q", key_style()),
                Span::raw(" close"),
            ]),
            ConfigWizardStep::EditItem => {
                if state.input_mode {
                    Line::from(vec![
                        Span::styled("Type", key_style()),
                        Span::raw(" text  "),
                        Span::styled("Enter/Esc", key_style()),
                        Span::raw(" stop editing  "),
                        Span::styled("Tab", key_style()),
                        Span::raw(" next field  "),
                        Span::styled("Ctrl+Q", key_style()),
                        Span::raw(" close"),
                    ])
                } else {
                    Line::from(vec![
                        Span::styled("j/k", key_style()),
                        Span::raw(" move  "),
                        Span::styled("Tab", key_style()),
                        Span::raw(" next  "),
                        Span::styled("Enter", key_style()),
                        Span::raw(" edit/toggle/select  "),
                        Span::styled("Esc", key_style()),
                        Span::raw(" back  "),
                        Span::styled("Ctrl+Q", key_style()),
                        Span::raw(" close"),
                    ])
                }
            }
            ConfigWizardStep::ConfirmSave => Line::from(vec![
                Span::styled("j/k", key_style()),
                Span::raw(" scroll  "),
                Span::styled("Enter", key_style()),
                Span::raw(" save  "),
                Span::styled("Esc", key_style()),
                Span::raw(" back  "),
                Span::styled("q", key_style()),
                Span::raw(" cancel  "),
                Span::styled("Ctrl+Q", key_style()),
                Span::raw(" close"),
            ]),
        },
        AppMode::Todos(state) => match &state.launch {
            Some(crate::app::TodoLaunchStep::Choice { .. }) => Line::from(vec![
                Span::styled("j/k", key_style()),
                Span::raw(" choose  "),
                Span::styled("Enter", key_style()),
                Span::raw(" confirm  "),
                Span::styled("Esc", key_style()),
                Span::raw(" back to list"),
            ]),
            None => Line::from(vec![
                Span::styled("j/k", key_style()),
                Span::raw(" navigate  "),
                Span::styled("Esc/q", key_style()),
                Span::raw(" close"),
            ]),
        },
        AppMode::TodoQuickCapture(_) => Line::from(vec![
            Span::styled("Enter", key_style()),
            Span::raw(" add TODO  "),
            Span::styled("Esc", key_style()),
            Span::raw(" cancel"),
        ]),
        AppMode::TodoImplementChoice(_) => Line::from(vec![
            Span::styled("j/k", key_style()),
            Span::raw(" choose  "),
            Span::styled("Enter", key_style()),
            Span::raw(" confirm  "),
            Span::styled("Esc", key_style()),
            Span::raw(" cancel"),
        ]),
        AppMode::TodoSpawnTarget(_) => Line::from(vec![
            Span::styled("j/k", key_style()),
            Span::raw(" choose feature  "),
            Span::styled("Enter", key_style()),
            Span::raw(" start  "),
            Span::styled("Esc", key_style()),
            Span::raw(" cancel"),
        ]),
        AppMode::TodoDeleteDisposition(_) => Line::from(vec![
            Span::styled("j/k", key_style()),
            Span::raw(" choose  "),
            Span::styled("Enter", key_style()),
            Span::raw(" confirm  "),
            Span::styled("Esc", key_style()),
            Span::raw(" cancel the deletion"),
        ]),
        AppMode::TodosHostReassign(_) => Line::from(vec![
            Span::styled("j/k", key_style()),
            Span::raw(" choose  "),
            Span::styled("Enter", key_style()),
            Span::raw(" confirm  "),
            Span::styled("Esc", key_style()),
            Span::raw(" keep list"),
        ]),
        AppMode::Help(_) => Line::from(vec![
            Span::styled("Esc/q/?", key_style()),
            Span::raw(" close help"),
        ]),
        AppMode::CommandPicker(_) => Line::from(vec![
            Span::styled("j/k or \u{2191}/\u{2193}", key_style()),
            Span::raw(" navigate  "),
            Span::styled("D", key_style()),
            Span::raw(" local  "),
            Span::styled("Enter", key_style()),
            Span::raw(" execute  "),
            Span::styled("Esc", key_style()),
            Span::raw(" cancel"),
        ]),
        AppMode::DiffPicker(_) => Line::from(vec![
            Span::styled("j/k or \u{2191}/\u{2193}", key_style()),
            Span::raw(" choose scope  "),
            Span::styled("Enter", key_style()),
            Span::raw(" view  "),
            Span::styled("Esc", key_style()),
            Span::raw(" cancel"),
        ]),
        AppMode::Searching(_) => Line::from(vec![
            Span::styled("↑/↓ or Tab/Shift+Tab", key_style()),
            Span::raw(" navigate  "),
            Span::styled("Enter", key_style()),
            Span::raw(" jump  "),
            Span::styled("Esc", key_style()),
            Span::raw(" cancel"),
        ]),
        AppMode::NotificationPicker(_, _)
        | AppMode::ClaudeSessionPicker(_)
        | AppMode::CodexSessionPicker(_)
        | AppMode::StoppedSessionDialog(_)
        | AppMode::SessionPicker(_)
        | AppMode::BookmarkPicker(_) => Line::from(vec![
            Span::styled("j/k or \u{2191}/\u{2193}", key_style()),
            Span::raw(" navigate  "),
            Span::styled("Enter", key_style()),
            Span::raw(" select  "),
            Span::styled("Esc", key_style()),
            Span::raw(" cancel"),
        ]),
        AppMode::NamingNewSession(_) => Line::from(vec![
            Span::styled("Enter", key_style()),
            Span::raw(" create  "),
            Span::styled("Esc", key_style()),
            Span::raw(" back"),
        ]),
        AppMode::ConfirmingClaudeSession { .. } | AppMode::ConfirmingCodexSession { .. } => {
            Line::from(vec![
                Span::styled("y", key_style()),
                Span::raw(" restart  "),
                Span::styled("n/Esc", key_style()),
                Span::raw(" cancel"),
            ])
        }
        AppMode::DiffReviewPrompt(state) => {
            if state.hold_active() {
                let secs = state.hold_remaining_secs().ceil() as u64;
                Line::from(vec![
                    Span::styled(
                        format!("Hold {secs}s"),
                        Style::default().fg(theme.warning.to_color()),
                    ),
                    Span::styled(
                        " — keys locked briefly",
                        Style::default().fg(theme.text_muted.to_color()),
                    ),
                ])
            } else if state.editing_feedback {
                Line::from(vec![
                    Span::styled("Enter", key_style()),
                    Span::raw(" submit reject  "),
                    Span::styled("Esc", key_style()),
                    Span::raw(" back  "),
                    Span::styled("Ctrl+U", key_style()),
                    Span::raw(" clear"),
                ])
            } else {
                Line::from(vec![
                    Span::styled("Enter", key_style()),
                    Span::raw(" approve  "),
                    Span::styled("e", Style::default().fg(theme.info.to_color())),
                    Span::raw(" explain  "),
                    Span::styled("v", key_style()),
                    Span::raw(" layout  "),
                    Span::styled("r", Style::default().fg(theme.danger.to_color())),
                    Span::raw(" feedback  "),
                    Span::styled("Esc", key_style()),
                    Span::raw(" cancel"),
                ])
            }
        }
        AppMode::RunningHook(state) => {
            if state.child.is_some() {
                Line::from(Span::styled(
                    "Running hook...",
                    Style::default().fg(theme.info.to_color()),
                ))
            } else {
                Line::from(vec![
                    Span::styled("Enter", key_style()),
                    Span::raw(" continue  "),
                    Span::styled("Esc", key_style()),
                    Span::raw(" skip"),
                ])
            }
        }
        AppMode::DeletingFeatureInProgress(state) => {
            if state.child.is_some() {
                Line::from(Span::styled(
                    "Deleting feature...",
                    Style::default().fg(theme.warning.to_color()),
                ))
            } else if state.error.is_some() {
                Line::from(vec![
                    Span::styled("Enter", key_style()),
                    Span::raw(" acknowledge"),
                ])
            } else {
                Line::from(Span::styled("Press any key to continue...", hint_style()))
            }
        }
        AppMode::HookPrompt(_) => Line::from(vec![
            Span::styled(" j/k", key_style()),
            Span::raw(" move  "),
            Span::styled("Enter", key_style()),
            Span::raw(" confirm  "),
            Span::styled("Esc", key_style()),
            Span::raw(" cancel"),
        ]),
        // FORK: permission detail overlay (P6).
        AppMode::PermissionDetail(_) => super::fork_permissions::detail_hints(theme),
        // FORK: the follow-up queue dialog (P7).
        AppMode::FollowupQueue(state) => {
            super::dialogs::fork_followups::dialog_hints(theme, state.focus)
        }
        // FORK: repo hook approval dialog.
        AppMode::ApprovingRepoHook(_) => Line::from(vec![
            Span::styled(" y", key_style()),
            Span::raw(" approve & run  "),
            Span::styled("s", key_style()),
            Span::raw(" skip this time  "),
            Span::styled("Esc", key_style()),
            Span::raw(" cancel"),
        ]),
        AppMode::LatestPrompt(state) => {
            let mut spans = Vec::new();
            if !state.prompts.is_empty() {
                spans.extend(vec![
                    Span::styled(" Tab", key_style()),
                    Span::raw("/"),
                    Span::styled("Enter", key_style()),
                    Span::raw(" inject  "),
                    Span::styled("y", key_style()),
                    Span::raw(" copy  "),
                    Span::styled("j/k", key_style()),
                    Span::raw(" nav  "),
                ]);
            }
            spans.extend(vec![
                Span::styled(" Esc", key_style()),
                Span::styled("/q", key_style()),
                Span::raw(" close"),
            ]);
            Line::from(spans)
        }
        AppMode::PromptLibrary(_) => Line::from(vec![
            Span::styled(" Enter", key_style()),
            Span::raw(" inject  "),
            Span::styled("n", key_style()),
            Span::raw(" new  "),
            Span::styled("e", key_style()),
            Span::raw(" edit  "),
            Span::styled("d", key_style()),
            Span::raw(" delete  "),
            Span::styled("/", key_style()),
            Span::raw(" search  "),
            Span::styled("Esc", key_style()),
            Span::raw(" close"),
        ]),
        AppMode::PromptEditor(_) => Line::from(vec![
            Span::styled(" Tab", key_style()),
            Span::raw(" switch field  "),
            Span::styled("Ctrl+S", key_style()),
            Span::raw(" save  "),
            Span::styled("Ctrl+K", key_style()),
            Span::raw(" skill  "),
            Span::styled("Esc", key_style()),
            Span::raw(" cancel"),
        ]),
        AppMode::PlaceholderFill(_) => Line::from(vec![
            Span::styled(" Tab", key_style()),
            Span::raw(" next  "),
            Span::styled("Shift+Tab", key_style()),
            Span::raw(" prev  "),
            Span::styled("Ctrl+S", key_style()),
            Span::raw(" inject  "),
            Span::styled("Ctrl+K", key_style()),
            Span::raw(" skill  "),
            Span::styled("Esc", key_style()),
            Span::raw(" cancel"),
        ]),
        AppMode::SkillPicker(_) => Line::from(vec![
            Span::styled(" type", key_style()),
            Span::raw(" filter  "),
            Span::styled("\u{2191}/\u{2193}", key_style()),
            Span::raw(" navigate  "),
            Span::styled("Enter", key_style()),
            Span::raw(" insert  "),
            Span::styled("Esc", key_style()),
            Span::raw(" cancel"),
        ]),
        AppMode::DiffViewer(_) => {
            let mut spans = vec![Span::styled(" Tab", key_style()), Span::raw(" focus  ")];
            if app.diff_viewer_selected_file_is_new() {
                spans.push(Span::styled(
                    " new file uses unified view  ",
                    Style::default().fg(theme.info.to_color()),
                ));
            } else {
                spans.push(Span::styled("v", key_style()));
                spans.push(Span::raw(" layout  "));
            }
            spans.extend(vec![
                Span::styled("j/k", key_style()),
                Span::raw(" move  "),
                Span::styled("PgUp/PgDn", key_style()),
                Span::raw(" scroll  "),
                Span::styled("r", key_style()),
                Span::raw(" refresh  "),
                Span::styled("Esc", key_style()),
                Span::raw(" close"),
            ]);
            Line::from(spans)
        }
        AppMode::DiffViewerLoading(_) => Line::from(vec![
            Span::raw(" Loading branch diff...  "),
            Span::styled("Esc", key_style()),
            Span::raw(" close"),
        ]),
        AppMode::MarkdownLoading(_) => Line::from(vec![
            Span::raw(" Loading markdown...  "),
            Span::styled("Esc", key_style()),
            Span::raw(" close"),
        ]),
        AppMode::MarkdownViewer(_) => Line::from(vec![
            Span::styled(" j/k", key_style()),
            Span::raw(" scroll  "),
            Span::styled("PgUp/PgDn", key_style()),
            Span::raw(" page  "),
            Span::styled("/", key_style()),
            Span::raw(" files  "),
            Span::styled("r", key_style()),
            Span::raw(" refresh  "),
            Span::styled("Esc", key_style()),
            Span::raw(" close"),
        ]),
        AppMode::MarkdownFilePicker(_) => {
            let mut spans = vec![
                Span::styled(" j/k", key_style()),
                Span::raw(" navigate  "),
                Span::styled("/", key_style()),
                Span::raw(" search  "),
            ];
            spans.push(Span::styled("p", key_style()));
            spans.push(Span::raw(" plan  "));
            spans.push(Span::styled("Enter", key_style()));
            spans.push(Span::raw(" open  "));
            spans.push(Span::styled("Esc", key_style()));
            spans.push(Span::raw(" cancel"));
            Line::from(spans)
        }
        AppMode::ForkingFeature(_) => Line::from(vec![
            Span::styled(" Enter", key_style()),
            Span::raw(" confirm  "),
            Span::styled("Esc", key_style()),
            Span::raw(" cancel"),
        ]),
        // FORK: §19.2
        AppMode::ForkSession(_) => Line::from(vec![
            Span::styled(" j/k", key_style()),
            Span::raw(" choose  "),
            Span::styled("Enter", key_style()),
            Span::raw(" fork  "),
            Span::styled("Esc", key_style()),
            Span::raw(" cancel"),
        ]),
        AppMode::ExternalSessions(_) => Line::from(vec![
            Span::styled(" j/k", key_style()),
            Span::raw(" move  "),
            Span::styled("Enter", key_style()),
            Span::raw(" copy attach command  "),
            Span::styled("Esc", key_style()),
            Span::raw(" close"),
        ]),
        // FORK: §19.7
        AppMode::BackgroundPrompt(_) => Line::from(vec![
            Span::styled(" Enter", key_style()),
            Span::raw(" start  "),
            Span::styled("Esc", key_style()),
            Span::raw(" cancel"),
        ]),
        AppMode::BackgroundRemove(_) => Line::from(vec![
            Span::styled(" y", key_style()),
            Span::raw(" remove  "),
            Span::styled("n", key_style()),
            Span::raw(" keep"),
        ]),
        // FORK: §19.8
        AppMode::Watchdog(_) => Line::from(vec![
            Span::styled(" j/k", key_style()),
            Span::raw(" move  "),
            Span::styled("Enter", key_style()),
            Span::raw(" run  "),
            Span::styled("1-9", key_style()),
            Span::raw(" run that one  "),
            Span::styled("Esc", key_style()),
            Span::raw(" close"),
        ]),
        // FORK: §19.6
        AppMode::ClaudeWorktrees(state) if state.confirm_remove => Line::from(vec![
            Span::styled(" y", key_style()),
            Span::raw(" remove  "),
            Span::styled("any key", key_style()),
            Span::raw(" keep"),
        ]),
        AppMode::ClaudeWorktrees(_) => Line::from(vec![
            Span::styled(" j/k", key_style()),
            Span::raw(" move  "),
            Span::styled("t", key_style()),
            Span::raw(" terminal  "),
            Span::styled("y", key_style()),
            Span::raw(" copy path  "),
            Span::styled("d", key_style()),
            Span::raw(" remove  "),
            Span::styled("Esc", key_style()),
            Span::raw(" close"),
        ]),
        // FORK: §19.4
        AppMode::FeatureGroups(state) if state.edit.is_some() => Line::from(vec![
            Span::styled(" Enter", key_style()),
            Span::raw(" save  "),
            Span::styled("Esc", key_style()),
            Span::raw(" back"),
        ]),
        AppMode::FeatureGroups(_) => Line::from(vec![
            Span::styled(" Enter", key_style()),
            Span::raw(" move here  "),
            Span::styled("n", key_style()),
            Span::raw(" new  "),
            Span::styled("r", key_style()),
            Span::raw(" rename  "),
            Span::styled("c", key_style()),
            Span::raw(" colour  "),
            Span::styled("d", key_style()),
            Span::raw(" delete  "),
            Span::styled("Esc", key_style()),
            Span::raw(" close"),
        ]),
        AppMode::MessageSession(_) => Line::from(vec![
            Span::styled(" Enter", key_style()),
            Span::raw(" send  "),
            Span::styled("↑/↓", key_style()),
            Span::raw(" target  "),
            Span::styled("Esc", key_style()),
            Span::raw(" cancel"),
        ]),
        AppMode::NoteEditor(_) => Line::from(vec![
            Span::styled(" Enter", key_style()),
            Span::raw(" save  "),
            Span::styled("Alt+Enter", key_style()),
            Span::raw(" newline  "),
            Span::styled("Esc", key_style()),
            Span::raw(" cancel"),
        ]),
        AppMode::ThemePicker(_) => Line::from(vec![
            Span::styled(" j/k", key_style()),
            Span::raw(" navigate  "),
            Span::styled("Enter", key_style()),
            Span::raw(" apply  "),
            Span::styled("Esc", key_style()),
            Span::raw(" cancel"),
        ]),
        AppMode::SyntaxLanguagePicker(_) => Line::from(vec![
            Span::styled(" j/k", key_style()),
            Span::raw(" navigate  "),
            Span::styled("Enter/i", key_style()),
            Span::raw(" download + install  "),
            Span::styled("x", key_style()),
            Span::raw(" remove  "),
            Span::styled("r", key_style()),
            Span::raw(" refresh  "),
            Span::styled("Esc", key_style()),
            Span::raw(" cancel"),
        ]),
        AppMode::SessionConfig(_) => Line::from(vec![
            Span::styled(" j/k", key_style()),
            Span::raw(" navigate  "),
            Span::styled("Enter", key_style()),
            Span::raw(" apply  "),
            Span::styled("Esc", key_style()),
            Span::raw(" cancel"),
        ]),
        AppMode::ProjectAgentConfig(_) => Line::from(vec![
            Span::styled(" j/k", key_style()),
            Span::raw(" navigate  "),
            Span::styled("Enter", key_style()),
            Span::raw(" apply  "),
            Span::styled("Esc", key_style()),
            Span::raw(" cancel"),
        ]),
        AppMode::DebugLog(_) => Line::from(vec![
            Span::styled(" j/k", key_style()),
            Span::raw(" scroll  "),
            Span::styled("c", key_style()),
            Span::raw(" clear  "),
            Span::styled("/", key_style()),
            Span::raw(" actions  "),
            Span::styled("Esc", key_style()),
            Span::raw(" close"),
        ]),
        AppMode::HarnessSetup(_) => Line::from(vec![
            Span::styled(" j/k", key_style()),
            Span::raw(" navigate  "),
            Span::styled("Enter", key_style()),
            Span::raw(" toggle  "),
            Span::styled("c", key_style()),
            Span::raw(" confirm  "),
            Span::styled("Esc", key_style()),
            Span::raw(" close"),
        ]),
        AppMode::Dormant(_) => Line::from(vec![
            Span::styled(" Enter", key_style()),
            Span::raw(" open  "),
            Span::styled("x", key_style()),
            Span::raw(" stop  "),
            Span::styled("e", key_style()),
            Span::raw(" close editor  "),
            Span::styled("d", key_style()),
            Span::raw(" delete  "),
            Span::styled("q", key_style()),
            Span::raw(" close"),
        ]),
        AppMode::ConfirmResourceStart(_) => Line::from(vec![
            Span::styled(" y", key_style()),
            Span::raw(" start anyway  "),
            Span::styled("n/Esc", key_style()),
            Span::raw(" cancel"),
        ]),
        AppMode::ConfirmTodoReferenceCompletion(_) => Line::from(vec![
            Span::styled(" y/Enter", key_style()),
            Span::raw(" complete TODO  "),
            Span::styled("n/Esc", key_style()),
            Span::raw(" cancel"),
        ]),
        AppMode::ContextSettings(_) => Line::from(vec![
            Span::styled(" Tab", key_style()),
            Span::raw(" next field  "),
            Span::styled("Enter", key_style()),
            Span::raw(" save  "),
            Span::styled("Esc", key_style()),
            Span::raw(" cancel"),
        ]),
        AppMode::IssueBrowser(_) => Line::from(vec![
            Span::styled(" h/l", key_style()),
            Span::raw(" page  "),
            Span::styled("r", key_style()),
            Span::raw(" refresh  "),
            Span::styled("Esc", key_style()),
            Span::raw(" close"),
        ]),
        AppMode::IssueSetup(_) => Line::from(vec![
            Span::styled(" j/k", key_style()),
            Span::raw(" move  "),
            Span::styled("h/l", key_style()),
            Span::raw(" change  "),
            Span::styled("Enter", key_style()),
            Span::raw(" create  "),
            Span::styled("Esc", key_style()),
            Span::raw(" cancel"),
        ]),
        AppMode::IssueDuplicateWarning(_) => Line::from(vec![
            Span::styled("Enter/y", key_style()),
            Span::raw(" create another  "),
            Span::styled("Esc/n", key_style()),
            Span::raw(" return"),
        ]),
    };

    let message_line = if let Some(ref msg) = app.message {
        let color = if msg.starts_with("Error:") {
            theme.danger.to_color()
        } else {
            theme.success.to_color()
        };
        Line::from(Span::styled(msg.as_str(), Style::default().fg(color)))
    } else {
        match &app.selection {
            Selection::Project(pi) if *pi < app.store.projects.len() => {
                let project = &app.store.projects[*pi];
                Line::from(vec![
                    Span::styled(
                        project.name.clone(),
                        Style::default()
                            .fg(theme.project_title.to_color())
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("  preferred: {}", project.preferred_agent.display_name()),
                        Style::default().fg(theme.primary.to_color()),
                    ),
                    Span::styled(format!("  {}", shorten_path(&project.repo)), hint_style()),
                ])
            }
            Selection::Feature(pi, fi)
                if *pi < app.store.projects.len()
                    && *fi < app.store.projects[*pi].features.len() =>
            {
                let feature = &app.store.projects[*pi].features[*fi];
                let branch_info = if feature.branch.is_empty() {
                    String::new()
                } else {
                    format!(" [{}]", feature.branch)
                };
                Line::from(vec![
                    Span::styled(
                        feature.name.clone(),
                        Style::default()
                            .fg(theme.feature_title.to_color())
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(branch_info, Style::default().fg(theme.warning.to_color())),
                    Span::styled(
                        format!("  {}", shorten_path(&feature.workdir)),
                        hint_style(),
                    ),
                ])
            }
            Selection::Session(pi, fi, si)
                if *pi < app.store.projects.len()
                    && *fi < app.store.projects[*pi].features.len()
                    && *si < app.store.projects[*pi].features[*fi].sessions.len() =>
            {
                let feature = &app.store.projects[*pi].features[*fi];
                let session = &feature.sessions[*si];
                let kind_label = match session.kind {
                    SessionKind::Claude => "claude",
                    SessionKind::Codex => "codex",
                    SessionKind::Pi => "pi",
                    SessionKind::Gemini => "gemini", // FORK
                    SessionKind::Cursor => "cursor", // FORK
                    SessionKind::Terminal => "terminal",
                    SessionKind::Nvim => "nvim",
                    SessionKind::Vscode => "vscode",
                    SessionKind::Custom => "custom",
                    SessionKind::Todos => "todos",
                };
                Line::from(vec![
                    Span::styled(
                        format!("{} ({})", session.label, kind_label),
                        Style::default()
                            .fg(theme.text.to_color())
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!("  {}", feature.name), hint_style()),
                    Span::styled(
                        format!("  {}", shorten_path(&feature.workdir)),
                        hint_style(),
                    ),
                ])
            }
            _ => {
                let project_count = app.store.projects.len();
                let feature_count: usize =
                    app.store.projects.iter().map(|p| p.features.len()).sum();
                Line::from(Span::styled(
                    format!(
                        "{} project{}, {} feature{}",
                        project_count,
                        if project_count == 1 { "" } else { "s" },
                        feature_count,
                        if feature_count == 1 { "" } else { "s" },
                    ),
                    hint_style(),
                ))
            }
        }
    };

    // FORK: G11 rounded panel with the contextual key hints in its bottom
    // border. The panel is three rows tall, so its single inner row only ever
    // showed the message line and the hints below it were clipped away. The
    // inside carries one column of padding like the list; hints that do not
    // fit are dropped whole (never cut mid-word) and end in `…`.
    let mut keybinds = keybinds.style(hint_style());
    if !keybinds.spans.is_empty() {
        if !keybinds.spans[0].content.starts_with(' ') {
            keybinds.spans.insert(0, Span::raw(" "));
        }
        keybinds.spans.push(Span::raw(" "));
    }
    let key = key_style();
    let keybinds = super::fork_text::fit_hints(
        keybinds,
        usize::from(area.width.saturating_sub(2)),
        |span| span.style == key,
    );
    let block = super::fork_chrome::plain_panel(theme)
        .padding(ratatui::widgets::Padding::horizontal(1))
        .title_bottom(keybinds);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    // Feedback needs the full line; usage otherwise paints over its right edge.
    if app.message.is_some() {
        let message_line = super::fork_text::fit_line(message_line, usize::from(inner.width));
        frame.render_widget(Paragraph::new(message_line), inner);
        return;
    }

    let usage = app.usage.get_data();
    let mut right_spans: Vec<Span> = Vec::new();

    let model_label = Span::styled(
        format!("[{}] ", usage.visible_model.label()),
        Style::default()
            .fg(theme.secondary.to_color())
            .add_modifier(Modifier::BOLD),
    );
    right_spans.push(model_label);

    match usage.visible_model {
        Model::Claude => {
            if let Some(pct5) = usage.claude.five_hour_pct {
                right_spans.extend(usage_bar_spans("5h", pct5, 15, theme));
                right_spans.push(Span::raw(" "));
            }

            if let Some(pct7) = usage.claude.seven_day_pct {
                right_spans.extend(usage_bar_spans("7d", pct7, 15, theme));
                right_spans.push(Span::raw(" "));
            } else if let Some(ref err) = usage.claude.last_error {
                right_spans.push(Span::styled(
                    format!("{} ", err),
                    Style::default().fg(theme.danger.to_color()),
                ));
                right_spans.push(Span::raw(" "));
            }

            right_spans.push(Span::styled(
                format!("{} msgs ", usage.claude.today_messages),
                hint_style(),
            ));

            if usage.claude.today_tokens > 0 {
                let tok = usage.claude.today_tokens;
                let tok_str = if tok >= 1_000_000 {
                    format!("{:.1}M tok ", tok as f64 / 1_000_000.0)
                } else if tok >= 1_000 {
                    format!("{:.1}K tok ", tok as f64 / 1_000.0)
                } else {
                    format!("{} tok ", tok)
                };
                right_spans.push(Span::styled(
                    tok_str,
                    Style::default().fg(theme.info.to_color()),
                ));
            }
        }
        Model::Codex => {
            if let Some(pct5) = usage.codex.five_hour_usage_pct {
                right_spans.extend(usage_bar_spans("5h", pct5, 15, theme));
                right_spans.push(Span::raw(" "));
            }

            if let Some(pct7) = usage.codex.weekly_usage_pct {
                right_spans.extend(usage_bar_spans("7d", pct7, 15, theme));
                right_spans.push(Span::raw(" "));
            } else if usage.codex.five_hour_tokens > 0 {
                right_spans.push(Span::styled(
                    format!("5h {} ", format_token_count(usage.codex.five_hour_tokens)),
                    Style::default().fg(theme.warning.to_color()),
                ));
            }

            if usage.codex.today_tokens > 0 {
                right_spans.push(Span::styled(
                    format!("{} tok ", format_token_count(usage.codex.today_tokens)),
                    Style::default().fg(theme.info.to_color()),
                ));
            }

            right_spans.push(Span::styled(
                format!("{} calls ", usage.codex.today_calls),
                hint_style(),
            ));
        }
        Model::Zai => {
            if let Some(pct) = usage.zai.five_hour_usage_pct {
                right_spans.extend(usage_bar_spans("5h", pct, 15, theme));
                right_spans.push(Span::raw(" "));
            }

            if let Some(pct) = usage.zai.weekly_usage_pct {
                right_spans.extend(usage_bar_spans("7d", pct, 15, theme));
                right_spans.push(Span::raw(" "));
            } else if usage.zai.today_tokens > 0 {
                right_spans.push(Span::styled(
                    format!("{} ", format_token_count(usage.zai.today_tokens)),
                    Style::default().fg(theme.info.to_color()),
                ));
            }

            right_spans.push(Span::styled(
                format!("{} calls ", usage.zai.today_calls),
                hint_style(),
            ));
        }
    }

    let right_width: u16 = right_spans
        .iter()
        .map(|s| unicode_width::UnicodeWidthStr::width(s.content.as_ref()) as u16)
        .sum();
    let max_right_width = inner.width.saturating_sub(1);
    let right_width = right_width.min(max_right_width);
    // The selection line stops short of the usage instead of running under it.
    let left_width = inner
        .width
        .saturating_sub(right_width + u16::from(right_width > 0));
    let message_line = super::fork_text::fit_line(message_line, usize::from(left_width));
    frame.render_widget(
        Paragraph::new(message_line),
        Rect {
            width: left_width,
            ..inner
        },
    );
    if right_width == 0 {
        return;
    }
    let right_area = Rect {
        x: inner
            .x
            .saturating_add(inner.width.saturating_sub(right_width)),
        y: inner.y,
        width: right_width,
        height: 1,
    };
    let right = Paragraph::new(Line::from(right_spans));
    frame.render_widget(right, right_area);
}
