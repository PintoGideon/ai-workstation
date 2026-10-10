//! FORK: text fitting for the dashboard chrome (DESIGN.md §12): one way to
//! cut a styled line with `…`, to drop key hints whole instead of mid-word,
//! to scroll an input field and to wrap a path at `/`.
//!
//! All widths are display columns (`unicode-width`), so a wide glyph or an
//! emoji at the cut never spills over a border.

use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// The cut marker: one column.
pub(crate) const ELLIPSIS: &str = "…";

/// Keep at most `width` columns of `line`; a line that was cut ends in `…`
/// (in the style of the span it was cut from). A wide glyph that would
/// straddle the cut is dropped and its column padded.
pub(crate) fn fit_line<'a>(line: Line<'a>, width: usize) -> Line<'a> {
    let total: usize = line.spans.iter().map(Span::width).sum();
    if total <= width {
        return line;
    }
    if width == 0 {
        return Line::default()
            .style(line.style)
            .alignment(line.alignment.unwrap_or_default());
    }
    let budget = width - 1;
    let mut used = 0usize;
    let mut spans = Vec::with_capacity(line.spans.len() + 1);
    for span in line.spans {
        let span_width = span.width();
        if used + span_width <= budget {
            used += span_width;
            spans.push(span);
            continue;
        }
        let remaining = budget - used;
        let style = span.style;
        let mut kept = String::new();
        let mut kept_width = 0usize;
        for ch in span.content.chars() {
            let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if kept_width + ch_width > remaining {
                break;
            }
            kept.push(ch);
            kept_width += ch_width;
        }
        if kept_width < remaining {
            kept.extend(std::iter::repeat_n(' ', remaining - kept_width));
        }
        if !kept.is_empty() {
            spans.push(Span::styled(kept, style));
        }
        spans.push(Span::styled(ELLIPSIS, style));
        break;
    }
    let mut fitted = Line::from(spans).style(line.style);
    if let Some(alignment) = line.alignment {
        fitted = fitted.alignment(alignment);
    }
    fitted
}

/// Fit a row of `key label  key label …` hints into `width` columns by
/// dropping whole hints from the end (never mid-word), then appending `…`.
/// `is_key` tells a key span from a label span; a hint starts at a key span.
pub(crate) fn fit_hints<'a>(
    line: Line<'a>,
    width: usize,
    is_key: impl Fn(&Span<'a>) -> bool,
) -> Line<'a> {
    let total: usize = line.spans.iter().map(Span::width).sum();
    if total <= width {
        return line;
    }
    if width == 0 {
        return Line::default().style(line.style);
    }
    // Width of the first `count` spans with the last one's trailing blanks
    // trimmed, which is how a kept prefix is shown before ` …`.
    let trimmed_width = |count: usize| -> usize {
        let spans = &line.spans[..count];
        let Some((last, head)) = spans.split_last() else {
            return 0;
        };
        head.iter().map(Span::width).sum::<usize>()
            + UnicodeWidthStr::width(last.content.trim_end())
    };
    let mut cut_at = 0usize;
    for (index, span) in line.spans.iter().enumerate() {
        if index > 0 && is_key(span) && trimmed_width(index) + 2 <= width {
            cut_at = index;
        }
    }
    let style = line
        .spans
        .get(cut_at)
        .map(|span| span.style)
        .unwrap_or_default();
    let mut spans: Vec<Span<'a>> = line.spans.into_iter().take(cut_at).collect();
    let marker = if let Some(last) = spans.last_mut() {
        last.content = last.content.trim_end().to_string().into();
        format!(" {ELLIPSIS}")
    } else {
        ELLIPSIS.to_string()
    };
    spans.push(Span::styled(marker, style));
    Line::from(spans).style(line.style)
}

/// Keep at most `width` columns of `text`; a text that was cut ends in `…`.
/// The plain-string form of [`fit_line`], for a cell that is built before
/// it is styled. Never splits a character, so a multibyte or wide glyph at
/// the cut is dropped rather than sliced.
pub(crate) fn fit_text(text: &str, width: usize) -> String {
    if UnicodeWidthStr::width(text) <= width {
        return text.to_string();
    }
    fit_line(Line::from(text), width)
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

/// The last `width` columns of `text`, marked with a leading `…` when the
/// front was dropped — for an input field that must keep its end (and the
/// cursor after it) in view.
pub(crate) fn elide_head(text: &str, width: usize) -> String {
    if UnicodeWidthStr::width(text) <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    if width == 1 {
        return ELLIPSIS.to_string();
    }
    let budget = width - 1;
    let mut tail: Vec<char> = Vec::new();
    let mut used = 0usize;
    for ch in text.chars().rev() {
        let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if used + ch_width > budget {
            break;
        }
        tail.push(ch);
        used += ch_width;
    }
    tail.reverse();
    let mut shown = ELLIPSIS.to_string();
    shown.extend(tail);
    shown
}

/// Wrap a path to `width` columns, breaking after a `/` wherever one
/// fits; a single component longer than the width is cut hard. Every line
/// after the first is meant for a hanging indent, which the caller adds.
pub(crate) fn wrap_path(path: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;
    // Components keep their trailing `/`, so a line never ends mid-name.
    let mut component = String::new();
    let flush_component = |component: &mut String,
                           current: &mut String,
                           current_width: &mut usize,
                           lines: &mut Vec<String>| {
        let component_width = UnicodeWidthStr::width(component.as_str());
        if *current_width + component_width > width && !current.is_empty() {
            lines.push(std::mem::take(current));
            *current_width = 0;
        }
        if component_width > width {
            for ch in component.chars() {
                let ch_width = UnicodeWidthChar::width(ch).unwrap_or(0);
                if *current_width + ch_width > width && !current.is_empty() {
                    lines.push(std::mem::take(current));
                    *current_width = 0;
                }
                current.push(ch);
                *current_width += ch_width;
            }
        } else {
            current.push_str(component);
            *current_width += component_width;
        }
        component.clear();
    };
    for ch in path.chars() {
        component.push(ch);
        if ch == '/' {
            flush_component(&mut component, &mut current, &mut current_width, &mut lines);
        }
    }
    if !component.is_empty() {
        flush_component(&mut component, &mut current, &mut current_width, &mut lines);
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Style};

    fn text(line: &Line) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn a_line_that_fits_is_untouched() {
        let line = Line::from(vec![Span::raw("abc"), Span::raw("def")]);
        assert_eq!(text(&fit_line(line, 6)), "abcdef");
    }

    #[test]
    fn a_long_line_is_cut_with_an_ellipsis_in_the_cut_spans_style() {
        let red = Style::default().fg(Color::Red);
        let line = Line::from(vec![Span::raw("abc"), Span::styled("defgh", red)]);
        let fitted = fit_line(line, 6);
        assert_eq!(text(&fitted), "abcde…");
        assert_eq!(fitted.spans.last().unwrap().style, red);
        assert_eq!(fitted.width(), 6);
    }

    #[test]
    fn a_wide_glyph_never_straddles_the_cut() {
        let line = Line::from("ab🚀cd");
        for width in 1..6 {
            let fitted = fit_line(line.clone(), width);
            assert_eq!(fitted.width(), width, "width {width}: {:?}", text(&fitted));
            assert!(text(&fitted).ends_with(ELLIPSIS));
        }
        assert_eq!(text(&fit_line(line.clone(), 3)), "ab…");
        assert_eq!(text(&fit_line(line.clone(), 4)), "ab …");
        assert_eq!(text(&fit_line(line, 0)), "");
    }

    #[test]
    fn fit_text_cuts_on_columns_not_bytes() {
        assert_eq!(fit_text("abc", 3), "abc");
        assert_eq!(fit_text("abcdef", 4), "abc…");
        // Every character here is multibyte; a byte slice at the width would
        // land inside one of them.
        let message = "Привет, нужно разрешение на запуск команды";
        let fitted = fit_text(message, 20);
        assert_eq!(UnicodeWidthStr::width(fitted.as_str()), 20);
        assert!(fitted.ends_with(ELLIPSIS));
        assert_eq!(fit_text("ab🚀cd", 4), "ab …");
    }

    #[test]
    fn hints_are_dropped_whole_not_mid_word() {
        let key = Style::default().fg(Color::Yellow);
        let is_key = |span: &Span| span.style == key;
        let line = Line::from(vec![
            Span::styled(" n", key),
            Span::raw(" feature  "),
            Span::styled("Enter", key),
            Span::raw(" expand  "),
            Span::styled("V", key),
            Span::raw(" pending review  "),
            Span::styled("q", key),
            Span::raw(" quit"),
        ]);
        assert_eq!(
            text(&fit_hints(line.clone(), 80, is_key)),
            " n feature  Enter expand  V pending review  q quit"
        );
        let fitted = fit_hints(line.clone(), 30, is_key);
        assert_eq!(text(&fitted), " n feature  Enter expand …");
        assert!(fitted.width() <= 30);
        let fitted = fit_hints(line.clone(), 12, is_key);
        assert_eq!(text(&fitted), " n feature …");
        assert_eq!(text(&fit_hints(line.clone(), 11, is_key)), "…");
        // Too narrow for even one hint: just the marker.
        assert_eq!(text(&fit_hints(line, 5, is_key)), "…");
    }

    #[test]
    fn an_input_keeps_its_tail_in_view() {
        assert_eq!(elide_head("/home/dev/code", 20), "/home/dev/code");
        assert_eq!(elide_head("/home/dev/code", 8), "…ev/code");
        assert_eq!(elide_head("a🚀b", 2), "…b");
        assert_eq!(elide_head("abc", 1), "…");
        assert_eq!(elide_head("abc", 0), "");
    }

    #[test]
    fn paths_wrap_after_slashes() {
        let path = "/home/dev/.local/share/ai_workstation/worktrees/alpha/alpha-auth";
        let lines = wrap_path(path, 26);
        assert_eq!(
            lines,
            vec![
                "/home/dev/.local/share/",
                "ai_workstation/worktrees/",
                "alpha/alpha-auth"
            ]
        );
        for line in &lines {
            assert!(UnicodeWidthStr::width(line.as_str()) <= 26);
        }
        assert_eq!(lines.concat(), path);
        // A component wider than the box is cut hard rather than lost.
        let long = "/x/".to_string() + &"y".repeat(30);
        let lines = wrap_path(&long, 10);
        assert_eq!(lines.concat(), long);
        assert!(
            lines
                .iter()
                .all(|line| UnicodeWidthStr::width(line.as_str()) <= 10)
        );
        assert_eq!(wrap_path("", 10), vec![""]);
    }
}
