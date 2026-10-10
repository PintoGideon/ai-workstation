//! FORK: tests for `crate::fork::session_reconcile` (sessions whose tmux window
//! closed on its own).

use super::support::*;
use crate::app::*;
use crate::fork::session_reconcile::selection_after_removal;
use crate::project::{FeatureSession, SessionKind};
use crate::traits::{MockTmuxOps, MockWorktreeOps};

fn session(kind: SessionKind, window: &str) -> FeatureSession {
    let mut s = make_session(window, None);
    s.kind = kind;
    s.created_at = chrono::Utc::now() - chrono::Duration::minutes(5);
    s
}

fn app_with(sessions: Vec<FeatureSession>, windows: &[(&str, &str)]) -> App {
    let mut store = store_with_feature(ProjectStatus::Active);
    store.projects[0].features[0].sessions = sessions;
    let list: Vec<(String, String, i64)> = windows
        .iter()
        .map(|(s, w)| (s.to_string(), w.to_string(), 0))
        .collect();
    let mut tmux = MockTmuxOps::new();
    tmux.expect_window_activity()
        .returning(move || list.clone());
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
        &[(FEATURE, "claude")],
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
        &[(FEATURE, "claude"), (FEATURE, "terminal")],
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
        &[("aiw-other", "x")],
    );
    assert!(!app.reconcile_vanished_windows());
    assert_eq!(listed(&app), vec![("claude".into(), false)]);
}

#[test]
fn brand_new_sessions_get_a_grace_period() {
    let mut fresh = session(SessionKind::Terminal, "terminal");
    fresh.created_at = chrono::Utc::now();
    let mut app = app_with(
        vec![session(SessionKind::Claude, "claude"), fresh],
        &[(FEATURE, "claude")],
    );
    assert!(!app.reconcile_vanished_windows());
    assert_eq!(listed(&app).len(), 2);
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
        &[(FEATURE, "claude")],
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
        &[(FEATURE, "claude")],
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
