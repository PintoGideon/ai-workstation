//! FORK: the permission detail overlay and its key hints (P6).

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::app::App;
use crate::app::fork_permissions::PermissionDetailState;
use crate::fork::permissions::Harness;
use crate::handlers::fork_permissions::{ALLOW_KEY, DENY_KEY, PASS_KEY};
use crate::theme::Theme;

/// The footer line while the overlay is up.
pub(crate) fn detail_hints(theme: &Theme) -> Line<'static> {
    let key = super::fork_chrome::key_style(theme);
    Line::from(vec![
        Span::styled(format!(" {ALLOW_KEY}"), key),
        Span::raw(" allow  "),
        Span::styled(DENY_KEY.to_string(), key),
        Span::raw(" deny  "),
        Span::styled(PASS_KEY.to_string(), key),
        Span::raw(" let the agent ask  "),
        Span::styled("o", key),
        Span::raw(" open the session  "),
        Span::styled("Esc", key),
        Span::raw(" back"),
    ])
}

/// `a allow  X deny` for a row with a request waiting.
pub(crate) fn row_hint_spans(theme: &Theme) -> Vec<Span<'static>> {
    let key = super::fork_chrome::key_style(theme);
    vec![
        Span::styled(format!(" {ALLOW_KEY}"), key),
        Span::raw(" allow  "),
        Span::styled(DENY_KEY.to_string(), key),
        Span::raw(" deny  "),
    ]
}

/// The overlay body: where, what, and the full input.
pub(crate) fn body_lines(
    app: &App,
    state: &PermissionDetailState,
    width: usize,
) -> Vec<Line<'static>> {
    let theme = &app.theme;
    let label = Style::default().fg(theme.text_muted.to_color());
    let value = Style::default().fg(theme.text.to_color());
    let strong = Style::default()
        .fg(theme.warning.to_color())
        .add_modifier(Modifier::BOLD);
    let Some(pending) = app.pending_permission_by_id(&state.request_id) else {
        return vec![Line::from(Span::styled(
            "This request was answered or the agent stopped waiting.",
            label,
        ))];
    };
    let location = app.pending_permission_location(pending);
    let where_text = location
        .and_then(|(pi, fi, si)| {
            let project = app.store.projects.get(pi)?;
            let feature = project.features.get(fi)?;
            let session = si
                .and_then(|si| feature.sessions.get(si))
                .map(|s| s.label.clone())
                .unwrap_or_else(|| pending.harness.as_str().to_string());
            Some(format!("{} › {} › {}", project.name, feature.name, session))
        })
        .unwrap_or_else(|| pending.tmux_session.clone());
    let harness = match pending.harness {
        Harness::Claude => "Claude Code",
        Harness::Codex => "Codex",
        Harness::Cursor => "Cursor agent",
    };
    let age =
        crate::fork::badges::compact_elapsed((chrono::Utc::now() - pending.since).num_seconds());
    let mut lines = vec![
        Line::from(vec![
            Span::styled("Session   ", label),
            Span::styled(where_text, value),
        ]),
        Line::from(vec![
            Span::styled("Harness   ", label),
            Span::styled(format!("{harness} · waiting {age}"), value),
        ]),
        Line::from(vec![
            Span::styled("Tool      ", label),
            Span::styled(pending.tool_name.clone(), strong),
        ]),
        Line::from(""),
        Line::from(Span::styled("Input", label)),
    ];
    let input = match &pending.tool_input {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => s.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    };
    let fit = |line: Line<'static>| super::fork_text::fit_line(line, width);
    if input.is_empty() {
        lines.push(Line::from(Span::styled("  (no input)", label)));
    }
    for raw in input.lines().take(40) {
        lines.push(fit(Line::from(Span::styled(format!("  {raw}"), value))));
    }
    let total = input.lines().count();
    if total > 40 {
        lines.push(Line::from(Span::styled(
            format!("  … {} more lines", total - 40),
            label,
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Allowing lets the agent run this once; denying tells it the dashboard refused. The agent waits until you answer, or shows its own prompt when the wait runs out.",
        label,
    )));
    lines
}

pub(crate) fn draw_detail(frame: &mut Frame, app: &App, state: &PermissionDetailState) {
    let theme = &app.theme;
    let width = 90.min(frame.area().width);
    let body = body_lines(app, state, usize::from(width.saturating_sub(4)));
    let height = (body.len() as u16).saturating_add(5).clamp(10, 36);
    let area = super::dialog_rect(frame.area(), width, height);
    super::draw_modal_overlay(frame, area, theme);

    let block = Block::default()
        .title(" Permission request ")
        .borders(Borders::ALL)
        .border_type(ratatui::widgets::BorderType::Rounded)
        .style(Style::default().bg(theme.effective_bg()))
        .border_style(Style::default().fg(theme.status_waiting.to_color()));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);
    frame.render_widget(Paragraph::new(body).wrap(Wrap { trim: false }), chunks[0]);
    frame.render_widget(Paragraph::new(detail_hints(theme)), chunks[1]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::AppMode;
    use crate::project::SessionKind;
    use serde_json::json;

    #[test]
    fn the_overlay_names_the_session_tool_and_full_input() {
        let mut app = crate::app::fork_workspace::workspace_app(&[SessionKind::Claude]);
        let session_id = app.store.projects[0].features[0].sessions[0].id.clone();
        assert!(app.handle_permission_request_message(&json!({
            "type": "permission_request", "request_id": "r1", "harness": "claude",
            "aiw_session": "aiw-hx-12", "aiw_feature_session_id": session_id, "cwd": "/tmp",
            "tool_name": "Bash", "tool_input": {"command": "git push origin main", "description": "push"}
        })));
        app.open_permission_detail("r1");
        let AppMode::PermissionDetail(state) = &app.mode else {
            panic!("overlay");
        };
        let text: String = body_lines(&app, state.as_ref(), 80)
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("RS › hx-12 › Claude 1"), "{text}");
        assert!(text.contains("Bash"));
        assert!(text.contains("git push origin main"));
        assert!(text.contains("\"description\": \"push\""));

        let backend = ratatui::backend::TestBackend::new(120, 40);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                if let AppMode::PermissionDetail(state) = &app.mode {
                    draw_detail(frame, &app, state.as_ref());
                }
            })
            .unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(rendered.contains("Permission request"));
    }
}
