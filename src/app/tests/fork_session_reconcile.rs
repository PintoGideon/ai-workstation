//! FORK: tests for `crate::fork::session_reconcile` (sessions whose tmux window
//! closed on its own, or whose agent exited at the shell prompt).

use super::support::*;
use crate::app::attention::{AttentionSource, AttentionState};
use crate::app::*;
use crate::fork::session_reconcile::{EXITED_STATUS, is_plain_shell, selection_after_removal};
use crate::project::{AgentKind, FeatureSession, SessionKind};
use crate::tmux::PaneCommand;
use crate::traits::{MockTmuxOps, MockWorktreeOps};

fn session(kind: SessionKind, window: &str) -> FeatureSession {
    let mut s = make_session(window, None);
    s.kind = kind;
    s.created_at = chrono::Utc::now() - chrono::Duration::minutes(5);
    s
}

/// `panes`: `(session, window, pane_current_command)`, in creation order
/// (pane ids `%1`, `%2`, …); a window not listed is gone. A pane at a shell
/// has no children: nothing runs under it.
fn app_with(sessions: Vec<FeatureSession>, panes: &[(&str, &str, &str)]) -> App {
    app_with_tmux(sessions, panes, MockTmuxOps::new())
}

fn panes_from(panes: &[(&str, &str, &str)]) -> Vec<PaneCommand> {
    panes
        .iter()
        .enumerate()
        .map(|(i, (s, w, c))| PaneCommand {
            session: s.to_string(),
            window: w.to_string(),
            pane_id: format!("%{}", i + 1),
            pid: Some(1000 + i as i64),
            command: c.to_string(),
            shell_has_children: is_plain_shell(c).then_some(false),
        })
        .collect()
}

fn app_with_tmux(
    sessions: Vec<FeatureSession>,
    panes: &[(&str, &str, &str)],
    tmux: MockTmuxOps,
) -> App {
    app_with_panes(sessions, panes_from(panes), tmux)
}

fn app_with_panes(
    sessions: Vec<FeatureSession>,
    list: Vec<PaneCommand>,
    mut tmux: MockTmuxOps,
) -> App {
    let mut store = store_with_feature(ProjectStatus::Active);
    store.projects[0].features[0].sessions = sessions;
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
fn whole_feature_session_gone_settles_its_sessions_like_vanished_windows() {
    // Its last window closed, so tmux lists nothing for it at all: the agent
    // is marked stopped (and a terminal would be removed), so Enter restarts
    // it instead of typing into nothing until feature recovery catches up.
    let mut app = app_with(
        vec![
            session(SessionKind::Claude, "claude"),
            session(SessionKind::Terminal, "terminal"),
        ],
        &[("aiw-other", "x", "bash")],
    );
    assert!(app.reconcile_vanished_windows());
    assert_eq!(listed(&app), vec![("claude".into(), true)]);
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
fn a_split_agent_window_is_judged_by_its_original_pane() {
    // The agent runs in the pane the window was created with; a shell the
    // user split off beside it is not an exit.
    let mut app = app_with(
        vec![session(SessionKind::Claude, "claude")],
        &[(FEATURE, "claude", "claude"), (FEATURE, "claude", "bash")],
    );
    assert!(!app.reconcile_vanished_windows());
    assert!(app.exited_sessions.is_empty());

    // …and `htop` in that other pane does not hide the agent exiting.
    let mut app = app_with(
        vec![session(SessionKind::Claude, "claude")],
        &[(FEATURE, "claude", "bash"), (FEATURE, "claude", "htop")],
    );
    assert!(app.reconcile_vanished_windows());
    assert_eq!(listed(&app), vec![("claude".into(), true)]);
}

#[test]
fn a_shell_in_the_foreground_with_a_child_is_a_running_agent() {
    // An agent launched before agents were `exec`ed (`sh script` → claude),
    // or a harness that is itself an `sh` script: tmux reports `sh`, but
    // the pane's shell still has a child, so the agent is up.
    let mut panes = panes_from(&[(FEATURE, "claude", "sh"), (FEATURE, "codex", "bash")]);
    panes[0].shell_has_children = Some(true);
    // `ps` could not answer: never conclude "exited" from that.
    panes[1].shell_has_children = None;
    let mut app = app_with_panes(
        vec![
            session(SessionKind::Claude, "claude"),
            session(SessionKind::Codex, "codex"),
        ],
        panes,
        MockTmuxOps::new(),
    );
    assert!(!app.reconcile_vanished_windows());
    assert!(app.exited_sessions.is_empty());
    assert_eq!(
        listed(&app),
        vec![("claude".into(), false), ("codex".into(), false)]
    );
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

#[test]
fn an_exited_agent_is_still_exited_after_aiw_restarts() {
    // Saved state: stopped, with the "exited" marker lost with the process.
    let mut exited = session(SessionKind::Claude, "claude");
    exited.stopped = true;
    let mut tmux = MockTmuxOps::new();
    tmux.expect_session_exists().returning(|_| true);
    tmux.expect_window_exists().returning(|_, _| true);
    tmux.expect_launch_claude()
        .times(1)
        .returning(|_, _, _, _, _| Ok(()));
    let mut app = app_with_tmux(vec![exited], &[(FEATURE, "claude", "bash")], tmux);
    assert!(app.exited_sessions.is_empty());

    // The first sweep restores the marker…
    assert!(app.reconcile_vanished_windows());
    let id = app.store.projects[0].features[0].sessions[0].id.clone();
    assert!(app.session_is_exited(&id));
    assert_eq!(
        app.store.projects[0].features[0].sessions[0]
            .status_text
            .as_deref(),
        Some(EXITED_STATUS)
    );
    assert!(!app.reconcile_vanished_windows());

    // …and one restart relaunches it.
    assert!(
        app.restart_stopped_session_window_unchecked(0, 0, 0)
            .unwrap()
    );
    assert!(!app.store.projects[0].features[0].sessions[0].stopped);
}

#[test]
fn a_restart_relaunches_without_waiting_for_the_sweep() {
    // Restart pressed before any sweep ran since AIW started.
    let mut exited = session(SessionKind::Claude, "claude");
    exited.stopped = true;
    let mut tmux = MockTmuxOps::new();
    tmux.expect_session_exists().returning(|_| true);
    tmux.expect_window_exists().returning(|_, _| true);
    tmux.expect_launch_claude()
        .times(1)
        .returning(|_, _, _, _, _| Ok(()));
    let mut app = app_with_tmux(vec![exited], &[(FEATURE, "claude", "zsh")], tmux);
    assert!(
        app.restart_stopped_session_window_unchecked(0, 0, 0)
            .unwrap()
    );
}

#[test]
fn a_restart_never_types_into_an_agent_that_is_running_again() {
    // Marked exited, then the user started the agent again by hand.
    let mut tmux = MockTmuxOps::new();
    tmux.expect_session_exists().returning(|_| true);
    tmux.expect_window_exists().returning(|_, _| true);
    tmux.expect_launch_claude().times(0);
    let mut app = app_with_tmux(
        vec![session(SessionKind::Claude, "claude")],
        &[(FEATURE, "claude", "claude")],
        tmux,
    );
    let id = app.store.projects[0].features[0].sessions[0].id.clone();
    app.store.projects[0].features[0].sessions[0].stopped = true;
    app.exited_sessions.insert(id.clone());

    assert!(
        !app.restart_stopped_session_window_unchecked(0, 0, 0)
            .unwrap()
    );
    assert!(!app.store.projects[0].features[0].sessions[0].stopped);
    assert!(!app.session_is_exited(&id));
}

#[test]
fn a_restart_in_a_split_window_selects_the_agent_pane_first() {
    let mut tmux = MockTmuxOps::new();
    tmux.expect_session_exists().returning(|_| true);
    tmux.expect_window_exists().returning(|_, _| true);
    let mut seq = mockall::Sequence::new();
    tmux.expect_select_pane()
        .times(1)
        .in_sequence(&mut seq)
        .withf(|pane| pane == "%1")
        .returning(|_| Ok(()));
    tmux.expect_launch_claude()
        .times(1)
        .in_sequence(&mut seq)
        .returning(|_, _, _, _, _| Ok(()));
    let mut app = app_with_tmux(
        vec![session(SessionKind::Claude, "claude")],
        &[(FEATURE, "claude", "bash"), (FEATURE, "claude", "htop")],
        tmux,
    );
    assert!(app.reconcile_vanished_windows());
    assert!(
        app.restart_stopped_session_window_unchecked(0, 0, 0)
            .unwrap()
    );
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
    app.return_to_dashboard(&view_state_for("my-project", "my-feat"));
    app.selection = Selection::Session(0, 0, 0);
    app.workspace_commit_now();

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
    app.return_to_dashboard(&view_state_for("my-project", "my-feat"));

    assert!(!app.reconcile_vanished_windows());
    assert!(matches!(app.mode, AppMode::Normal));
}

#[test]
fn the_viewed_feature_losing_its_whole_tmux_session_also_leaves_the_view() {
    let mut app = app_with(
        vec![session(SessionKind::Claude, "claude")],
        &[("aiw-other", "x", "bash")],
    );
    app.return_to_dashboard(&view_state_for("my-project", "my-feat"));

    assert!(app.reconcile_vanished_windows());

    assert!(matches!(app.mode, AppMode::Normal));
    assert_eq!(listed(&app), vec![("claude".into(), true)]);
}

// ── projects whose folder is gone ────────────────────────────────────────────

#[test]
fn a_missing_project_folder_is_flagged_and_announced_once() {
    let mut app = app_with(vec![], &[]);
    let dir = tempfile::tempdir().unwrap();
    let gone = dir.path().join("vanished");
    app.store.projects[0].repo = gone.clone();

    assert!(app.reconcile_missing_project_dirs());
    assert!(app.missing_project_repos.contains("proj-1"));
    assert_eq!(app.toasts.len(), 1);
    assert!(app.toasts[0].message.contains("folder is gone"));
    assert!(app.toasts[0].message.contains("1 feature(s) still run"));

    // Same state next sweep: no second toast.
    assert!(!app.reconcile_missing_project_dirs());
    assert_eq!(app.toasts.len(), 1);

    // The folder coming back clears the flag.
    std::fs::create_dir_all(&gone).unwrap();
    assert!(app.reconcile_missing_project_dirs());
    assert!(app.missing_project_repos.is_empty());
}
