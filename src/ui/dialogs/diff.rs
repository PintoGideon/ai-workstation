use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use std::path::Path;
use unicode_width::UnicodeWidthStr;

use crate::{
    app::{DiffPickerState, DiffScope, DiffViewerFocus, DiffViewerLayout, DiffViewerState},
    diff::{DiffFile, DiffFileStatus, DiffHunk, DiffLine, DiffLineKind},
    highlight,
    theme::Theme,
};

use super::super::dashboard::centered_rect;

#[derive(Debug, Clone, PartialEq, Eq)]
struct StyledChunk {
    text: String,
    style: Style,
}

struct FileHighlights {
    old: Option<highlight::HighlightedText>,
    new: Option<highlight::HighlightedText>,
}

pub fn draw_diff_picker(frame: &mut Frame, state: &DiffPickerState, theme: &Theme) {
    let area = centered_rect(72, 70, frame.area());
    crate::ui::draw_modal_overlay(frame, area, theme);

    let block = Block::default()
        .title(" Choose Diff Scope ")
        .borders(Borders::ALL)
        .style(Style::default().bg(theme.effective_bg()))
        .border_style(Style::default().fg(theme.primary.to_color()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(3),
            Constraint::Length(2),
            Constraint::Length(1),
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(vec![
            Line::from(" View the whole branch and worktree, or isolate one commit."),
            Line::from(Span::styled(
                format!(" {} commit(s) on this feature branch", state.commits.len()),
                Style::default().fg(theme.text_muted.to_color()),
            )),
        ]),
        rows[0],
    );

    let mut items = vec![ListItem::new(Line::from(vec![
        Span::styled(
            "All current changes",
            Style::default()
                .fg(theme.text.to_color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "  commits + staged + unstaged + untracked",
            Style::default().fg(theme.text_muted.to_color()),
        ),
    ]))];
    items.extend(state.commits.iter().map(|commit| {
        ListItem::new(Line::from(vec![
            Span::styled(
                format!("{}  ", commit.short_hash),
                Style::default()
                    .fg(theme.primary.to_color())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                commit.subject.clone(),
                Style::default().fg(theme.text.to_color()),
            ),
        ]))
    }));

    let list = List::new(items)
        .highlight_style(
            Style::default()
                .bg(theme.effective_selection_bg())
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("> ");
    let mut list_state = ListState::default();
    list_state.select(Some(state.selected.min(state.commits.len())));
    frame.render_stateful_widget(list, rows[1], &mut list_state);

    if let Some(error) = &state.error {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!(" Could not list feature commits: {error}"),
                Style::default().fg(theme.danger.to_color()),
            )))
            .wrap(Wrap { trim: false }),
            rows[2],
        );
    }

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" j/k", Style::default().fg(theme.warning.to_color())),
            Span::raw(" move  "),
            Span::styled("Enter", Style::default().fg(theme.warning.to_color())),
            Span::raw(" view  "),
            Span::styled("Esc", Style::default().fg(theme.warning.to_color())),
            Span::raw(" cancel"),
        ])),
        rows[3],
    );
}

pub fn draw_diff_viewer(frame: &mut Frame, state: &mut DiffViewerState, theme: &Theme) {
    let area = centered_rect(96, 90, frame.area());
    crate::ui::draw_modal_overlay(frame, area, theme);

    let border_color = if state.error.is_some() {
        theme.danger.to_color()
    } else {
        theme.primary.to_color()
    };

    let block = Block::default()
        .title(if matches!(&state.scope, DiffScope::Commit(_)) {
            " Commit Diff "
        } else {
            " Current Changes "
        })
        .borders(Borders::ALL)
        .style(Style::default().bg(theme.effective_bg()))
        .border_style(Style::default().fg(border_color));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(inner);

    draw_header(frame, chunks[0], state, theme);
    draw_body(frame, chunks[1], state, theme);
    draw_footer(frame, chunks[2], state, theme);

    if state.editing_base_ref {
        draw_base_ref_prompt(frame, state, theme);
    }
}

fn draw_base_ref_prompt(frame: &mut Frame, state: &DiffViewerState, theme: &Theme) {
    let area = centered_rect(60, 22, frame.area());
    crate::ui::draw_modal_overlay(frame, area, theme);

    let block = Block::default()
        .title(" Choose Base Ref ")
        .borders(Borders::ALL)
        .style(Style::default().bg(theme.effective_bg()))
        .border_style(Style::default().fg(theme.primary.to_color()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Branch, tag, or commit to compare against:",
            Style::default().fg(theme.text_muted.to_color()),
        )))
        .wrap(Wrap { trim: false }),
        rows[0],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("› ", Style::default().fg(theme.primary.to_color())),
            Span::styled(
                state.base_ref_input.clone(),
                Style::default().fg(theme.text.to_color()),
            ),
            Span::styled("▏", Style::default().fg(theme.primary.to_color())),
        ])),
        rows[1],
    );

    let key = |k: &'static str| Span::styled(k, Style::default().fg(theme.warning.to_color()));
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            key("Enter"),
            Span::raw(" apply  "),
            key("Esc"),
            Span::raw(" cancel  "),
            Span::styled(
                "(blank = auto)",
                Style::default().fg(theme.text_muted.to_color()),
            ),
        ])),
        rows[3],
    );
}

pub fn draw_diff_viewer_loading(
    frame: &mut Frame,
    state: &DiffViewerState,
    throbber_state: &throbber_widgets_tui::ThrobberState,
    theme: &Theme,
) {
    let area = centered_rect(54, 28, frame.area());
    crate::ui::draw_modal_overlay(frame, area, theme);

    let branch = if state.branch.is_empty() {
        state.from_view.feature_name.as_str()
    } else {
        state.branch.as_str()
    };
    let (title, loading_label, detail) = match &state.scope {
        DiffScope::Commit(commit) => (
            " Commit Diff ",
            " Loading commit diff...",
            format!("{}  {}", commit.short_hash, commit.subject),
        ),
        DiffScope::CurrentChanges => (
            " Current Changes ",
            " Loading current changes...",
            format!("Comparing all changes for {branch}"),
        ),
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .style(Style::default().bg(theme.effective_bg()))
        .border_style(Style::default().fg(theme.primary.to_color()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let throbber = throbber_widgets_tui::Throbber::default()
        .style(Style::default().fg(theme.warning.to_color()));
    let spinner = throbber.to_symbol_span(throbber_state);

    let loading = Paragraph::new(vec![
        Line::from(""),
        Line::from(vec![
            spinner,
            Span::styled(
                loading_label,
                Style::default()
                    .fg(theme.text.to_color())
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            detail,
            Style::default().fg(theme.text_muted.to_color()),
        )),
    ])
    .wrap(Wrap { trim: false });

    frame.render_widget(loading, inner);
}

fn draw_header(frame: &mut Frame, area: Rect, state: &DiffViewerState, theme: &Theme) {
    let branch = if state.branch.is_empty() {
        "(unknown branch)"
    } else {
        &state.branch
    };
    let base = if state.base_ref.is_empty() {
        "(no base)"
    } else {
        &state.base_ref
    };
    let commit = if state.base_commit.is_empty() {
        String::new()
    } else {
        let short = state.base_commit.chars().take(12).collect::<String>();
        format!(" @ {short}")
    };
    let additions: usize = state.files.iter().map(|file| file.additions).sum();
    let deletions: usize = state.files.iter().map(|file| file.deletions).sum();

    let second_line = vec![
        Span::styled(
            format!(" {} file(s)  ", state.files.len()),
            Style::default().fg(theme.text.to_color()),
        ),
        Span::styled(
            format!("+{additions}"),
            Style::default()
                .fg(theme.success.to_color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            format!("-{deletions}"),
            Style::default()
                .fg(theme.danger.to_color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            state.workdir.to_string_lossy().into_owned(),
            Style::default().fg(theme.text_muted.to_color()),
        ),
    ];

    let scope_line = match &state.scope {
        DiffScope::CurrentChanges => vec![
            Span::styled(" Branch ", Style::default().fg(theme.text_muted.to_color())),
            Span::styled(
                branch.to_string(),
                Style::default()
                    .fg(theme.project_title.to_color())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  vs  ", Style::default().fg(theme.text_muted.to_color())),
            Span::styled(
                format!("{base}{commit}"),
                Style::default()
                    .fg(theme.primary.to_color())
                    .add_modifier(Modifier::BOLD),
            ),
            if state.override_base_ref.is_some() {
                Span::styled("  (manual)", Style::default().fg(theme.warning.to_color()))
            } else {
                Span::raw("")
            },
        ],
        DiffScope::Commit(commit) => vec![
            Span::styled(" Commit ", Style::default().fg(theme.text_muted.to_color())),
            Span::styled(
                commit.short_hash.clone(),
                Style::default()
                    .fg(theme.primary.to_color())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                commit.subject.clone(),
                Style::default()
                    .fg(theme.project_title.to_color())
                    .add_modifier(Modifier::BOLD),
            ),
        ],
    };
    let header = Paragraph::new(vec![Line::from(scope_line), Line::from(second_line)])
        .wrap(Wrap { trim: false });

    frame.render_widget(header, area);
}

fn draw_body(frame: &mut Frame, area: Rect, state: &mut DiffViewerState, theme: &Theme) {
    if let Some(error) = &state.error {
        let error_widget = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                match &state.scope {
                    DiffScope::Commit(_) => " Could not load commit diff ",
                    DiffScope::CurrentChanges => " Could not load current changes ",
                },
                Style::default()
                    .fg(theme.danger.to_color())
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                error.as_str(),
                Style::default().fg(theme.text.to_color()),
            )),
        ])
        .wrap(Wrap { trim: false });
        frame.render_widget(error_widget, area);
        return;
    }

    if state.files.is_empty() {
        let empty = Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                match &state.scope {
                    DiffScope::Commit(_) => " The selected commit has no file changes ",
                    DiffScope::CurrentChanges => " No changes against the selected base ",
                },
                Style::default()
                    .fg(theme.success.to_color())
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                match &state.scope {
                    DiffScope::Commit(_) => {
                        "This can happen for an empty commit or some merge commits."
                    }
                    DiffScope::CurrentChanges => {
                        "Refresh with r after making more edits or commits."
                    }
                },
                Style::default().fg(theme.text.to_color()),
            )),
        ]);
        frame.render_widget(empty, area);
        return;
    }

    if state.focus == DiffViewerFocus::Patch {
        draw_patch(frame, area, state, theme);
        return;
    }

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(body_constraints(area, state))
        .split(area);

    draw_file_list(frame, body[0], state, theme);
    draw_patch(frame, body[1], state, theme);
}

fn body_constraints(area: Rect, state: &DiffViewerState) -> [Constraint; 2] {
    match (&effective_layout(state), &state.focus) {
        (DiffViewerLayout::SideBySide, DiffViewerFocus::Patch) => {
            let file_width = area.width.saturating_mul(22) / 100;
            [Constraint::Length(file_width.max(24)), Constraint::Min(40)]
        }
        (DiffViewerLayout::SideBySide, DiffViewerFocus::FileList) => {
            let file_width = area.width.saturating_mul(30) / 100;
            [Constraint::Length(file_width.max(30)), Constraint::Min(34)]
        }
        _ => [Constraint::Percentage(32), Constraint::Percentage(68)],
    }
}
fn draw_file_list(frame: &mut Frame, area: Rect, state: &DiffViewerState, theme: &Theme) {
    let rows = state.file_tree_rows();
    let items: Vec<ListItem<'static>> = rows
        .iter()
        .map(|row| {
            let (idx, depth) = match row {
                crate::app::FileTreeRow::Dir {
                    label,
                    depth,
                    collapsed,
                    files,
                    ..
                } => {
                    let mut spans = vec![
                        Span::raw(" ".repeat(depth * 2 + 1)),
                        Span::styled(
                            if *collapsed { "▸ " } else { "▾ " },
                            Style::default().fg(theme.text_muted.to_color()),
                        ),
                        Span::styled(
                            format!("{label}/"),
                            Style::default()
                                .fg(theme.info.to_color())
                                .add_modifier(Modifier::BOLD),
                        ),
                    ];
                    // A collapsed directory's row says how many files it hides.
                    if *collapsed {
                        spans.push(Span::styled(
                            format!("  ({files})"),
                            Style::default().fg(theme.text_muted.to_color()),
                        ));
                    }
                    return ListItem::new(Line::from(spans));
                }
                crate::app::FileTreeRow::File { index, depth, .. } => (*index, *depth),
            };
            let file = &state.files[idx];
            let status_style = Style::default()
                .fg(status_color(&file.status, theme))
                .add_modifier(Modifier::BOLD);
            let mut spans = Vec::new();
            if depth > 0 {
                spans.push(Span::raw(" ".repeat(depth * 2)));
            }
            spans.push(Span::styled(
                format!(" {} ", status_label(&file.status)),
                status_style,
            ));
            // Only the basename: the directories are the rows above it.
            spans.push(Span::styled(
                match row {
                    crate::app::FileTreeRow::File { name, .. } => name.clone(),
                    crate::app::FileTreeRow::Dir { .. } => file.path.clone(),
                },
                Style::default().fg(theme.text.to_color()),
            ));
            spans.push(Span::styled(
                format!("  +{} -{}", file.additions, file.deletions),
                Style::default().fg(theme.text_muted.to_color()),
            ));
            ListItem::new(Line::from(spans))
        })
        .collect();

    let border = if state.focus == DiffViewerFocus::FileList {
        theme.warning.to_color()
    } else {
        theme.primary.to_color()
    };
    let list = List::new(items)
        .block(
            Block::default()
                .title(format!(" Files ({}) ", state.files.len()))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border)),
        )
        .highlight_style(
            Style::default()
                .bg(theme.shortcut_background.to_color())
                .fg(theme.shortcut_text.to_color())
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">");

    let mut list_state = ListState::default();
    // Highlight maps onto the rendered tree rows: the cursored directory when
    // the cursor is parked on one, else the selected file's row.
    list_state.select(state.tree_cursor_row(&rows));
    frame.render_stateful_widget(list, area, &mut list_state);
}

fn draw_patch(frame: &mut Frame, area: Rect, state: &mut DiffViewerState, theme: &Theme) {
    let border = if state.focus == DiffViewerFocus::Patch {
        theme.warning.to_color()
    } else {
        theme.primary.to_color()
    };
    let file = state.files.get(state.selected_file);
    let title = file
        .map(|file| {
            if is_new_diff_file(file) {
                format!("New File: {}", file.path)
            } else {
                format!("Patch: {}", file.path)
            }
        })
        .unwrap_or_else(|| "Patch".to_string());

    draw_patch_panel(
        frame,
        area,
        file,
        PatchPanelOptions {
            layout: effective_layout(state),
            title,
            border_color: border,
            scroll: state.patch_scroll,
            include_prologue: true,
            new_file_presentation: file.map(is_new_diff_file).unwrap_or(false),
        },
        theme,
    );
}

pub(crate) struct PatchPanelOptions {
    pub layout: DiffViewerLayout,
    pub title: String,
    pub border_color: Color,
    pub scroll: usize,
    pub include_prologue: bool,
    pub new_file_presentation: bool,
}

impl Default for PatchPanelOptions {
    fn default() -> Self {
        Self {
            layout: DiffViewerLayout::Unified,
            title: String::new(),
            border_color: Color::Reset,
            scroll: 0,
            include_prologue: true,
            new_file_presentation: false,
        }
    }
}

pub(crate) fn draw_patch_panel(
    frame: &mut Frame,
    area: Rect,
    file: Option<&DiffFile>,
    options: PatchPanelOptions,
    theme: &Theme,
) {
    let layout_label = match options.layout {
        DiffViewerLayout::Unified => "unified",
        DiffViewerLayout::SideBySide => "side-by-side",
    };
    let title = if options.new_file_presentation {
        Span::styled(
            format!(" {} [{layout_label}] ", options.title),
            Style::default()
                .fg(theme.success.to_color())
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw(format!(" {} [{layout_label}] ", options.title))
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(options.border_color));

    let Some(file) = file else {
        frame.render_widget(Paragraph::new("No file selected").block(block), area);
        return;
    };
    let rows = cached_patch_rows(PatchRowsRequest {
        file,
        width: area.width.saturating_sub(2),
        theme,
        layout: options.layout.clone(),
        include_prologue: options.include_prologue,
        new_file_presentation: options.new_file_presentation,
    });

    // Only the visible rows go to the widget: a `Paragraph` scrolled over the
    // whole file re-walks every row above the viewport on each frame.
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let start = options.scroll.min(rows.lines.len());
    let end = (start + inner.height as usize).min(rows.lines.len());
    let visible = Paragraph::new(rows.lines[start..end].to_vec());
    let visible = if matches!(options.layout, DiffViewerLayout::SideBySide) {
        visible
    } else {
        visible.wrap(Wrap { trim: false })
    };
    frame.render_widget(visible, inner);
}

/// The patch panel's rows for one file, memoised across frames.
///
/// Building them highlights, word-diffs, and wraps the whole file, which is
/// far too slow to repeat on every frame while `j`/`k` is held.
struct PatchRows {
    lines: Vec<Line<'static>>,
}

struct PatchRowsRequest<'a> {
    file: &'a DiffFile,
    width: u16,
    theme: &'a Theme,
    layout: DiffViewerLayout,
    include_prologue: bool,
    new_file_presentation: bool,
}

impl PatchRowsRequest<'_> {
    /// Everything the rows are derived from. Hashing the hunks rather than
    /// `file.patch` matters: context expansion rewrites the hunks without
    /// touching the patch text.
    ///
    /// This re-hashes the file's text every frame — about 0.15ms for a 550KB
    /// diff, against a rebuild of several milliseconds. A revision stamped on
    /// `DiffFile` would be cheaper, but every path that rewrites hunks would
    /// have to remember to bump it, and one that forgot would draw stale rows.
    fn key(&self) -> u64 {
        use std::hash::{Hash, Hasher};

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        highlight::cache_generation().hash(&mut hasher);
        // The same sides and paths `file_highlights` asks the service for.
        if let Some(source) = self.file.old_content.as_deref() {
            highlight::parser_state_for(self.file.old_path.as_deref().map(Path::new), source)
                .hash(&mut hasher);
        }
        if let Some(source) = self.file.new_content.as_deref() {
            highlight::parser_state_for(Some(Path::new(&self.file.path)), source).hash(&mut hasher);
        }
        format!("{:?}", self.theme).hash(&mut hasher);
        self.width.hash(&mut hasher);
        std::mem::discriminant(&self.layout).hash(&mut hasher);
        self.include_prologue.hash(&mut hasher);
        self.new_file_presentation.hash(&mut hasher);

        let file = self.file;
        file.path.hash(&mut hasher);
        file.old_path.hash(&mut hasher);
        std::mem::discriminant(&file.status).hash(&mut hasher);
        file.is_binary.hash(&mut hasher);
        file.patch.hash(&mut hasher);
        file.old_content.hash(&mut hasher);
        file.new_content.hash(&mut hasher);
        for hunk in &file.hunks {
            hunk.header.hash(&mut hasher);
            (
                hunk.old_start,
                hunk.old_lines,
                hunk.new_start,
                hunk.new_lines,
            )
                .hash(&mut hasher);
            for line in &hunk.lines {
                std::mem::discriminant(&line.kind).hash(&mut hasher);
                line.text.hash(&mut hasher);
            }
        }
        hasher.finish()
    }

    fn build(&self) -> PatchRows {
        let lines = match self.layout {
            DiffViewerLayout::SideBySide => {
                side_by_side_lines(self.file, self.width, self.theme, self.include_prologue)
            }
            DiffViewerLayout::Unified => unified_patch_lines(
                self.file,
                self.width,
                self.theme,
                self.include_prologue,
                self.new_file_presentation,
            ),
        };
        PatchRows { lines }
    }
}

thread_local! {
    /// One entry is enough: a frame draws at most one patch panel.
    static PATCH_ROWS_CACHE: std::cell::RefCell<Option<(u64, std::rc::Rc<PatchRows>)>> =
        const { std::cell::RefCell::new(None) };
}

fn cached_patch_rows(request: PatchRowsRequest<'_>) -> std::rc::Rc<PatchRows> {
    let key = request.key();
    PATCH_ROWS_CACHE.with(|cache| {
        if let Some((cached_key, rows)) = cache.borrow().as_ref()
            && *cached_key == key
        {
            return rows.clone();
        }
        let rows = std::rc::Rc::new(request.build());
        *cache.borrow_mut() = Some((key, rows.clone()));
        rows
    })
}

fn draw_footer(frame: &mut Frame, area: Rect, state: &mut DiffViewerState, theme: &Theme) {
    let focus = match state.focus {
        DiffViewerFocus::FileList => "files",
        DiffViewerFocus::Patch => "patch",
    };
    let new_file_selected = state
        .files
        .get(state.selected_file)
        .map(is_new_diff_file)
        .unwrap_or(false);
    let layout = match effective_layout(state) {
        DiffViewerLayout::Unified => "unified",
        DiffViewerLayout::SideBySide => "side-by-side",
    };
    let syntax_status = state
        .files
        .get(state.selected_file)
        .and_then(|file| highlight::language_install_state_for_path(Path::new(&file.path)));
    let context_level = state
        .files
        .get(state.selected_file)
        .filter(|file| file.can_expand_context())
        .map(|file| {
            state
                .context_expansion
                .get(&file.path)
                .copied()
                .unwrap_or(crate::diff::DIFF_DEFAULT_CONTEXT)
        });
    let footer = Paragraph::new(diff_footer_lines(
        focus,
        layout,
        new_file_selected,
        syntax_status,
        matches!(&state.scope, DiffScope::CurrentChanges),
        context_level,
        state.ignore_whitespace,
        theme,
    ))
    .wrap(Wrap { trim: false });
    frame.render_widget(footer, area);
}

#[allow(clippy::too_many_arguments)]
fn diff_footer_lines(
    focus: &str,
    layout: &str,
    new_file_selected: bool,
    syntax_status: Option<(
        highlight::HighlightLanguage,
        highlight::HighlightInstallState,
    )>,
    can_change_base: bool,
    context_level: Option<usize>,
    ignore_whitespace: bool,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let mut primary = vec![
        Span::styled(" Tab", Style::default().fg(theme.warning.to_color())),
        Span::raw(format!(" focus:{focus}  ")),
    ];
    if let Some((language, status)) = syntax_status {
        primary.push(Span::styled(
            "i",
            Style::default().fg(theme.warning.to_color()),
        ));
        let label = match status {
            highlight::HighlightInstallState::Installed => {
                format!(" syntax:{} installed  ", language.display_name())
            }
            highlight::HighlightInstallState::Available => {
                format!(" install {} parser  ", language.display_name())
            }
            highlight::HighlightInstallState::Broken => {
                format!(" repair {} parser  ", language.display_name())
            }
        };
        let color = match status {
            highlight::HighlightInstallState::Installed => theme.info.to_color(),
            highlight::HighlightInstallState::Available => theme.warning.to_color(),
            highlight::HighlightInstallState::Broken => theme.danger.to_color(),
        };
        primary.push(Span::styled(label, Style::default().fg(color)));
    }
    primary.extend(vec![
        Span::styled("Esc", Style::default().fg(theme.warning.to_color())),
        Span::raw(" close"),
    ]);

    let mut secondary = Vec::new();
    if new_file_selected {
        secondary.push(Span::styled(
            format!(" layout:{layout} (new file)  "),
            Style::default().fg(theme.info.to_color()),
        ));
    } else {
        secondary.push(Span::styled(
            "v",
            Style::default().fg(theme.warning.to_color()),
        ));
        secondary.push(Span::raw(format!(" layout:{layout}  ")));
    }
    secondary.extend(vec![
        Span::styled("j/k", Style::default().fg(theme.warning.to_color())),
        Span::raw(" move  "),
        Span::styled("z/Z", Style::default().fg(theme.warning.to_color())),
        Span::raw(" fold  "),
        Span::styled("PgUp/PgDn", Style::default().fg(theme.warning.to_color())),
        Span::raw(" patch  "),
        Span::styled("g/G", Style::default().fg(theme.warning.to_color())),
        Span::raw(" top/bottom  "),
        Span::styled("r", Style::default().fg(theme.warning.to_color())),
        Span::raw(" refresh  "),
    ]);
    if let Some(level) = context_level {
        secondary.push(Span::styled(
            "+/-",
            Style::default().fg(theme.warning.to_color()),
        ));
        secondary.push(Span::styled(
            format!(" context:{}  ", crate::app::context_level_label(level)),
            Style::default().fg(if level == crate::diff::DIFF_DEFAULT_CONTEXT {
                theme.text_muted.to_color()
            } else {
                theme.info.to_color()
            }),
        ));
    }
    secondary.push(Span::styled(
        "W",
        Style::default().fg(theme.warning.to_color()),
    ));
    secondary.push(Span::styled(
        if ignore_whitespace {
            " ws: ignored  "
        } else {
            " ws: shown  "
        },
        Style::default().fg(if ignore_whitespace {
            theme.info.to_color()
        } else {
            theme.text_muted.to_color()
        }),
    ));
    if can_change_base {
        secondary.push(Span::styled(
            "b",
            Style::default().fg(theme.warning.to_color()),
        ));
        secondary.push(Span::raw(" base ref"));
    }

    vec![Line::from(primary), Line::from(secondary)]
}

#[allow(clippy::too_many_arguments)]
/// The unified layout: a two-column line-number gutter and one row per diff
/// line, wrapped to `width`.
fn unified_patch_lines(
    file: &DiffFile,
    width: u16,
    theme: &Theme,
    include_prologue: bool,
    new_file_presentation: bool,
) -> Vec<Line<'static>> {
    let content_width = width as usize;
    if file.is_binary || file.hunks.is_empty() || content_width < 16 {
        return raw_patch_wrapped_lines(file, content_width, theme);
    }

    let number_width = line_number_width(file);
    let gutter_width = number_width * 2 + 4;
    if content_width <= gutter_width + 4 {
        return raw_patch_wrapped_lines(file, content_width, theme);
    }
    let text_width = content_width - gutter_width;
    let highlights = file_highlights(file);
    let added_style = if new_file_presentation {
        new_file_added_row_style(theme)
    } else {
        added_row_style(theme)
    };
    let hunk_style = if new_file_presentation {
        new_file_hunk_header_style(theme)
    } else {
        hunk_header_style(theme)
    };

    let mut lines = Vec::new();
    if include_prologue {
        for meta in patch_prologue(file) {
            lines.extend(wrap_gutter_line(
                None,
                None,
                plain_chunks(meta, meta_style(meta, theme)),
                meta_style(meta, theme),
                number_width,
                text_width,
                theme,
            ));
        }
    }

    for (idx, hunk) in file.hunks.iter().enumerate() {
        if idx > 0 {
            lines.push(Line::from(""));
        }
        lines.extend(wrap_gutter_line(
            None,
            None,
            plain_chunks(&format_hunk_header(hunk), hunk_style),
            hunk_style,
            number_width,
            text_width,
            theme,
        ));

        let mut old_line = hunk.old_start;
        let mut new_line = hunk.new_start;
        // Which tokens changed within each paired removed/added line.
        let intra = hunk_intra_line_ranges(hunk);
        for (line_idx, diff_line) in hunk.lines.iter().enumerate() {
            match diff_line.kind {
                DiffLineKind::Context => {
                    lines.extend(wrap_gutter_line(
                        Some(old_line),
                        Some(new_line),
                        diff_chunks(
                            &diff_line.text,
                            context_row_style(theme),
                            theme,
                            highlighted_line(highlights.new.as_ref(), new_line)
                                .or_else(|| highlighted_line(highlights.old.as_ref(), old_line)),
                        ),
                        context_row_style(theme),
                        number_width,
                        text_width,
                        theme,
                    ));
                    old_line += 1;
                    new_line += 1;
                }
                DiffLineKind::Removed => {
                    lines.extend(wrap_gutter_line(
                        Some(old_line),
                        None,
                        diff_chunks_emphasized(
                            &diff_line.text,
                            removed_row_style(theme),
                            theme,
                            highlighted_line(highlights.old.as_ref(), old_line),
                            intra[line_idx]
                                .clone()
                                .map(|ranges| IntraLineEmphasis {
                                    ranges,
                                    style: removed_emphasis_style(theme),
                                })
                                .as_ref(),
                        ),
                        removed_row_style(theme),
                        number_width,
                        text_width,
                        theme,
                    ));
                    old_line += 1;
                }
                DiffLineKind::Added => {
                    lines.extend(wrap_gutter_line(
                        None,
                        Some(new_line),
                        diff_chunks_emphasized(
                            &diff_line.text,
                            added_style,
                            theme,
                            highlighted_line(highlights.new.as_ref(), new_line),
                            // A brand-new file has no counterpart lines, so its
                            // rows never carry emphasis.
                            intra[line_idx]
                                .clone()
                                .map(|ranges| IntraLineEmphasis {
                                    ranges,
                                    style: added_emphasis_style(theme),
                                })
                                .as_ref(),
                        ),
                        added_style,
                        number_width,
                        text_width,
                        theme,
                    ));
                    new_line += 1;
                }
                DiffLineKind::NoNewlineMarker => {
                    lines.extend(wrap_gutter_line(
                        None,
                        None,
                        plain_chunks(&diff_line.text, meta_subtle_style(theme)),
                        meta_subtle_style(theme),
                        number_width,
                        text_width,
                        theme,
                    ));
                }
            }
        }
    }

    lines
}

fn side_by_side_lines(
    file: &DiffFile,
    width: u16,
    theme: &Theme,
    include_prologue: bool,
) -> Vec<Line<'static>> {
    if file.is_binary || file.hunks.is_empty() || width < 24 {
        return unified_patch_lines(file, width, theme, include_prologue, false);
    }

    let inner_width = width as usize;
    let separator = " | ";
    let column_width = inner_width.saturating_sub(separator.len()) / 2;
    let number_width = line_number_width(file);
    let cell_prefix_width = number_width + 2;
    if column_width <= cell_prefix_width + 6 {
        return unified_patch_lines(file, width, theme, include_prologue, false);
    }
    let cell_text_width = column_width - cell_prefix_width;
    let highlights = file_highlights(file);

    let mut lines = vec![Line::from(vec![
        Span::styled(
            pad_cell("BASE", column_width),
            removed_row_style(theme).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            separator.to_string(),
            Style::default().fg(theme.text_muted.to_color()),
        ),
        Span::styled(
            pad_cell("CURRENT", column_width),
            added_row_style(theme).add_modifier(Modifier::BOLD),
        ),
    ])];

    if include_prologue {
        for meta in patch_prologue(file) {
            for chunk in wrap_text_to_width(meta, inner_width) {
                lines.push(Line::from(Span::styled(chunk, meta_style(meta, theme))));
            }
        }
    }

    for (idx, hunk) in file.hunks.iter().enumerate() {
        if idx > 0 {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(
            format_hunk_header(hunk),
            hunk_header_style(theme),
        )));

        let mut index = 0usize;
        let mut old_line = hunk.old_start;
        let mut new_line = hunk.new_start;
        while index < hunk.lines.len() {
            match hunk.lines[index].kind {
                DiffLineKind::Context => {
                    let text = trim_diff_prefix(&hunk.lines[index]).to_string();
                    lines.extend(side_by_side_rows(
                        Some(old_line),
                        Some(new_line),
                        format!(" {text}"),
                        format!(" {text}"),
                        highlighted_line(highlights.old.as_ref(), old_line),
                        highlighted_line(highlights.new.as_ref(), new_line),
                        context_row_style(theme),
                        context_row_style(theme),
                        number_width,
                        cell_text_width,
                        separator,
                        theme,
                    ));
                    index += 1;
                    old_line += 1;
                    new_line += 1;
                }
                DiffLineKind::Removed => {
                    let removed = collect_run(&hunk.lines, &mut index, DiffLineKind::Removed);
                    let added = collect_run(&hunk.lines, &mut index, DiffLineKind::Added);
                    let row_count = removed.len().max(added.len());
                    for row in 0..row_count {
                        let left = removed
                            .get(row)
                            .map(|line| format!("-{}", trim_diff_prefix(line)))
                            .unwrap_or_default();
                        let right = added
                            .get(row)
                            .map(|line| format!("+{}", trim_diff_prefix(line)))
                            .unwrap_or_default();
                        let left_number = removed.get(row).map(|_| old_line + row);
                        let right_number = added.get(row).map(|_| new_line + row);
                        lines.extend(side_by_side_rows(
                            left_number,
                            right_number,
                            left,
                            right,
                            left_number
                                .and_then(|line| highlighted_line(highlights.old.as_ref(), line)),
                            right_number
                                .and_then(|line| highlighted_line(highlights.new.as_ref(), line)),
                            removed_row_style(theme),
                            added_row_style(theme),
                            number_width,
                            cell_text_width,
                            separator,
                            theme,
                        ));
                    }
                    old_line += removed.len();
                    new_line += added.len();
                }
                DiffLineKind::Added => {
                    let added = collect_run(&hunk.lines, &mut index, DiffLineKind::Added);
                    for (row, line) in added.iter().enumerate() {
                        lines.extend(side_by_side_rows(
                            None,
                            Some(new_line + row),
                            String::new(),
                            format!("+{}", trim_diff_prefix(line)),
                            None,
                            highlighted_line(highlights.new.as_ref(), new_line + row),
                            neutral_side_style(theme),
                            added_row_style(theme),
                            number_width,
                            cell_text_width,
                            separator,
                            theme,
                        ));
                    }
                    new_line += added.len();
                }
                DiffLineKind::NoNewlineMarker => {
                    lines.push(Line::from(Span::styled(
                        hunk.lines[index].text.clone(),
                        meta_subtle_style(theme),
                    )));
                    index += 1;
                }
            }
        }
    }

    lines
}

fn format_hunk_header(hunk: &crate::diff::DiffHunk) -> String {
    format!(
        " Change: base {} -> current {} ",
        format_hunk_range(hunk.old_start, hunk.old_lines),
        format_hunk_range(hunk.new_start, hunk.new_lines)
    )
}

fn format_hunk_range(start: usize, len: usize) -> String {
    match len {
        0 => format!("{start}"),
        1 => format!("{start}"),
        _ => format!("{start}-{}", start + len - 1),
    }
}

fn effective_layout(state: &DiffViewerState) -> DiffViewerLayout {
    state
        .files
        .get(state.selected_file)
        .filter(|file| is_new_diff_file(file))
        .map(|_| DiffViewerLayout::Unified)
        .unwrap_or_else(|| state.layout.clone())
}

fn is_new_diff_file(file: &DiffFile) -> bool {
    matches!(
        file.status,
        DiffFileStatus::Added | DiffFileStatus::Untracked
    )
}

fn status_label(status: &DiffFileStatus) -> &'static str {
    match status {
        DiffFileStatus::Added => "A",
        DiffFileStatus::Modified => "M",
        DiffFileStatus::Deleted => "D",
        DiffFileStatus::Renamed => "R",
        DiffFileStatus::Copied => "C",
        DiffFileStatus::TypeChanged => "T",
        DiffFileStatus::Untracked => "U",
    }
}

fn status_color(status: &DiffFileStatus, theme: &Theme) -> ratatui::style::Color {
    match status {
        DiffFileStatus::Added | DiffFileStatus::Untracked => theme.success.to_color(),
        DiffFileStatus::Modified | DiffFileStatus::Renamed | DiffFileStatus::Copied => {
            theme.warning.to_color()
        }
        DiffFileStatus::Deleted => theme.danger.to_color(),
        DiffFileStatus::TypeChanged => theme.info.to_color(),
    }
}

fn collect_run(lines: &[DiffLine], index: &mut usize, kind: DiffLineKind) -> Vec<DiffLine> {
    let mut run = Vec::new();
    while *index < lines.len() && lines[*index].kind == kind {
        run.push(lines[*index].clone());
        *index += 1;
    }
    run
}

fn trim_diff_prefix(line: &DiffLine) -> &str {
    line.text
        .strip_prefix(['+', '-', ' '])
        .unwrap_or(line.text.as_str())
}

// Layout inputs for one paired diff row; they vary per call site, so a
// struct would be built and torn down for every row.
#[allow(clippy::too_many_arguments)]
fn side_by_side_rows(
    left_number: Option<usize>,
    right_number: Option<usize>,
    left: String,
    right: String,
    left_highlight: Option<&highlight::HighlightedLine>,
    right_highlight: Option<&highlight::HighlightedLine>,
    left_style: Style,
    right_style: Style,
    number_width: usize,
    text_width: usize,
    separator: &str,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let base_bg = popup_base_bg(theme);
    let paired_change_row = left_number.is_some()
        && right_number.is_some()
        && left_style.bg != Some(base_bg)
        && right_style.bg != Some(base_bg);
    // A row showing a removal beside its replacement is exactly the pair a
    // word-level diff describes, so derive the emphasis here rather than
    // threading it through yet another parameter.
    let intra = if paired_change_row {
        crate::worddiff::word_diff_cached(line_content(&left), line_content(&right))
    } else {
        None
    };
    let left_wrapped = if left_number.is_none() && left.is_empty() {
        vec![plain_chunks(
            &hatch_fill(text_width, 0),
            hatched_side_style(right_style, theme),
        )]
    } else {
        wrap_chunks(
            &diff_chunks_emphasized(
                &left,
                left_style,
                theme,
                left_highlight,
                intra
                    .as_ref()
                    .map(|diff| IntraLineEmphasis {
                        ranges: diff.old.clone(),
                        style: removed_emphasis_style(theme),
                    })
                    .as_ref(),
            ),
            text_width,
            left_style,
        )
    };
    let right_wrapped = if right_number.is_none() && right.is_empty() {
        vec![plain_chunks(
            &hatch_fill(text_width, 0),
            hatched_side_style(left_style, theme),
        )]
    } else {
        wrap_chunks(
            &diff_chunks_emphasized(
                &right,
                right_style,
                theme,
                right_highlight,
                intra
                    .as_ref()
                    .map(|diff| IntraLineEmphasis {
                        ranges: diff.new.clone(),
                        style: added_emphasis_style(theme),
                    })
                    .as_ref(),
            ),
            text_width,
            right_style,
        )
    };
    let row_count = left_wrapped.len().max(right_wrapped.len());
    let mut rows = Vec::with_capacity(row_count);
    let left_missing = left_number.is_none() && left.is_empty();
    let right_missing = right_number.is_none() && right.is_empty();

    for row in 0..row_count {
        let left_has_content = left_wrapped
            .get(row)
            .map(|chunks| !chunks.is_empty())
            .unwrap_or(false);
        let right_has_content = right_wrapped
            .get(row)
            .map(|chunks| !chunks.is_empty())
            .unwrap_or(false);
        let left_prefix = if row == 0 {
            line_number_label(left_number, number_width)
        } else {
            blank_line_number_label(number_width)
        };
        let right_prefix = if row == 0 {
            line_number_label(right_number, number_width)
        } else {
            blank_line_number_label(number_width)
        };
        let left_cell_style = if left_missing {
            hatched_side_style(right_style, theme)
        } else if left_has_content {
            left_style
        } else {
            context_row_style(theme)
        };
        let right_cell_style = if right_missing {
            hatched_side_style(left_style, theme)
        } else if right_has_content {
            right_style
        } else {
            context_row_style(theme)
        };
        let left_bg = left_cell_style.bg.unwrap_or_else(|| popup_base_bg(theme));
        let right_bg = right_cell_style.bg.unwrap_or_else(|| popup_base_bg(theme));
        let left_cell = if left_missing {
            pad_chunks_to_width(
                plain_chunks(&hatch_fill(text_width, row), left_cell_style),
                text_width,
                left_cell_style,
            )
        } else {
            pad_chunks_to_width(
                left_wrapped.get(row).cloned().unwrap_or_default(),
                text_width,
                if paired_change_row {
                    Style::default().bg(base_bg)
                } else {
                    left_style
                },
            )
        };
        let right_cell = if right_missing {
            pad_chunks_to_width(
                plain_chunks(&hatch_fill(text_width, row), right_cell_style),
                text_width,
                right_cell_style,
            )
        } else {
            pad_chunks_to_width(
                right_wrapped.get(row).cloned().unwrap_or_default(),
                text_width,
                if paired_change_row {
                    Style::default().bg(base_bg)
                } else {
                    right_style
                },
            )
        };
        let mut line = vec![Span::styled(left_prefix, left_cell_style)];
        line.push(Span::styled(
            "│ ",
            Style::default()
                .fg(line_number_fg(left_cell_style, left_bg))
                .bg(left_bg),
        ));
        line.extend(chunks_to_spans(left_cell));
        line.push(Span::styled(
            separator.to_string(),
            Style::default().fg(theme.text_muted.to_color()).bg(
                if paired_change_row && (!left_has_content || !right_has_content) {
                    base_bg
                } else {
                    blend_color(left_bg, right_bg, 0.5)
                },
            ),
        ));
        line.push(Span::styled(right_prefix, right_cell_style));
        line.push(Span::styled(
            "│ ",
            Style::default()
                .fg(line_number_fg(right_cell_style, right_bg))
                .bg(right_bg),
        ));
        line.extend(chunks_to_spans(right_cell));
        rows.push(Line::from(line));
    }

    rows
}

fn raw_patch_wrapped_lines(file: &DiffFile, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for line in file.patch.lines() {
        for chunk in wrap_text_to_width(line, width) {
            lines.push(Line::from(Span::styled(chunk, meta_style(line, theme))));
        }
    }
    lines
}
fn wrap_gutter_line(
    old_number: Option<usize>,
    new_number: Option<usize>,
    chunks: Vec<StyledChunk>,
    style: Style,
    number_width: usize,
    text_width: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let _ = theme;
    let wrapped = wrap_chunks(&chunks, text_width, style);
    let mut lines = Vec::with_capacity(wrapped.len());
    let row_bg = style.bg.unwrap_or(Color::Black);
    let number_fg = line_number_fg(style, row_bg);
    for (index, chunk_line) in wrapped.into_iter().enumerate() {
        let old_label = if index == 0 {
            line_number_label(old_number, number_width)
        } else {
            blank_line_number_label(number_width)
        };
        let new_label = if index == 0 {
            line_number_label(new_number, number_width)
        } else {
            blank_line_number_label(number_width)
        };
        let mut line = vec![
            Span::styled(old_label, Style::default().fg(number_fg).bg(row_bg)),
            Span::styled(" ", Style::default().bg(row_bg)),
            Span::styled(new_label, Style::default().fg(number_fg).bg(row_bg)),
            Span::styled(" ", Style::default().bg(row_bg)),
            Span::styled("│ ", Style::default().fg(number_fg).bg(row_bg)),
        ];
        line.extend(chunks_to_spans(chunk_line));
        lines.push(Line::from(line));
    }
    lines
}

fn diff_chunks(
    text: &str,
    row_style: Style,
    theme: &Theme,
    highlighted_line: Option<&highlight::HighlightedLine>,
) -> Vec<StyledChunk> {
    diff_chunks_emphasized(text, row_style, theme, highlighted_line, None)
}

/// As `diff_chunks`, but additionally brightens the byte ranges in `emphasis`
/// (offsets into the line's content, prefix excluded) — the tokens a
/// word-level diff found actually changed. Only the background is touched, so
/// syntax highlighting shows through unchanged.
fn diff_chunks_emphasized(
    text: &str,
    row_style: Style,
    theme: &Theme,
    highlighted_line: Option<&highlight::HighlightedLine>,
    emphasis: Option<&IntraLineEmphasis>,
) -> Vec<StyledChunk> {
    if text.is_empty() {
        return Vec::new();
    }

    let mut chars = text.chars();
    let first = chars.next().expect("diff chunk text should not be empty");
    let (prefix, content) = if matches!(first, '+' | '-' | ' ') {
        (Some(first), chars.as_str())
    } else {
        (None, text)
    };

    let mut chunks = Vec::new();
    if let Some(prefix) = prefix {
        chunks.push(StyledChunk {
            text: prefix.to_string(),
            style: row_style,
        });
    }

    if !content.is_empty() {
        let mut content_chunks = Vec::new();
        append_highlighted_content(
            &mut content_chunks,
            content,
            row_style,
            theme,
            highlighted_line,
        );
        if let Some(emphasis) = emphasis {
            content_chunks =
                apply_intra_line_emphasis(content_chunks, &emphasis.ranges, emphasis.style);
        }
        chunks.extend(content_chunks);
    }

    chunks
}

/// The changed byte ranges of one line plus the style to mark them with.
struct IntraLineEmphasis {
    ranges: Vec<std::ops::Range<usize>>,
    style: Style,
}

/// Split `chunks` at every emphasis boundary and patch `style` onto the parts
/// that fall inside a changed range. `chunks` must cover the line's content
/// contiguously from offset 0, which is what `append_highlighted_content`
/// guarantees.
fn apply_intra_line_emphasis(
    chunks: Vec<StyledChunk>,
    ranges: &[std::ops::Range<usize>],
    style: Style,
) -> Vec<StyledChunk> {
    if ranges.is_empty() {
        return chunks;
    }
    let mut rebuilt: Vec<StyledChunk> = Vec::with_capacity(chunks.len());
    let mut offset = 0usize;
    for chunk in chunks.iter() {
        let chunk_start = offset;
        offset += chunk.text.len();
        // Cut points inside this chunk where emphasis starts or stops.
        let mut cuts: Vec<usize> = vec![0, chunk.text.len()];
        for range in ranges {
            for edge in [range.start, range.end] {
                if edge > chunk_start && edge < offset {
                    cuts.push(edge - chunk_start);
                }
            }
        }
        cuts.sort_unstable();
        cuts.dedup();
        for pair in cuts.windows(2) {
            let (from, to) = (pair[0], pair[1]);
            let Some(text) = chunk.text.get(from..to) else {
                // A cut landed mid-character: keep the chunk whole rather than
                // slicing a UTF-8 boundary.
                continue;
            };
            let absolute = chunk_start + from;
            let emphasized = ranges
                .iter()
                .any(|range| absolute >= range.start && absolute < range.end);
            rebuilt.push(StyledChunk {
                text: text.to_string(),
                style: if emphasized {
                    chunk.style.patch(style)
                } else {
                    chunk.style
                },
            });
        }
    }
    rebuilt
}

fn append_highlighted_content(
    chunks: &mut Vec<StyledChunk>,
    content: &str,
    row_style: Style,
    theme: &Theme,
    highlighted_line: Option<&highlight::HighlightedLine>,
) {
    let Some(highlighted_line) = highlighted_line else {
        chunks.push(StyledChunk {
            text: content.to_string(),
            style: row_style,
        });
        return;
    };

    if highlighted_line.spans.is_empty() {
        chunks.push(StyledChunk {
            text: content.to_string(),
            style: row_style,
        });
        return;
    }

    let mut rendered_any = false;
    let mut remaining = content;
    for span in &highlighted_line.spans {
        if remaining.is_empty() {
            break;
        }
        if span.text.is_empty() {
            continue;
        }

        let consumed = consume_shared_prefix(remaining, &span.text);
        if consumed.is_empty() {
            continue;
        }

        chunks.push(StyledChunk {
            text: consumed.to_string(),
            style: row_style.patch(highlight::style_for_class(span.class, theme)),
        });
        remaining = &remaining[consumed.len()..];
        rendered_any = true;
    }

    if !remaining.is_empty() {
        chunks.push(StyledChunk {
            text: remaining.to_string(),
            style: row_style,
        });
    } else if !rendered_any {
        chunks.push(StyledChunk {
            text: content.to_string(),
            style: row_style,
        });
    }
}

fn consume_shared_prefix<'a>(content: &'a str, highlighted: &str) -> &'a str {
    let mut end = 0usize;
    for (left, right) in content.chars().zip(highlighted.chars()) {
        if left != right {
            break;
        }
        end += left.len_utf8();
    }
    &content[..end]
}

fn file_highlights(file: &DiffFile) -> FileHighlights {
    let old = file.old_content.as_deref().map(|source| {
        highlight::highlight_source(highlight::HighlightRequest {
            path: file.old_path.as_deref().map(Path::new),
            language_hint: None,
            source,
        })
    });
    let new = file.new_content.as_deref().map(|source| {
        highlight::highlight_source(highlight::HighlightRequest {
            path: Some(Path::new(&file.path)),
            language_hint: None,
            source,
        })
    });

    FileHighlights { old, new }
}

fn highlighted_line(
    highlighted: Option<&highlight::HighlightedText>,
    line_number: usize,
) -> Option<&highlight::HighlightedLine> {
    line_number
        .checked_sub(1)
        .and_then(|index| highlighted.and_then(|text| text.lines.get(index)))
}

fn plain_chunks(text: &str, style: Style) -> Vec<StyledChunk> {
    if text.is_empty() {
        Vec::new()
    } else {
        vec![StyledChunk {
            text: text.to_string(),
            style,
        }]
    }
}

fn wrap_chunks(
    chunks: &[StyledChunk],
    width: usize,
    fallback_style: Style,
) -> Vec<Vec<StyledChunk>> {
    if width == 0 {
        return vec![Vec::new()];
    }
    if chunks.is_empty() {
        return vec![Vec::new()];
    }

    let mut lines = Vec::new();
    let mut current = Vec::new();
    let mut used = 0usize;

    for chunk in chunks {
        for ch in chunk.text.chars() {
            let ch_width = UnicodeWidthStr::width(ch.encode_utf8(&mut [0; 4])).max(1);
            if used + ch_width > width && !current.is_empty() {
                lines.push(current);
                current = Vec::new();
                used = 0;
            }

            push_chunk_char(&mut current, chunk.style, ch);
            used += ch_width;

            if used >= width {
                lines.push(current);
                current = Vec::new();
                used = 0;
            }
        }
    }

    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(plain_chunks("", fallback_style));
    }

    lines
}

fn push_chunk_char(chunks: &mut Vec<StyledChunk>, style: Style, ch: char) {
    if let Some(last) = chunks.last_mut()
        && last.style == style
    {
        last.text.push(ch);
        return;
    }
    chunks.push(StyledChunk {
        text: ch.to_string(),
        style,
    });
}

fn chunks_to_spans(chunks: Vec<StyledChunk>) -> Vec<Span<'static>> {
    chunks
        .into_iter()
        .map(|chunk| Span::styled(chunk.text, chunk.style))
        .collect()
}

fn pad_chunks_to_width(
    mut chunks: Vec<StyledChunk>,
    width: usize,
    pad_style: Style,
) -> Vec<StyledChunk> {
    let used = chunks_width(&chunks);
    if used < width {
        chunks.push(StyledChunk {
            text: " ".repeat(width - used),
            style: pad_style,
        });
    }
    chunks
}

fn chunks_width(chunks: &[StyledChunk]) -> usize {
    chunks
        .iter()
        .map(|chunk| UnicodeWidthStr::width(chunk.text.as_str()))
        .sum()
}

fn line_number_width(file: &DiffFile) -> usize {
    let mut max_line = 1usize;
    for hunk in &file.hunks {
        max_line = max_line.max(hunk.old_start.saturating_add(hunk.old_lines));
        max_line = max_line.max(hunk.new_start.saturating_add(hunk.new_lines));
    }
    max_line.to_string().len().max(1)
}

fn patch_prologue(file: &DiffFile) -> Vec<&str> {
    if is_new_diff_file(file) {
        return vec!["NEW FILE added in this branch"];
    }
    file.patch
        .lines()
        .take_while(|line| !line.starts_with("@@ "))
        .collect()
}

fn meta_style(line: &str, theme: &Theme) -> Style {
    if line.starts_with("NEW FILE ") || line.starts_with("New file ") {
        new_file_hunk_header_style(theme)
    } else if line.starts_with("diff --git ")
        || line.starts_with("index ")
        || line.starts_with("new file mode ")
        || line.starts_with("deleted file mode ")
        || line.starts_with("rename from ")
        || line.starts_with("rename to ")
        || line.starts_with("copy from ")
        || line.starts_with("copy to ")
    {
        meta_subtle_style(theme)
    } else if line.starts_with("@@ ") {
        hunk_header_style(theme)
    } else if line.starts_with('+') && !line.starts_with("+++") {
        added_row_style(theme)
    } else if line.starts_with('-') && !line.starts_with("---") {
        removed_row_style(theme)
    } else if line.starts_with("+++ ") || line.starts_with("--- ") {
        meta_subtle_style(theme)
    } else {
        context_row_style(theme)
    }
}

fn line_number_label(number: Option<usize>, width: usize) -> String {
    match number {
        Some(number) => format!("{number:>width$}", width = width),
        None => blank_line_number_label(width),
    }
}

fn blank_line_number_label(width: usize) -> String {
    " ".repeat(width)
}

fn popup_base_bg(theme: &Theme) -> Color {
    theme.background.to_color()
}

fn context_row_style(theme: &Theme) -> Style {
    Style::default()
        .fg(theme.text.to_color())
        .bg(popup_base_bg(theme))
}

fn neutral_side_style(theme: &Theme) -> Style {
    Style::default()
        .fg(theme.text_muted.to_color())
        .bg(blend_color(
            popup_base_bg(theme),
            theme.header_background.to_color(),
            0.42,
        ))
}

fn added_row_style(theme: &Theme) -> Style {
    Style::default().fg(theme.text.to_color()).bg(blend_color(
        popup_base_bg(theme),
        theme.success.to_color(),
        0.28,
    ))
}

/// Background for the tokens a word-level diff found changed: the row's own
/// hue, blended harder so the changed part reads as "more added" / "more
/// removed" rather than as a different kind of thing. Only the background is
/// set, so the syntax-highlight foreground survives.
fn added_emphasis_style(theme: &Theme) -> Style {
    Style::default().bg(blend_color(
        popup_base_bg(theme),
        theme.success.to_color(),
        0.62,
    ))
}

fn removed_emphasis_style(theme: &Theme) -> Style {
    Style::default().bg(blend_color(
        popup_base_bg(theme),
        theme.danger.to_color(),
        0.58,
    ))
}

/// Word-level emphasis for every line of a hunk, indexed the same as
/// `hunk.lines`.
///
/// Consecutive removed lines followed by consecutive added lines form a change
/// block; within it the i-th removal pairs with the i-th addition, which is how
/// git lays out a rewritten run and therefore what the reviewer reads as "this
/// line became that line". Unpaired leftovers (a block that removes 3 and adds
/// 1) get no emphasis — there is no counterpart to diff against.
///
/// Runs on every frame, so the per-pair token diff comes from
/// `worddiff::word_diff_cached` rather than being recomputed: only the (cheap)
/// walk over the hunk's lines is repeated.
fn hunk_intra_line_ranges(hunk: &DiffHunk) -> Vec<Option<Vec<std::ops::Range<usize>>>> {
    let mut out: Vec<Option<Vec<std::ops::Range<usize>>>> = vec![None; hunk.lines.len()];
    let mut idx = 0usize;
    while idx < hunk.lines.len() {
        if !matches!(hunk.lines[idx].kind, DiffLineKind::Removed) {
            idx += 1;
            continue;
        }
        let removed_start = idx;
        while idx < hunk.lines.len() && matches!(hunk.lines[idx].kind, DiffLineKind::Removed) {
            idx += 1;
        }
        let added_start = idx;
        while idx < hunk.lines.len() && matches!(hunk.lines[idx].kind, DiffLineKind::Added) {
            idx += 1;
        }

        let removed = removed_start..added_start;
        let added = added_start..idx;
        for (old_idx, new_idx) in removed.zip(added) {
            let old_text = line_content(&hunk.lines[old_idx].text);
            let new_text = line_content(&hunk.lines[new_idx].text);
            if let Some(diff) = crate::worddiff::word_diff_cached(old_text, new_text) {
                out[old_idx] = Some(diff.old.clone());
                out[new_idx] = Some(diff.new.clone());
            }
        }
    }
    out
}

/// A diff line's text with its leading `+`/`-`/space prefix removed, matching
/// the offsets `diff_chunks_emphasized` measures emphasis ranges against.
fn line_content(text: &str) -> &str {
    let mut chars = text.chars();
    match chars.next() {
        Some('+' | '-' | ' ') => chars.as_str(),
        _ => text,
    }
}

fn removed_row_style(theme: &Theme) -> Style {
    Style::default().fg(theme.text.to_color()).bg(blend_color(
        popup_base_bg(theme),
        theme.danger.to_color(),
        0.26,
    ))
}

fn hunk_header_style(theme: &Theme) -> Style {
    Style::default()
        .fg(theme.info.to_color())
        .bg(blend_color(
            popup_base_bg(theme),
            theme.info.to_color(),
            0.12,
        ))
        .add_modifier(Modifier::BOLD)
}

fn new_file_added_row_style(theme: &Theme) -> Style {
    Style::default()
        .fg(theme.success.to_color())
        .bg(popup_base_bg(theme))
}

fn new_file_hunk_header_style(theme: &Theme) -> Style {
    Style::default()
        .fg(theme.success.to_color())
        .bg(blend_color(
            popup_base_bg(theme),
            theme.success.to_color(),
            0.18,
        ))
        .add_modifier(Modifier::BOLD)
}

fn meta_subtle_style(theme: &Theme) -> Style {
    Style::default()
        .fg(theme.text_muted.to_color())
        .bg(blend_color(
            popup_base_bg(theme),
            theme.primary.to_color(),
            0.08,
        ))
}

fn line_number_fg(style: Style, row_bg: Color) -> Color {
    style.fg.unwrap_or(blend_color(row_bg, Color::White, 0.55))
}

fn hatched_side_style(reference: Style, theme: &Theme) -> Style {
    let row_bg = reference.bg.unwrap_or_else(|| popup_base_bg(theme));
    Style::default()
        .fg(blend_color(row_bg, Color::White, 0.42))
        .bg(row_bg)
}

fn hatch_fill(width: usize, row: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let _ = row;
    " ".repeat(width)
}

fn blend_color(base: Color, overlay: Color, alpha: f32) -> Color {
    let alpha = alpha.clamp(0.0, 1.0);
    let (br, bg, bb) = color_to_rgb(base);
    let (or, og, ob) = color_to_rgb(overlay);
    Color::Rgb(
        ((br as f32 * (1.0 - alpha)) + (or as f32 * alpha)).round() as u8,
        ((bg as f32 * (1.0 - alpha)) + (og as f32 * alpha)).round() as u8,
        ((bb as f32 * (1.0 - alpha)) + (ob as f32 * alpha)).round() as u8,
    )
}

fn color_to_rgb(color: Color) -> (u8, u8, u8) {
    match color {
        Color::Black => (0, 0, 0),
        Color::Red => (205, 49, 49),
        Color::Green => (13, 188, 121),
        Color::Yellow => (229, 229, 16),
        Color::Blue => (36, 114, 200),
        Color::Magenta => (188, 63, 188),
        Color::Cyan => (17, 168, 205),
        Color::Gray => (204, 204, 204),
        Color::DarkGray => (118, 118, 118),
        Color::LightRed => (241, 76, 76),
        Color::LightGreen => (35, 209, 139),
        Color::LightYellow => (245, 245, 67),
        Color::LightBlue => (59, 142, 234),
        Color::LightMagenta => (214, 112, 214),
        Color::LightCyan => (41, 184, 219),
        Color::White => (242, 242, 242),
        Color::Rgb(r, g, b) => (r, g, b),
        Color::Indexed(i) => (i, i, i),
        Color::Reset => (48, 52, 70),
    }
}

fn pad_cell(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let ch_width = UnicodeWidthStr::width(ch.encode_utf8(&mut [0; 4]));
        if used + ch_width > width {
            break;
        }
        out.push(ch);
        used += ch_width;
    }

    if used < width {
        out.push_str(&" ".repeat(width - used));
    }

    out
}

fn wrap_text_to_width(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    if text.is_empty() {
        return vec![String::new()];
    }

    let mut out = Vec::new();
    let mut current = String::new();
    let mut used = 0usize;

    for ch in text.chars() {
        let mut buf = [0; 4];
        let ch_str = ch.encode_utf8(&mut buf);
        let ch_width = UnicodeWidthStr::width(ch_str).max(1);

        if used + ch_width > width && !current.is_empty() {
            out.push(current);
            current = String::new();
            used = 0;
        }

        current.push(ch);
        used += ch_width;

        if used >= width {
            out.push(current);
            current = String::new();
            used = 0;
        }
    }

    if !current.is_empty() {
        out.push(current);
    }

    if out.is_empty() {
        out.push(String::new());
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_text(line: &Line<'static>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    }

    fn changed_pair_hunk(old: &str, new: &str) -> DiffHunk {
        DiffHunk {
            header: "@@ -1 +1 @@".into(),
            old_start: 1,
            old_lines: 1,
            new_start: 1,
            new_lines: 1,
            lines: vec![
                DiffLine {
                    kind: DiffLineKind::Removed,
                    text: format!("-{old}"),
                },
                DiffLine {
                    kind: DiffLineKind::Added,
                    text: format!("+{new}"),
                },
            ],
        }
    }

    #[test]
    fn intra_line_ranges_pair_a_removal_with_its_replacement() {
        let hunk = changed_pair_hunk("let x = foo(1);", "let x = foo(2);");
        let ranges = hunk_intra_line_ranges(&hunk);

        // Index 0 is the removal, index 1 its paired addition.
        let old = ranges[0].as_ref().expect("removal has emphasis");
        let new = ranges[1].as_ref().expect("addition has emphasis");
        assert_eq!(&"let x = foo(1);"[old[0].clone()], "1");
        assert_eq!(&"let x = foo(2);"[new[0].clone()], "2");
    }

    #[test]
    fn intra_line_ranges_skip_unpaired_and_context_lines() {
        // Two removals, one addition: only the first pair has a counterpart.
        let hunk = DiffHunk {
            header: "@@ -1,3 +1,2 @@".into(),
            old_start: 1,
            old_lines: 3,
            new_start: 1,
            new_lines: 2,
            lines: vec![
                DiffLine {
                    kind: DiffLineKind::Context,
                    text: " unchanged".into(),
                },
                DiffLine {
                    kind: DiffLineKind::Removed,
                    text: "-value = 1;".into(),
                },
                DiffLine {
                    kind: DiffLineKind::Removed,
                    text: "-dropped();".into(),
                },
                DiffLine {
                    kind: DiffLineKind::Added,
                    text: "+value = 2;".into(),
                },
            ],
        };
        let ranges = hunk_intra_line_ranges(&hunk);

        assert!(ranges[0].is_none(), "context lines are never emphasised");
        assert!(ranges[1].is_some(), "first removal pairs with the addition");
        assert!(ranges[2].is_none(), "second removal has no counterpart");
        assert!(ranges[3].is_some());
    }

    #[test]
    fn emphasis_splits_chunks_without_losing_or_reordering_text() {
        let theme = Theme::default();
        let row = added_row_style(&theme);
        let emphasis = IntraLineEmphasis {
            // Two disjoint runs inside "abcdef": "bc" and "e".
            ranges: Vec::from([1..3, 4..5]),
            style: added_emphasis_style(&theme),
        };
        let chunks = diff_chunks_emphasized("+abcdef", row, &theme, None, Some(&emphasis));

        let text: String = chunks.iter().map(|c| c.text.as_str()).collect();
        assert_eq!(text, "+abcdef", "no text may be lost or reordered");

        // Only the emphasised slices carry the brighter background; the rest
        // keeps the plain row background.
        let emphasised: String = chunks
            .iter()
            .filter(|c| c.style.bg == added_emphasis_style(&theme).bg)
            .map(|c| c.text.as_str())
            .collect();
        assert_eq!(emphasised, "bce");
    }

    #[test]
    fn emphasis_offsets_are_measured_past_the_diff_prefix() {
        let theme = Theme::default();
        // Offsets are into the *content*, not the raw line — starting a range
        // at 0 must highlight `a`, never the `+` prefix.
        let emphasis = IntraLineEmphasis {
            ranges: Vec::from([0..3, 5..6]),
            style: added_emphasis_style(&theme),
        };
        let chunks = diff_chunks_emphasized(
            "+abcdef",
            added_row_style(&theme),
            &theme,
            None,
            Some(&emphasis),
        );

        let emphasised: String = chunks
            .iter()
            .filter(|c| c.style.bg == added_emphasis_style(&theme).bg)
            .map(|c| c.text.as_str())
            .collect();
        assert_eq!(emphasised, "abcf");
    }

    #[test]
    fn diff_footer_shows_the_ignore_whitespace_state() {
        let theme = Theme::default();
        let shown = diff_footer_lines("files", "unified", false, None, true, None, false, &theme);
        assert!(line_text(&shown[1]).contains("ws: shown"));

        let ignored = diff_footer_lines("files", "unified", false, None, true, None, true, &theme);
        assert!(line_text(&ignored[1]).contains("ws: ignored"));
    }

    #[test]
    fn diff_footer_prioritizes_syntax_install_hint() {
        let theme = Theme::default();
        let lines = diff_footer_lines(
            "files",
            "unified",
            true,
            Some((
                highlight::HighlightLanguage::Tsx,
                highlight::HighlightInstallState::Available,
            )),
            true,
            None,
            false,
            &theme,
        );

        assert_eq!(lines.len(), 2);
        assert!(line_text(&lines[0]).contains("install tsx parser"));
        assert!(line_text(&lines[0]).contains("Esc close"));
        assert!(line_text(&lines[1]).contains("layout:unified (new file)"));
    }

    #[test]
    fn new_file_patch_lines_preserve_syntax_coloring_for_indented_javascript() {
        if highlight::HighlightLanguage::JavaScript.install_state()
            != highlight::HighlightInstallState::Installed
        {
            return;
        }

        crate::highlight::reload_runtime_state();

        let theme = Theme::default();
        let file = DiffFile {
            old_path: None,
            path: "syntax-test.js".to_string(),
            status: DiffFileStatus::Added,
            additions: 3,
            deletions: 0,
            is_binary: false,
            old_content: None,
            new_content: Some("const palette = {\n  primary: \"#0f172a\",\n};\n".to_string()),
            patch: "\
diff --git a/syntax-test.js b/syntax-test.js
new file mode 100644
index 0000000..1111111
--- /dev/null
+++ b/syntax-test.js
@@ -0,0 +1,3 @@
+const palette = {
+  primary: \"#0f172a\",
+};
"
            .to_string(),
            hunks: vec![crate::diff::DiffHunk {
                header: "@@ -0,0 +1,3 @@".to_string(),
                old_start: 0,
                old_lines: 0,
                new_start: 1,
                new_lines: 3,
                lines: vec![
                    crate::diff::DiffLine {
                        kind: crate::diff::DiffLineKind::Added,
                        text: "+const palette = {".to_string(),
                    },
                    crate::diff::DiffLine {
                        kind: crate::diff::DiffLineKind::Added,
                        text: "+  primary: \"#0f172a\",".to_string(),
                    },
                    crate::diff::DiffLine {
                        kind: crate::diff::DiffLineKind::Added,
                        text: "+};".to_string(),
                    },
                ],
            }],
        };

        let lines = unified_patch_lines(&file, 100, &theme, false, true);
        let indented_code_line = &lines[2];
        let default_added_fg = new_file_added_row_style(&theme).fg;
        let has_syntax_colored_token = indented_code_line.spans.iter().any(|span| {
            !span.content.trim().is_empty()
                && span.content.contains("primary")
                && span.style.fg != default_added_fg
        });

        assert!(
            has_syntax_colored_token,
            "expected indented JavaScript property to keep syntax coloring in new-file diff rows"
        );
    }

    #[test]
    fn new_file_patch_lines_preserve_syntax_coloring_for_typescript() {
        if highlight::HighlightLanguage::TypeScript.install_state()
            != highlight::HighlightInstallState::Installed
        {
            return;
        }

        crate::highlight::reload_runtime_state();

        let theme = Theme::default();
        let file = DiffFile {
            old_path: None,
            path: "docs/syntax-tests/syntax-test-highlight.ts".to_string(),
            status: DiffFileStatus::Added,
            additions: 74,
            deletions: 0,
            is_binary: false,
            old_content: None,
            new_content: Some(
                include_str!("../../../docs/syntax-tests/syntax-test-highlight.ts").to_string(),
            ),
            patch: "\
diff --git a/docs/syntax-tests/syntax-test-highlight.ts b/docs/syntax-tests/syntax-test-highlight.ts
new file mode 100644
index 0000000..1111111
--- /dev/null
+++ b/docs/syntax-tests/syntax-test-highlight.ts
@@ -0,0 +1,74 @@
"
            .to_string(),
            hunks: vec![crate::diff::DiffHunk {
                header: "@@ -0,0 +1,74 @@".to_string(),
                old_start: 0,
                old_lines: 0,
                new_start: 1,
                new_lines: 74,
                lines: include_str!("../../../docs/syntax-tests/syntax-test-highlight.ts")
                    .lines()
                    .map(|line| crate::diff::DiffLine {
                        kind: crate::diff::DiffLineKind::Added,
                        text: format!("+{line}"),
                    })
                    .collect(),
            }],
        };

        let lines = unified_patch_lines(&file, 120, &theme, false, true);
        let has_syntax_colored_token =
            lines.iter().flat_map(|line| line.spans.iter()).any(|span| {
                !span.content.trim().is_empty()
                    && span.content.contains("JsonPrimitive")
                    && span.style.fg != new_file_added_row_style(&theme).fg
            });

        assert!(
            has_syntax_colored_token,
            "expected TypeScript diff rows to keep syntax coloring in new-file diff rows"
        );
    }

    fn viewer_state() -> DiffViewerState {
        DiffViewerState::new(
            crate::app::ViewState::new(
                "proj".into(),
                "feat".into(),
                "sess".into(),
                "claude".into(),
                "Claude".into(),
                crate::project::SessionKind::Claude,
                crate::project::VibeMode::Vibeless,
            ),
            std::path::PathBuf::from("/tmp"),
        )
    }

    #[test]
    fn file_list_renders_a_directory_tree_and_folds_it() {
        use ratatui::{Terminal, backend::TestBackend};

        let mut state = viewer_state();
        state.files = ["src/app/mod.rs", "src/app/state.rs", "src/ui/diff.rs"]
            .iter()
            .map(|path| DiffFile {
                old_path: Some((*path).to_string()),
                path: (*path).to_string(),
                status: DiffFileStatus::Modified,
                additions: 2,
                deletions: 1,
                is_binary: false,
                old_content: None,
                new_content: None,
                patch: String::new(),
                hunks: vec![],
            })
            .collect();
        state.selected_file = 0;

        // Only the file-list column: the patch panel beside it still titles
        // itself with the full path, which would mask what the rows show.
        const WIDTH: u16 = 120;
        const FILE_LIST_COLS: usize = 34;
        let render = |state: &mut DiffViewerState| {
            let backend = TestBackend::new(WIDTH, 30);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal
                .draw(|frame| draw_diff_viewer(frame, state, &Theme::default()))
                .unwrap();
            let cells: Vec<String> = terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|cell| cell.symbol().to_string())
                .collect();
            cells
                .chunks(WIDTH as usize)
                .map(|row| row[..FILE_LIST_COLS].concat())
                .collect::<Vec<String>>()
                .join("\n")
        };

        let rendered = render(&mut state);
        // Directory headers replace the repeated path prefixes, and file rows
        // show only their basename.
        assert!(rendered.contains("▾ src/"), "expected a src directory row");
        assert!(
            rendered.contains("  ▾ app/"),
            "expected a nested, indented app directory row: {rendered}"
        );
        assert!(rendered.contains("mod.rs"));
        assert!(
            !rendered.contains("src/app/mod.rs"),
            "file rows should drop the prefix their directory rows already show"
        );

        // Folding hides the files beneath and summarises what it swallowed.
        state.toggle_dir_collapsed("src/app");
        let folded = render(&mut state);
        assert!(!folded.contains("mod.rs"));
        assert!(folded.contains("▸ app/"));
        assert!(
            folded.contains("(2)"),
            "collapsed row should count its files"
        );
    }
}
