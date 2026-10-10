//! FORK: tests for `crate::fork::session_reconcile` (sessions whose tmux window
//! closed on its own, or whose agent exited at the shell prompt).

use super::support::*;
use crate::app::attention::{AttentionSource, AttentionState};
use crate::app::*;
use crate::fork::session_reconcile::{EXITED_STATUS, selection_after_removal};
use crate::project::{AgentKind, FeatureSession, SessionKind};
use crate::traits::{MockTmuxOps, MockWorktreeOps};

fn session(kind: SessionKind, window: &str) -> FeatureSession {
    let mut s = make_session(window, None);
    s.kind = kind;
    s.created_at = chrono::Utc::now() - chrono::Duration::minutes(5);
    s
}

/// `panes`: `(session, window, pane_current_command)`; a window not listed
/// is gone.
fn app_with(sessions: Vec<FeatureSession>, panes: &[(&str, &str, &str)]) -> App {
    app_with_tmux(sessions, panes, MockTmuxOps::new())
}

fn app_with_tmux(
    sessions: Vec<FeatureSession>,
    panes: &[(&str, &str, &str)],
    mut tmux: MockTmuxOps,
) -> App {
    let mut store = store_with_feature(ProjectStatus::Active);
    store.projects[0].features[0].sessions = sessions;
    let list: Vec<(String, String, String)> = panes
        .iter()
        .map(|(s, w, c)| (s.to_string(), w.to_string(), c.to_string()))
        .collect();
    tmux.expect_pane_commands().returning(move || list.clone());
    App::new_for_test(store, Box::new(tmux), Box::new(MockWorktreeOps::new()))
}

fn listed(app: &App) -> Vec<(String, bool)> {
    app.store.projects[0].features[0]
        .sessions
        .iter()
        .map(|s| (s.tmux_window.clone(), s.stopped))
        .collect()
}

const FEATURE: &str = "aiw-my-feat";

#[test]
fn exited_terminal_is_removed_and_quit_agent_is_marked_stopped() {
    let mut app = app_with(
        vec![
            session(SessionKind::Claude, "claude"),
            session(SessionKind::Claude, "claude-2"),
            session(SessionKind::Terminal, "terminal"),
        ],
        &[(FEATURE, "claude", "claude")],
    );
    assert!(app.reconcile_vanished_windows());
    assert_eq!(
        listed(&app),
        vec![("claude".into(), false), ("claude-2".into(), true)]
    );
}

#[test]
fn nothing_changes_when_every_window_is_up() {
    let mut app = app_with(
        vec![
            session(SessionKind::Claude, "claude"),
            session(SessionKind::Terminal, "terminal"),
        ],
        &[(FEATURE, "claude", "claude"), (FEATURE, "terminal", "bash")],
    );
    assert!(!app.reconcile_vanished_windows());
    assert_eq!(listed(&app).len(), 2);
}

#[test]
fn unreachable_tmux_changes_nothing() {
    let mut app = app_with(vec![session(SessionKind::Terminal, "terminal")], &[]);
    assert!(!app.reconcile_vanished_windows());
    assert_eq!(listed(&app).len(), 1);
}

#[test]
fn whole_feature_session_gone_is_left_to_feature_recovery() {
    let mut app = app_with(
        vec![session(SessionKind::Claude, "claude")],
        &[("aiw-other", "x", "bash")],
    );
    assert!(!app.reconcile_vanished_windows());
    assert_eq!(listed(&app), vec![("claude".into(), false)]);
}

#[test]
fn brand_new_sessions_get_a_grace_period() {
    let mut fresh = session(SessionKind::Terminal, "terminal");
    fresh.created_at = chrono::Utc::now();
    let mut starting = session(SessionKind::Codex, "codex");
    starting.created_at = chrono::Utc::now();
    let mut app = app_with(
        vec![session(SessionKind::Claude, "claude"), fresh, starting],
        &[(FEATURE, "claude", "claude"), (FEATURE, "codex", "zsh")],
    );
    assert!(!app.reconcile_vanished_windows());
    assert_eq!(listed(&app).len(), 3);
    assert!(app.exited_sessions.is_empty());
}

#[test]
fn already_stopped_and_todo_sessions_are_untouched() {
    let mut stopped = session(SessionKind::Codex, "codex");
    stopped.stopped = true;
    let mut app = app_with(
        vec![
            session(SessionKind::Claude, "claude"),
            stopped,
            session(SessionKind::Todos, "todos"),
        ],
        &[(FEATURE, "claude", "claude")],
    );
    assert!(!app.reconcile_vanished_windows());
    assert_eq!(listed(&app).len(), 3);
}

#[test]
fn removing_the_selected_session_selects_its_feature() {
    let mut app = app_with(
        vec![
            session(SessionKind::Claude, "claude"),
            session(SessionKind::Terminal, "terminal"),
        ],
        &[(FEATURE, "claude", "claude")],
    );
    app.selection = Selection::Session(0, 0, 1);
    assert!(app.reconcile_vanished_windows());
    assert!(matches!(app.selection, Selection::Feature(0, 0)));
}

#[test]
fn selection_shifts_past_a_removed_session() {
    assert!(matches!(
        selection_after_removal(&Selection::Session(0, 0, 3), 0, 0, 2),
        Selection::Session(0, 0, 2)
    ));
    assert!(matches!(
        selection_after_removal(&Selection::Session(0, 0, 1), 0, 0, 2),
        Selection::Session(0, 0, 1)
    ));
    assert!(matches!(
        selection_after_removal(&Selection::Session(1, 0, 5), 0, 0, 2),
        Selection::Session(1, 0, 5)
    ));
}

// ── agent exited, shell alive ────────────────────────────────────────────────

#[test]
fn an_agent_window_back_at_the_shell_is_marked_exited_not_finished() {
    let mut app = app_with(
        vec![
            session(SessionKind::Claude, "claude"),
            session(SessionKind::Terminal, "terminal"),
        ],
        &[(FEATURE, "claude", "bash"), (FEATURE, "terminal", "bash")],
    );
    app.record_attention(
        FEATURE,
        &AgentKind::Claude,
        AttentionState::CompletedAwaitingReview,
        AttentionSource::Inferred,
    );

    assert!(app.reconcile_vanished_windows());

    // The terminal's shell is its job, not an exit.
    assert_eq!(
        listed(&app),
        vec![("claude".into(), true), ("terminal".into(), false)]
    );
    let claude = &app.store.projects[0].features[0].sessions[0];
    assert_eq!(claude.status_text.as_deref(), Some(EXITED_STATUS));
    assert!(app.session_is_exited(&claude.id));
    assert!(
        app.attention_for(FEATURE).is_none(),
        "a crash is not a finished turn"
    );
    // Settled: the next sweep has nothing new to say.
    assert!(!app.reconcile_vanished_windows());
}

#[test]
fn a_feature_with_another_agent_still_running_keeps_its_attention() {
    let mut app = app_with(
        vec![
            session(SessionKind::Claude, "claude"),
            session(SessionKind::Codex, "codex"),
        ],
        &[(FEATURE, "claude", "zsh"), (FEATURE, "codex", "codex")],
    );
    app.record_attention(
        FEATURE,
        &AgentKind::Codex,
        AttentionState::CompletedAwaitingReview,
        AttentionSource::Hook,
    );

    assert!(app.reconcile_vanished_windows());

    assert_eq!(
        listed(&app),
        vec![("claude".into(), true), ("codex".into(), false)]
    );
    assert!(app.attention_for(FEATURE).is_some());
}

#[test]
fn a_split_agent_window_is_running_while_any_pane_is() {
    let mut app = app_with(
        vec![session(SessionKind::Claude, "claude")],
        &[(FEATURE, "claude", "bash"), (FEATURE, "claude", "claude")],
    );
    assert!(!app.reconcile_vanished_windows());
    assert!(app.exited_sessions.is_empty());
}

#[test]
fn the_usage_sweep_keeps_the_exited_marker_and_a_restart_clears_it() {
    let mut app = app_with(
        vec![session(SessionKind::Claude, "claude")],
        &[(FEATURE, "claude", "bash")],
    );
    assert!(app.reconcile_vanished_windows());
    let id = app.store.projects[0].features[0].sessions[0].id.clone();
    assert!(app.session_is_exited(&id));

    // What `apply_bg_result` does for every session: the marker wins.
    {
        let session = &mut app.store.projects[0].features[0].sessions[0];
        if app.exited_sessions.contains(&session.id) {
            session.status_text = Some(EXITED_STATUS.to_string());
        } else {
            session.status_text = Some("12k in".to_string());
        }
    }
    assert_eq!(
        app.store.projects[0].features[0].sessions[0]
            .status_text
            .as_deref(),
        Some(EXITED_STATUS)
    );

    app.set_session_stopped(0, 0, 0, false);
    assert!(!app.session_is_exited(&id));
    assert_eq!(listed(&app), vec![("claude".into(), false)]);
    assert!(
        app.store.projects[0].features[0].sessions[0]
            .status_text
            .is_none()
    );
}

#[test]
fn restarting_an_exited_agent_relaunches_it_in_its_live_window() {
    let mut tmux = MockTmuxOps::new();
    tmux.expect_session_exists().returning(|_| true);
    tmux.expect_window_exists().returning(|_, _| true);
    // No `create_window` expectation: the live window is reused.
    tmux.expect_launch_claude()
        .times(1)
        .withf(|session, window, _, _, _| session == FEATURE && window == "claude")
        .returning(|_, _, _, _, _| Ok(()));
    let mut app = app_with_tmux(
        vec![session(SessionKind::Claude, "claude")],
        &[(FEATURE, "claude", "bash")],
        tmux,
    );
    assert!(app.reconcile_vanished_windows());
    assert!(
        app.restart_stopped_session_window_unchecked(0, 0, 0)
            .unwrap()
    );

    let claude = &app.store.projects[0].features[0].sessions[0];
    assert!(!claude.stopped);
    assert!(!app.session_is_exited(&claude.id));
}

// ── the session on screen ────────────────────────────────────────────────────

#[test]
fn the_viewed_session_losing_its_window_returns_to_the_dashboard() {
    let mut app = app_with(
        vec![
            session(SessionKind::Claude, "claude"),
            session(SessionKind::Terminal, "terminal"),
        ],
        &[(FEATURE, "terminal", "bash")],
    );
    app.mode = AppMode::Viewing(view_state_for("my-project", "my-feat"));

    assert!(app.reconcile_vanished_windows());

    assert!(matches!(app.mode, AppMode::Normal));
    assert_eq!(
        listed(&app),
        vec![("claude".into(), true), ("terminal".into(), false)]
    );
    assert!(
        app.message
            .as_deref()
            .is_some_and(|m| m.contains("window is gone")),
        "{:?}",
        app.message
    );
}

#[test]
fn the_viewed_session_stays_open_while_its_window_lives() {
    let mut app = app_with(
        vec![session(SessionKind::Claude, "claude")],
        &[(FEATURE, "claude", "claude")],
    );
    app.mode = AppMode::Viewing(view_state_for("my-project", "my-feat"));

    assert!(!app.reconcile_vanished_windows());
    assert!(matches!(app.mode, AppMode::Viewing(_)));
}

#[test]
fn the_viewed_feature_losing_its_whole_tmux_session_also_leaves_the_view() {
    let mut app = app_with(
        vec![session(SessionKind::Claude, "claude")],
        &[("aiw-other", "x", "bash")],
    );
    app.mode = AppMode::Viewing(view_state_for("my-project", "my-feat"));

    assert!(app.reconcile_vanished_windows());

    assert!(matches!(app.mode, AppMode::Normal));
    // Feature-level recovery owns the session rows in this case.
    assert_eq!(listed(&app), vec![("claude".into(), false)]);
}
