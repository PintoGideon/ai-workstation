//! FORK: G11 UI revamp (DESIGN.md §12) — the shared look of the dashboard
//! chrome: rounded panels, the header's status pills, key-hint styling and the
//! designed empty state.
//!
//! Everything here is drawn from data the app already holds and costs a few
//! integer counts per frame: no tmux, no filesystem, no sorting.

use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Padding, Paragraph},
};
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::fork::badges::BadgeKind;
use crate::theme::Theme;

/// The working directory shown in the header. AIW never changes its cwd, so
/// it is read once instead of with a `getcwd` per frame and per click.
pub fn cwd_label() -> &'static str {
    static CWD: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    CWD.get_or_init(|| {
        std::env::current_dir()
            .map(|p| crate::app::util::shorten_path(&p))
            .unwrap_or_default()
    })
}

/// A rounded, quietly bordered panel with a bold title — the dashboard's one
/// panel style.
pub(crate) fn panel<'a>(title: &'a str, theme: &Theme) -> Block<'a> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border.to_color()))
        .title(Span::styled(
            title,
            Style::default()
                .fg(theme.primary.to_color())
                .add_modifier(Modifier::BOLD),
        ))
}

/// The same panel with one column of padding on each side.
pub(crate) fn padded_panel<'a>(title: &'a str, theme: &Theme) -> Block<'a> {
    panel(title, theme).padding(Padding::horizontal(1))
}

/// A rounded panel with no title (header, footer).
pub(crate) fn plain_panel(theme: &Theme) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border.to_color()))
}

/// Key in a key hint: the single accent colour, bold.
pub(crate) fn key_style(theme: &Theme) -> Style {
    Style::default()
        .fg(theme.primary.to_color())
        .add_modifier(Modifier::BOLD)
}

/// Label in a key hint and any other metadata: dim.
pub(crate) fn dim_style(theme: &Theme) -> Style {
    Style::default().fg(theme.text_muted.to_color())
}

/// Tree guide lines (`├─`, `│`, `└─`): chrome, so the border colour.
pub(crate) fn guide_style(theme: &Theme) -> Style {
    Style::default().fg(theme.border.to_color())
}

/// The header's status pills: `⛔ needs you · ● working · ✓ finished` — the
/// badge model's own glyphs and tones (DESIGN.md §15).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HeaderPills {
    /// Questions, generic waits and pending inputs nothing explained — the
    /// same things the `i` overlay lists, minus finished turns.
    pub needs_you: usize,
    /// Running features whose agent is working right now.
    pub working: usize,
    /// Finished turns awaiting review, plus features marked ready.
    pub done: usize,
}

const PILL_SEPARATOR: &str = " · ";

impl HeaderPills {
    /// Count the pills from the badge model (DESIGN.md §15): every feature's
    /// roll-up badge, so the pills are the project rows' counts summed and
    /// the sidebar, the tree and the header can never disagree.
    pub fn from_app(app: &App) -> Self {
        let counts = crate::fork::badges::total_counts(app, chrono::Utc::now());
        Self {
            needs_you: counts.needs_you,
            working: counts.working,
            done: counts.done,
        }
    }

    fn parts(&self) -> [(BadgeKind, usize); 3] {
        [
            (BadgeKind::NeedsYou, self.needs_you),
            (BadgeKind::Working, self.working),
            (BadgeKind::FinishedUnread, self.done),
        ]
    }

    /// Rendered width in columns (`⛔` is two wide, the rest one).
    pub fn width(&self) -> u16 {
        let cells: usize = self
            .parts()
            .iter()
            .map(|(kind, count)| UnicodeWidthStr::width(kind.glyph()) + 1 + count.to_string().len())
            .sum();
        // glyph + space + digits per pill, separators between (three columns
        // each), two trailing spaces before the help hint.
        (cells + 2 * 3 + 2) as u16
    }

    /// The pills as coloured spans. A zero count stays visible but dim, so
    /// the row keeps its shape and reads as "nothing here" at a glance.
    pub fn spans(&self, theme: &Theme) -> Vec<Span<'static>> {
        let mut spans = Vec::with_capacity(6);
        for (index, (kind, count)) in self.parts().into_iter().enumerate() {
            if index > 0 {
                spans.push(Span::styled(PILL_SEPARATOR, guide_style(theme)));
            }
            let style = if count > 0 {
                Style::default()
                    .fg(kind.tone(theme))
                    .add_modifier(Modifier::BOLD)
            } else {
                dim_style(theme)
            };
            spans.push(Span::styled(format!("{} {count}", kind.glyph()), style));
        }
        spans.push(Span::raw("  "));
        spans
    }
}

/// The pills for the header, or `None` before there is any feature to count.
pub fn header_pills(app: &App) -> Option<HeaderPills> {
    app.store
        .projects
        .iter()
        .any(|project| !project.features.is_empty())
        .then(|| HeaderPills::from_app(app))
}

/// The designed "no projects yet" state, centred in the list panel.
pub(crate) fn draw_no_projects(frame: &mut Frame, area: Rect, theme: &Theme) {
    let block = padded_panel(" Projects ", theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let key = key_style(theme);
    let dim = dim_style(theme);
    let lines = vec![
        Line::from(Span::styled(
            "No projects yet",
            Style::default()
                .fg(theme.text.to_color())
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("Press ", dim),
            Span::styled("N", key),
            Span::styled(" to add a repository as your first project.", dim),
        ]),
        Line::from(Span::styled(
            "Each feature in it gets its own branch, worktree and agents.",
            dim,
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("N", key),
            Span::styled(" new project   ", dim),
            Span::styled("?", key),
            Span::styled(" help   ", dim),
            Span::styled("q", key),
            Span::styled(" quit", dim),
        ]),
    ];
    let height = lines.len() as u16;
    let top = inner.y + inner.height.saturating_sub(height) / 2;
    let text_area = Rect::new(inner.x, top, inner.width, height.min(inner.height));
    frame.render_widget(
        Paragraph::new(lines).alignment(Alignment::Center),
        text_area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::{MockTmuxOps, MockWorktreeOps};
    use crate::ui::fork_bench::realistic_app;
    use ratatui::{Terminal, backend::TestBackend};

    /// The rendered rows as text. A wide glyph (`⛔`) owns two cells; the
    /// continuation cell is skipped so the text reads as a terminal shows it.
    fn row_text<'a>(cells: impl Iterator<Item = &'a str>) -> String {
        let mut row = String::new();
        let mut skip = 0usize;
        for symbol in cells {
            if skip > 0 {
                skip -= 1;
                continue;
            }
            skip = UnicodeWidthStr::width(symbol).saturating_sub(1);
            row.push_str(symbol);
        }
        row
    }

    fn rows(app: &mut App, width: u16, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| crate::ui::draw(frame, app)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| row_text((0..width).map(|x| buffer[(x, y)].symbol())))
            .collect()
    }

    #[test]
    fn header_shows_status_pills_counted_from_app_state() {
        let mut app = realistic_app(MockTmuxOps::new());
        let screen = rows(&mut app, 200, 60);
        // One question; two working features; one finished turn plus three
        // features marked ready.
        assert!(screen[1].contains("⛔ 1 · ● 2 · ✓ 4"), "{}", screen[1]);
        assert!(
            screen[1].contains("[1 question | <leader i>]"),
            "{}",
            screen[1]
        );
        assert!(screen[1].ends_with("? help │"), "{}", screen[1]);
    }

    #[test]
    fn pills_give_way_before_the_badge_and_the_hit_box_follows_the_drawing() {
        let theme = Theme::default();
        let pills = Some(HeaderPills {
            needs_you: 1,
            working: 2,
            done: 3,
        });
        let cwd = "/home/dev/code/ai-workstation/.worktrees/some-long-branch";
        for width in [70u16, 90, 120, 200] {
            let mut terminal = Terminal::new(TestBackend::new(width, 3)).unwrap();
            terminal
                .draw(|frame| {
                    crate::ui::header::draw(
                        frame,
                        Rect::new(0, 0, width, 3),
                        cwd,
                        "0.37.0",
                        1,
                        pills,
                        &theme,
                    )
                })
                .unwrap();
            let buffer = terminal.backend().buffer();
            let row: Vec<&str> = (0..width).map(|x| buffer[(x, 1)].symbol()).collect();
            let columns = crate::ui::header::badge_hit_columns(
                Rect::new(0, 0, width, 3),
                cwd,
                "0.37.0",
                1,
                pills,
            )
            .expect("badge is clickable");
            let under_hit_box: String = row[columns.start as usize..columns.end as usize].concat();
            assert_eq!(
                under_hit_box, "  [1 question | <leader i>]",
                "width {width}: hit box and drawn badge disagree"
            );
            let line = row_text(row.iter().copied());
            assert_eq!(
                line.contains("⛔ 1 · ● 2 · ✓ 3"),
                width >= 90,
                "width {width}: {line}"
            );
        }
    }

    #[test]
    fn footer_shows_the_contextual_key_hints() {
        let mut app = realistic_app(MockTmuxOps::new());
        let screen = rows(&mut app, 200, 60);
        let footer = &screen[59];
        assert!(footer.starts_with("╰ n feature  Enter expand"), "{footer}");

        app.selection = crate::app::Selection::Session(0, 0, 1);
        let screen = rows(&mut app, 200, 60);
        assert!(
            screen[59].contains("Enter view  r rename"),
            "{}",
            screen[59]
        );
    }

    #[test]
    fn the_selected_row_gets_the_accent_bar_and_no_other_row_does() {
        let mut app = realistic_app(MockTmuxOps::new());
        let screen = rows(&mut app, 200, 60);
        let bars: Vec<&String> = screen.iter().filter(|row| row.starts_with("│▎")).collect();
        assert_eq!(bars.len(), 1, "{bars:?}");
        assert!(bars[0].contains("billing-13"), "{}", bars[0]);
    }

    #[test]
    fn an_empty_dashboard_explains_what_to_do() {
        let mut app = App::new_for_test(
            crate::project::ProjectStore::empty(),
            Box::new(MockTmuxOps::new()),
            Box::new(MockWorktreeOps::new()),
        );
        let screen = rows(&mut app, 120, 30).concat();
        assert!(screen.contains("No projects yet"));
        assert!(screen.contains("Press N to add a repository as your first project."));
        assert!(screen.contains("N new project"));
        // No features anywhere, so no pills to show.
        assert!(!screen.contains("⛔ 0"));
    }

    #[test]
    fn a_project_without_features_says_how_to_add_one() {
        let mut store = crate::ui::fork_bench::realistic_store();
        store.projects[2].features.clear();
        let mut app = App::new_for_test(
            store,
            Box::new(MockTmuxOps::new()),
            Box::new(MockWorktreeOps::new()),
        );
        let screen = rows(&mut app, 200, 60).concat();
        assert!(screen.contains("▾ workflow-engine  /home/dev/code/workflow-engine  · no features yet — press n to add one"));
    }

    /// Box-drawing glyphs that only a dialog border or a toast would leave on
    /// a dashboard row: anything but the panel's own corners at the edges.
    fn box_drawing_residue(screen: &[String]) -> Vec<(usize, String)> {
        let mut residue = Vec::new();
        for (y, row) in screen.iter().enumerate() {
            let chars: Vec<char> = row.chars().collect();
            let last = chars.len().saturating_sub(1);
            for (x, ch) in chars.iter().enumerate() {
                // `└` is also a tree guide, which sits in the first columns.
                let corner =
                    matches!(ch, '┌' | '┐' | '┘' | '╭' | '╮' | '╰' | '╯') || (*ch == '└' && x > 10);
                if corner && x != 0 && x != last {
                    residue.push((y, row.clone()));
                    break;
                }
            }
        }
        residue
    }

    #[test]
    fn closing_a_dialog_leaves_no_border_residue_on_the_dashboard() {
        let mut app = realistic_app(MockTmuxOps::new());
        let mut terminal = Terminal::new(TestBackend::new(200, 60)).unwrap();
        let read = |terminal: &Terminal<TestBackend>| -> Vec<String> {
            let buffer = terminal.backend().buffer();
            (0..60)
                .map(|y| (0..200).map(|x| buffer[(x, y)].symbol()).collect())
                .collect()
        };
        terminal
            .draw(|frame| crate::ui::draw(frame, &mut app))
            .unwrap();
        // Dialogs of several shapes, each followed by the plain dashboard.
        let dialogs: Vec<crate::app::AppMode> = vec![
            crate::app::AppMode::Help(crate::app::HelpState {
                from_view: None,
                scroll_offset: 0,
            }),
            crate::app::AppMode::DeletingFeature("harbor-api".into(), "harbor-12".into()),
            crate::app::AppMode::DeletingProject("harbor-api".into()),
            crate::app::AppMode::NotificationPicker(0, None),
        ];
        for dialog in dialogs {
            app.mode = dialog;
            terminal
                .draw(|frame| crate::ui::draw(frame, &mut app))
                .unwrap();
            app.mode = crate::app::AppMode::Normal;
            app.message = Some("Deleted feature 'harbor-12'".into());
            terminal
                .draw(|frame| crate::ui::draw(frame, &mut app))
                .unwrap();
            let screen = read(&terminal);
            assert!(
                box_drawing_residue(&screen).is_empty(),
                "residue after a dialog: {:?}",
                box_drawing_residue(&screen)
            );
            assert!(
                screen[58].starts_with("│ Deleted feature 'harbor-12'"),
                "{}",
                screen[58]
            );
        }
    }

    #[test]
    fn a_toast_sits_inside_the_list_panel_not_across_its_border() {
        let mut app = realistic_app(MockTmuxOps::new());
        app.push_toast_success("Deleted feature 'harbor-12'");
        let screen = rows(&mut app, 200, 60);
        // The panels' own borders are intact on every row...
        for (y, row) in screen.iter().enumerate() {
            // `⛔` in the header owns two columns, so count from both ends
            // rather than by a fixed index.
            let first = row.chars().next();
            let last = row.chars().last();
            assert!(
                matches!(first, Some('│' | '╭' | '╰')) && matches!(last, Some('│' | '╮' | '╯')),
                "row {y}: {row}"
            );
        }
        assert!(
            screen[56].chars().all(|c| c == '─' || c == '╰' || c == '╯'),
            "{}",
            screen[56]
        );
        // ...and the toast box is whole, inside the list panel.
        let top = screen
            .iter()
            .position(|row| row.contains("╭ ✓ "))
            .expect("toast top border");
        assert!(top > 4 && top < 56, "toast at row {top}");
        assert!(
            screen[top + 1].contains("Deleted feature 'harbor-12'"),
            "{}",
            screen[top + 1]
        );
        assert!(screen[top + 2].contains("╰─"), "{}", screen[top + 2]);
        assert!(screen[top + 2].ends_with("╯ │"), "{}", screen[top + 2]);
    }

    #[test]
    fn pill_width_matches_the_rendered_spans() {
        let theme = Theme::default();
        for pills in [
            HeaderPills::default(),
            HeaderPills {
                needs_you: 2,
                working: 16,
                done: 1,
            },
            HeaderPills {
                needs_you: 120,
                working: 0,
                done: 3,
            },
        ] {
            let rendered: usize = pills
                .spans(&theme)
                .iter()
                .map(|span| unicode_width::UnicodeWidthStr::width(span.content.as_ref()))
                .sum();
            assert_eq!(pills.width() as usize, rendered, "{pills:?}");
        }
    }
}
