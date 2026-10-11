//! FORK: drawing the session info block (DESIGN.md §18): the bottom rows of
//! the cockpit sidebar, describing the selected session from
//! `App::session_info_for`. Store and cache lookups only; nothing here reads
//! a file or calls tmux.

use chrono::Utc;
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::app::App;
use crate::app::fork_session_info::SessionInfo;
use crate::context_tracking::ContextBand;
use crate::theme::Theme;

use super::fork_text::fit_line;

/// Rows the block takes, border included.
pub const HEIGHT: u16 = 8;
/// Sidebars shorter than this keep every row for the list.
pub const MIN_SIDEBAR_ROWS: u16 = 24;

/// The sidebar rows the block takes for a sidebar `height` rows tall, with
/// a session to describe: `HEIGHT` from `MIN_SIDEBAR_ROWS` up, else none.
pub fn rows(height: u16, has_target: bool) -> u16 {
    if has_target && height >= MIN_SIDEBAR_ROWS {
        HEIGHT
    } else {
        0
    }
}

fn context_span(pct: u8, band: ContextBand, theme: &Theme, dim: Style) -> Span<'static> {
    let style = match band {
        ContextBand::Normal => dim,
        ContextBand::Warning => Style::default().fg(theme.warning.to_color()),
        ContextBand::Critical => Style::default()
            .fg(theme.danger.to_color())
            .add_modifier(Modifier::BOLD),
    };
    Span::styled(format!("ctx {pct}%"), style)
}

/// The block's lines (title first) for `info`, each fitted to `width`.
pub fn lines(info: &SessionInfo, theme: &Theme, width: usize) -> Vec<Line<'static>> {
    let dim = super::fork_chrome::dim_style(theme);
    let text = Style::default().fg(theme.text.to_color());
    let mut out = Vec::with_capacity(6);
    out.push(fit_line(
        Line::from(Span::styled(
            info.title.clone(),
            text.add_modifier(Modifier::BOLD),
        )),
        width,
    ));
    let mut harness = vec![Span::styled(info.harness.clone(), text)];
    if let Some(model) = &info.model {
        harness.push(Span::styled(" · ", dim));
        harness.push(Span::styled(model.clone(), dim));
    }
    out.push(fit_line(Line::from(harness), width));
    let mut numbers: Vec<Span<'static>> = Vec::new();
    if let Some((pct, band)) = info.context {
        numbers.push(context_span(pct, band, theme, dim));
    }
    if let Some(cost) = &info.cost {
        if !numbers.is_empty() {
            numbers.push(Span::styled(" · ", dim));
        }
        numbers.push(Span::styled(cost.clone(), dim));
    }
    if !numbers.is_empty() {
        numbers.push(Span::styled(" · ", dim));
    }
    numbers.push(Span::styled(format!("{} old", info.age), dim));
    out.push(fit_line(Line::from(numbers), width));
    out.push(fit_line(
        Line::from(vec![
            Span::styled("⎇ ", dim),
            Span::styled(
                info.branch.clone(),
                Style::default().fg(theme.feature_title.to_color()),
            ),
        ]),
        width,
    ));
    out.push(fit_line(
        Line::from(match &info.worktree {
            Some(path) => vec![Span::styled(path.clone(), dim)],
            None => vec![Span::styled("in the repo checkout", dim)],
        }),
        width,
    ));
    let mut last: Vec<Span<'static>> = Vec::new();
    if info.queued > 0 {
        last.push(Span::styled(
            format!("⏭ {} queued", info.queued),
            Style::default().fg(theme.primary.to_color()),
        ));
    }
    if let Some(decision) = &info.decision {
        if !last.is_empty() {
            last.push(Span::styled(" · ", dim));
        }
        last.push(Span::styled(decision.clone(), dim));
    }
    if last.is_empty() {
        last.push(Span::styled("no follow-ups queued", dim));
    }
    out.push(fit_line(Line::from(last), width));
    out
}

/// Draw the block for the session at `(pi, fi, si)` into `area`.
pub fn draw(frame: &mut Frame, app: &App, area: Rect, target: (usize, usize, usize)) {
    let (pi, fi, si) = target;
    let Some(info) = app.session_info_for(pi, fi, si, Utc::now()) else {
        return;
    };
    let theme = &app.theme;
    let title = format!(" {} ", info.label);
    let block = super::fork_chrome::padded_panel(&title, theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let width = usize::from(inner.width);
    let mut lines = lines(&info, theme, width);
    lines.truncate(usize::from(inner.height));
    frame.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_block_only_takes_rows_from_a_tall_enough_sidebar_with_a_session() {
        assert_eq!(rows(23, true), 0);
        assert_eq!(rows(24, true), HEIGHT);
        assert_eq!(rows(60, false), 0);
    }

    #[test]
    fn the_lines_say_what_is_known_and_fit_the_width() {
        let theme = Theme::default();
        let info = SessionInfo {
            label: "Claude 1".into(),
            title: "Fix the login redirect loop in the SSO flow for every tenant".into(),
            harness: "Claude Code".into(),
            model: Some("Opus 4".into()),
            context: Some((42, ContextBand::Normal)),
            cost: Some("$0.84".into()),
            age: "12m".into(),
            branch: "hx-12-sso".into(),
            worktree: Some("…/RS/hx-12".into()),
            queued: 2,
            decision: Some("✓ allowed Bash".into()),
        };
        let lines = super::lines(&info, &theme, 30);
        let text: Vec<String> = lines
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect();
        assert_eq!(lines.len(), 6);
        assert!(
            text[0].starts_with("Fix the login redirect loop"),
            "{text:?}"
        );
        assert!(text[0].ends_with('…'), "elided: {text:?}");
        assert_eq!(text[1], "Claude Code · Opus 4");
        assert_eq!(text[2], "ctx 42% · $0.84 · 12m old");
        assert_eq!(text[3], "⎇ hx-12-sso");
        assert_eq!(text[4], "…/RS/hx-12");
        assert_eq!(text[5], "⏭ 2 queued · ✓ allowed Bash");
        assert!(lines.iter().all(|l| {
            unicode_width::UnicodeWidthStr::width(
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
                    .as_str(),
            ) <= 30
        }));
        let bare = SessionInfo {
            model: None,
            context: None,
            cost: None,
            worktree: None,
            queued: 0,
            decision: None,
            ..info
        };
        let text: Vec<String> = super::lines(&bare, &theme, 40)
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect();
        assert_eq!(text[1], "Claude Code");
        assert_eq!(text[2], "12m old");
        assert_eq!(text[4], "in the repo checkout");
        assert_eq!(text[5], "no follow-ups queued");
    }
}
