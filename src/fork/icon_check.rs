//! First-run icon check: shows a few Nerd Font glyphs and asks whether they
//! render. The answer becomes `nerd_font` (icons vs ASCII fallbacks), so a
//! terminal without a Nerd Font never shows rows of empty boxes.
//!
//! Runs once, before the dashboard's event loop, as a small modal of its own
//! so it needs no new `AppMode`.

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::{
    Frame, Terminal,
    backend::Backend,
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
};

use crate::app::App;
use crate::theme::Theme;

/// The sample: glyphs the dashboard actually uses, each one cell wide.
pub const SAMPLE: [(&str, &str); 5] = [
    ("\u{f059}", "question"),
    ("\u{f06e}", "review"),
    ("\u{e0a0}", "branch"),
    ("\u{f0ae}", "todos"),
    ("\u{e6ae}", "nvim"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answer {
    Icons,
    Ascii,
    Later,
}

pub fn answer_for(key: KeyCode) -> Option<Answer> {
    match key {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => Some(Answer::Icons),
        KeyCode::Char('n') | KeyCode::Char('N') => Some(Answer::Ascii),
        KeyCode::Esc | KeyCode::Char('q') => Some(Answer::Later),
        _ => None,
    }
}

/// Store the answer: `nerd_font` plus the `nerd_font_confirmed` marker
/// (`false` for "ask me later", so the check comes back next start).
pub fn record(app: &mut App, answer: Answer) {
    match answer {
        Answer::Icons => {
            app.config.nerd_font = true;
            app.config.appearance.nerd_font_confirmed = Some(true);
        }
        Answer::Ascii => {
            app.config.nerd_font = false;
            app.config.appearance.nerd_font_confirmed = Some(true);
        }
        Answer::Later => app.config.appearance.nerd_font_confirmed = Some(false),
    }
    app.save_config();
}

/// Show the check and wait for y / n / Esc.
pub fn run<B: Backend>(terminal: &mut Terminal<B>, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|frame| draw(frame, &app.theme))?;
        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
            && let Some(answer) = answer_for(key.code)
        {
            record(app, answer);
            return Ok(());
        }
    }
}

pub fn draw(frame: &mut Frame, theme: &Theme) {
    let full = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().bg(theme.effective_bg())),
        full,
    );

    let width = 58.min(full.width);
    let height = 14.min(full.height);
    let area = Rect {
        x: full.x + (full.width - width) / 2,
        y: full.y + (full.height - height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, area);

    let text = Style::default().fg(theme.text.to_color());
    let muted = Style::default().fg(theme.text_muted.to_color());
    let key = Style::default()
        .fg(theme.primary.to_color())
        .add_modifier(Modifier::BOLD);

    let mut glyphs = Vec::new();
    let mut labels = Vec::new();
    for (glyph, label) in SAMPLE {
        // Each column is 10 cells wide; the glyph sits in the middle.
        glyphs.push(Span::styled(
            format!("{:^10}", glyph),
            Style::default()
                .fg(theme.text.to_color())
                .add_modifier(Modifier::BOLD),
        ));
        labels.push(Span::styled(format!("{label:^10}"), muted));
    }

    let lines = vec![
        Line::from(""),
        Line::from(Span::styled("Do you see five small icons here?", text)),
        Line::from(""),
        Line::from(glyphs),
        Line::from(labels),
        Line::from(""),
        Line::from(Span::styled(
            "Empty boxes, question marks or blanks mean no.",
            muted,
        )),
        Line::from(Span::styled(
            "You can change this later: nerd_font in config.json.",
            muted,
        )),
        Line::from(""),
        Line::from(vec![
            Span::styled("y", key),
            Span::styled(" yes, use icons    ", text),
            Span::styled("n", key),
            Span::styled(" no, plain text    ", text),
            Span::styled("Esc", key),
            Span::styled(" later", text),
        ]),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_focus.to_color()))
        .style(Style::default().bg(theme.effective_bg()))
        .title(Span::styled(
            " AI Workstation: icon check ",
            Style::default()
                .fg(theme.text.to_color())
                .add_modifier(Modifier::BOLD),
        ));
    frame.render_widget(
        Paragraph::new(lines)
            .alignment(Alignment::Center)
            .block(block),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    /// Every Nerd Font glyph the TUI draws, paired with its ASCII fallback.
    /// The glyph must be one cell wide (so columns line up whether or not the
    /// font has it) and the fallback must be plain ASCII.
    const ICONS: &[(&str, &str)] = &[
        // attention states (app/attention.rs)
        ("\u{f059}", "?"),
        ("\u{f06e}", "*"),
        ("\u{f254}", "-"),
        // session kinds (ui/list.rs, ui/picker.rs)
        ("\u{e6ae}", "~"),
        ("\u{E70C}", "V"),
        ("\u{f0ae}", "="),
        // TODO markers (ui/dialogs/todos.rs)
        ("\u{f036}", "="),
        ("\u{f135}", ">"),
        ("\u{e0a0}", "Y"),
    ];

    #[test]
    fn icons_are_single_width_with_ascii_fallbacks() {
        for (glyph, fallback) in ICONS {
            assert_eq!(glyph.width(), 1, "{glyph:?} must be one cell wide");
            assert!(fallback.is_ascii() && fallback.width() == 1, "{fallback:?}");
        }
        for (glyph, _) in SAMPLE {
            assert_eq!(glyph.width(), 1, "{glyph:?}");
        }
        for icon in crate::custom_session_icons::CUSTOM_SESSION_ICONS {
            assert_eq!(icon.glyph.width(), 1, "{}", icon.nerd_font_name);
        }
    }

    #[test]
    fn attention_glyphs_match_the_audit() {
        use crate::app::attention::AttentionState;
        for state in [
            AttentionState::Question,
            AttentionState::CompletedAwaitingReview,
            AttentionState::Waiting,
        ] {
            let pair = (state.glyph(true), state.glyph(false));
            assert!(
                ICONS.contains(&pair),
                "{state:?} glyphs not audited: {pair:?}"
            );
        }
    }

    #[test]
    fn keys_map_to_answers() {
        assert_eq!(answer_for(KeyCode::Char('y')), Some(Answer::Icons));
        assert_eq!(answer_for(KeyCode::Enter), Some(Answer::Icons));
        assert_eq!(answer_for(KeyCode::Char('n')), Some(Answer::Ascii));
        assert_eq!(answer_for(KeyCode::Esc), Some(Answer::Later));
        assert_eq!(answer_for(KeyCode::Char('x')), None);
    }

    #[test]
    fn dialog_renders_the_sample() {
        let backend = ratatui::backend::TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = Theme::load(&crate::theme::ThemeName::Ember);
        terminal.draw(|frame| draw(frame, &theme)).unwrap();
        let screen: String = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(screen.contains("icon check"));
        assert!(screen.contains("\u{f059}"));
        assert!(screen.contains("plain text"));
    }
}
