//! FORK: the watchdog at the App level (DESIGN.md §19.8): flags from what the
//! app knows, their badges and needs-you rows, and the recovery actions.

use std::sync::{Arc, Mutex};

use chrono::{Duration, Utc};
use serde_json::json;

use super::support::*;
use crate::app::*;
use crate::fork::badges::{BadgeKind, session_badge};
use crate::fork::session_reconcile::{EXITED_STATUS, is_plain_shell};
use crate::fork::watchdog::{WatchAction, WatchKind};
use crate::project::SessionKind;
use crate::tmux::PaneCommand;
use crate::traits::{MockTmuxOps, MockWorktreeOps};

const FEATURE: &str = "aiw-my-feat";

/// One feature (`my-feat`) with `Claude 1` (conversation `conv-1`) and
/// `Codex 1` (thread `thread-1`), on `tmux`.
fn app_on(tmux: MockTmuxOps) -> (App, String, String) {
    let mut store = store_with_feature(ProjectStatus::Active);
    let feature = &mut store.projects[0].features[0];
    feature.add_session_named(SessionKind::Claude, "Claude 1".into());
    feature.add_session_named(SessionKind::Codex, "Codex 1".into());
    feature.sessions[0].claude_session_id = Some("conv-1".into());
    feature.sessions[1].token_usage_source = Some(crate::token_tracking::TokenUsageSource {
        provider: crate::token_tracking::TokenUsageProvider::Codex,
        id: "thread-1".into(),
    });
    let claude = feature.sessions[0].id.clone();
    let codex = feature.sessions[1].id.clone();
    let mut app = App::new_for_test(store, Box::new(tmux), Box::new(MockWorktreeOps::new()));
    app.config.low_memory_warn_mb = 0; // the resource gate never asks in tests
    (app, claude, codex)
}

fn app() -> (App, String, String) {
    app_on(MockTmuxOps::new())
}

fn window_of(app: &App, id: &str) -> String {
    let (pi, fi, si) = app.session_indices_by_id(id).unwrap();
    app.store.projects[pi].features[fi].sessions[si]
        .tmux_window
        .clone()
}

fn badge(app: &App, id: &str) -> crate::fork::badges::SessionBadge {
    let (pi, fi, si) = app.session_indices_by_id(id).unwrap();
    session_badge(app, pi, fi, si, Utc::now())
}

fn kind_of(app: &App, id: &str) -> Option<WatchKind> {
    app.watchdog_flag(id).map(|f| f.kind)
}

fn mark_exited(app: &mut App, id: &str) {
    let (pi, fi, si) = app.session_indices_by_id(id).unwrap();
    let session = &mut app.store.projects[pi].features[fi].sessions[si];
    session.stopped = true;
    session.status_text = Some(EXITED_STATUS.to_string());
    app.exited_sessions.insert(id.to_string());
}

fn pane(window: &str, command: &str, pid: i64) -> PaneCommand {
    PaneCommand {
        session: FEATURE.to_string(),
        window: window.to_string(),
        pane_id: format!("%{pid}"),
        pid: Some(pid),
        command: command.to_string(),
        shell_has_children: is_plain_shell(command).then_some(false),
    }
}

/// A Claude session that looks stuck at `now`: working, last hook and pane
/// change 12 minutes ago.
fn make_stuck(app: &mut App, id: &str, now: chrono::DateTime<Utc>) {
    app.ipc_thinking_feature_sessions.insert(id.to_string());
    app.watchdog_set_activity(id, now - Duration::minutes(12));
    app.watchdog_record_pane(id, 42, now - Duration::minutes(12));
}

#[test]
fn a_working_session_with_quiet_hooks_and_an_unchanged_pane_is_stuck() {
    let (mut app, claude, _) = app();
    let now = Utc::now();
    make_stuck(&mut app, &claude, now);

    assert!(app.watchdog_tick_at(now));
    assert_eq!(kind_of(&app, &claude), Some(WatchKind::Stuck));
    let b = badge(&app, &claude);
    assert_eq!(b.kind, BadgeKind::NeedsYou);
    assert!(
        b.detail
            .as_deref()
            .unwrap()
            .starts_with("stuck · no hooks or output for 12m"),
        "{:?}",
        b.detail
    );
    // Listed in the needs-you list, and the `!` actions fit a stuck agent.
    assert!(app.attention_rows().iter().any(|row| matches!(
        row,
        crate::app::attention::AttentionRow::Watchdog { session_id, .. } if *session_id == claude
    )));
    assert_eq!(
        app.watchdog_actions(&claude),
        vec![
            WatchAction::Interrupt,
            WatchAction::RestartResume,
            WatchAction::Dismiss
        ]
    );

    // A hook arrives: alive after all, the flag goes at once.
    app.watchdog_saw_activity(&claude);
    assert_eq!(kind_of(&app, &claude), None);
}

#[test]
fn moving_output_or_running_subagents_keep_a_quiet_session_from_being_stuck() {
    let (mut app, claude, _) = app();
    let now = Utc::now();
    make_stuck(&mut app, &claude, now);
    // The pane changed a minute ago (a long build printing): not stuck.
    app.watchdog_record_pane(&claude, 43, now - Duration::minutes(1));
    app.watchdog_tick_at(now);
    assert_eq!(kind_of(&app, &claude), None);

    // Quiet again, but a subagent runs: the parent waits on it.
    let (mut fresh, id, _) = self::app();
    make_stuck(&mut fresh, &id, now);
    fresh.subagent_counts.insert(id.clone(), 1);
    fresh.watchdog_tick_at(now);
    assert_eq!(kind_of(&fresh, &id), None);
}

#[test]
fn a_session_waiting_on_a_question_is_neither_stuck_nor_pasted_into() {
    use crate::app::attention::{AttentionSource, AttentionState};
    let mut tmux = MockTmuxOps::new();
    tmux.expect_paste_text().times(0);
    let (mut app, claude, _) = app_on(tmux);
    let now = Utc::now();
    make_stuck(&mut app, &claude, now);
    app.queue_followup(&claude, "next").unwrap();
    app.record_attention(
        FEATURE,
        &crate::project::AgentKind::Claude,
        AttentionState::Question,
        AttentionSource::Hook,
    );
    app.watchdog_tick_at(now);
    app.watchdog_tick_at(now + Duration::seconds(45));
    assert_eq!(kind_of(&app, &claude), None);
    app.ipc_thinking_feature_sessions.remove(&claude);
    let err = app
        .watchdog_perform(&claude, WatchAction::DeliverQueued)
        .unwrap_err()
        .to_string();
    assert!(err.contains("waiting on you"), "{err}");
    assert_eq!(app.queued_followup_count(&claude), 1);
}

#[test]
fn interrupt_presses_escape_in_the_agent() {
    let mut tmux = MockTmuxOps::new();
    tmux.expect_send_key_name()
        .times(1)
        .withf(|s, w, k| s == FEATURE && w == "claude" && k == "Escape")
        .returning(|_, _, _| Ok(()));
    let (mut app, claude, _) = app_on(tmux);
    make_stuck(&mut app, &claude, Utc::now());
    let message = app
        .watchdog_perform(&claude, WatchAction::Interrupt)
        .unwrap();
    assert_eq!(message, "Pressed Esc in Claude 1");
}

#[test]
fn an_exited_agent_is_flagged_and_restart_resumes_its_conversation() {
    let mut tmux = MockTmuxOps::new();
    tmux.expect_session_exists().returning(|_| true);
    tmux.expect_window_exists().returning(|_, _| true);
    tmux.expect_pane_commands()
        .returning(|| vec![pane("claude", "bash", 1001), pane("codex", "codex", 1002)]);
    tmux.expect_launch_claude()
        .times(1)
        .withf(|s, w, _, resume, _| {
            s == FEATURE && w == "claude" && resume.as_deref() == Some("conv-1")
        })
        .returning(|_, _, _, _, _| Ok(()));
    let (mut app, claude, _) = app_on(tmux);
    mark_exited(&mut app, &claude);

    app.watchdog_tick_at(Utc::now());
    assert_eq!(kind_of(&app, &claude), Some(WatchKind::Exited));
    let b = badge(&app, &claude);
    assert_eq!(b.kind, BadgeKind::Exited);
    assert_eq!(b.label_compact(), "exited · ! recover");

    let message = app
        .watchdog_perform(&claude, WatchAction::RestartResume)
        .unwrap();
    assert_eq!(message, "Restarted Claude 1, resuming its conversation");
    assert!(!app.store.projects[0].features[0].sessions[0].stopped);
    assert_eq!(kind_of(&app, &claude), None);
}

#[test]
fn a_codex_restart_resumes_its_thread() {
    let mut tmux = MockTmuxOps::new();
    tmux.expect_session_exists().returning(|_| true);
    tmux.expect_window_exists().returning(|_, _| true);
    tmux.expect_pane_commands()
        .returning(|| vec![pane("claude", "claude", 1001), pane("codex", "zsh", 1002)]);
    tmux.expect_launch_codex()
        .times(1)
        .withf(|_, w, _, resume, _| w == "codex" && resume.as_deref() == Some("thread-1"))
        .returning(|_, _, _, _, _| Ok(()));
    let (mut app, _, codex) = app_on(tmux);
    mark_exited(&mut app, &codex);
    app.watchdog_tick_at(Utc::now());
    assert_eq!(kind_of(&app, &codex), Some(WatchKind::Exited));
    app.watchdog_perform(&codex, WatchAction::RestartResume)
        .unwrap();
}

#[test]
fn restarting_a_stuck_agent_ends_it_then_relaunches_once_its_pane_is_at_the_shell() {
    let panes = Arc::new(Mutex::new(vec![
        pane("claude", "claude", 1001),
        pane("codex", "codex", 1002),
    ]));
    let mut tmux = MockTmuxOps::new();
    tmux.expect_session_exists().returning(|_| true);
    tmux.expect_window_exists().returning(|_, _| true);
    let listed = panes.clone();
    tmux.expect_pane_commands()
        .returning(move || listed.lock().unwrap().clone());
    tmux.expect_launch_claude()
        .times(1)
        .withf(|_, w, _, resume, _| w == "claude" && resume.as_deref() == Some("conv-1"))
        .returning(|_, _, _, _, _| Ok(()));
    let (mut app, claude, _) = app_on(tmux);
    let killed = Arc::new(Mutex::new(Vec::new()));
    let record = killed.clone();
    app.watchdog.killer = Arc::new(move |pid| record.lock().unwrap().push(pid));
    let now = Utc::now();
    make_stuck(&mut app, &claude, now);
    app.watchdog_tick_at(now);

    let message = app
        .watchdog_perform(&claude, WatchAction::RestartResume)
        .unwrap();
    assert!(
        message.starts_with("Stopping Claude 1's agent"),
        "{message}"
    );
    assert_eq!(*killed.lock().unwrap(), vec![1001]);
    // The killed turn sends no Stop: its working markers go now, or the
    // relaunched agent would read as working (and soon stuck) forever.
    assert!(!app.ipc_thinking_feature_sessions.contains(&claude));
    assert!(app.watchdog_restart_pending(&claude));
    assert_eq!(badge(&app, &claude).detail.as_deref(), Some("restarting…"));

    // Still running: nothing relaunched yet.
    app.watchdog_tick_at(now + Duration::seconds(2));
    assert!(app.watchdog_restart_pending(&claude));
    // The agent is gone, its shell is back: relaunched, resuming.
    panes.lock().unwrap()[0] = pane("claude", "bash", 1001);
    app.watchdog_tick_at(now + Duration::seconds(4));
    assert!(!app.watchdog_restart_pending(&claude));
}

#[test]
fn an_agent_that_dies_mid_turn_stops_reading_as_working_once_marked_exited() {
    let mut tmux = MockTmuxOps::new();
    tmux.expect_pane_commands()
        .returning(|| vec![pane("claude", "bash", 1001), pane("codex", "codex", 1002)]);
    let (mut app, claude, _) = app_on(tmux);
    app.store.projects[0].features[0].sessions[0].created_at = Utc::now() - Duration::minutes(5);
    app.ipc_thinking_feature_sessions.insert(claude.clone());
    app.ipc_tool_feature_sessions.insert(claude.clone());
    app.ipc_thinking_sessions.insert("conv-1".into());
    app.subagent_counts.insert(claude.clone(), 2);

    assert!(app.reconcile_vanished_windows());
    assert!(app.session_is_exited(&claude));
    assert!(!app.ipc_thinking_feature_sessions.contains(&claude));
    assert!(!app.ipc_tool_feature_sessions.contains(&claude));
    assert!(!app.ipc_thinking_sessions.contains("conv-1"));
    assert_eq!(app.subagent_count(&claude), 0);
}

#[test]
fn a_dead_agents_feature_level_marker_goes_unless_a_sibling_still_works() {
    let (mut app, claude, codex) = app();
    let working = |app: &App, id: &str| {
        let (pi, fi, si) = app.session_indices_by_id(id).unwrap();
        let feature = &app.store.projects[pi].features[fi];
        crate::fork::badges::session_working(app, feature, &feature.sessions[si])
    };
    // The claude's turn marked the feature too; the codex, with no marker of
    // its own, falls back to it and reads as working.
    app.ipc_thinking_feature_sessions.insert(claude.clone());
    app.ipc_thinking_sessions.insert(FEATURE.into());
    app.thinking_features.insert(FEATURE.into());
    app.ipc_thinking_feature_sessions.remove(&claude);
    assert!(working(&app, &codex));
    app.ipc_thinking_feature_sessions.insert(claude.clone());

    app.clear_session_working_markers(&claude);
    assert!(!working(&app, &claude));
    assert!(
        !working(&app, &codex),
        "the dead agent's feature marker is gone"
    );
    assert!(!app.ipc_thinking_sessions.contains(FEATURE));

    // A sibling that still works keeps the feature's marker.
    app.ipc_thinking_feature_sessions.insert(codex.clone());
    app.ipc_thinking_sessions.insert(FEATURE.into());
    app.thinking_features.insert(FEATURE.into());
    app.ipc_thinking_feature_sessions.insert(claude.clone());
    app.clear_session_working_markers(&claude);
    assert!(app.thinking_features.contains(FEATURE));
    assert!(working(&app, &codex));
}

#[test]
fn a_stuck_agent_that_never_stops_is_given_up_at_the_deadline() {
    let mut tmux = MockTmuxOps::new();
    tmux.expect_pane_commands()
        .returning(|| vec![pane("claude", "claude", 1001)]);
    tmux.expect_launch_claude().times(0);
    let (mut app, claude, _) = app_on(tmux);
    app.watchdog.killer = Arc::new(|_| {});
    let now = Utc::now();
    make_stuck(&mut app, &claude, now);
    app.watchdog_perform(&claude, WatchAction::RestartResume)
        .unwrap();
    app.watchdog_tick_at(now + Duration::seconds(31));
    assert!(!app.watchdog_restart_pending(&claude));
}

#[test]
fn items_queued_for_an_idle_codex_are_flagged_and_delivered_now_exactly_once() {
    let mut tmux = MockTmuxOps::new();
    tmux.expect_paste_text()
        .times(1)
        .withf(|s, w, text| s == FEATURE && w == "codex" && text == "run the tests")
        .returning(|_, _, _| Ok(()));
    let (mut app, _, codex) = app_on(tmux);
    app.queue_followup(&codex, "run the tests").unwrap();
    app.queue_followup(&codex, "then lint").unwrap();
    let t0 = Utc::now();

    // Idle, but only just: give the Stop hook its chance.
    app.watchdog_tick_at(t0);
    assert_eq!(kind_of(&app, &codex), None);
    app.watchdog_tick_at(t0 + Duration::seconds(45));
    assert_eq!(kind_of(&app, &codex), Some(WatchKind::QueuedIdle));
    let b = badge(&app, &codex);
    assert_eq!(b.kind, BadgeKind::NeedsYou);
    assert_eq!(
        b.label_compact(),
        "queued for an idle agent · ! recover · ⏭ 2 queued"
    );

    let message = app
        .watchdog_perform(&codex, WatchAction::DeliverQueued)
        .unwrap();
    assert_eq!(
        message,
        "Delivered the first queued item to Codex 1; 1 more go at its next turn end"
    );
    assert_eq!(app.queued_followup_count(&codex), 1);
    assert_eq!(app.followups_for(&codex)[0].text, "then lint");
    assert_eq!(kind_of(&app, &codex), None);
}

#[test]
fn a_failed_paste_leaves_the_item_queued() {
    let mut tmux = MockTmuxOps::new();
    tmux.expect_paste_text()
        .returning(|_, _, _| Err(anyhow::anyhow!("no such pane")));
    let (mut app, _, codex) = app_on(tmux);
    app.queue_followup(&codex, "run the tests").unwrap();
    assert!(
        app.watchdog_perform(&codex, WatchAction::DeliverQueued)
            .is_err()
    );
    assert_eq!(app.queued_followup_count(&codex), 1);
}

#[test]
fn queued_items_are_never_pasted_into_a_working_agent() {
    let mut tmux = MockTmuxOps::new();
    tmux.expect_paste_text().times(0);
    let (mut app, _, codex) = app_on(tmux);
    app.queue_followup(&codex, "run the tests").unwrap();
    app.ipc_thinking_feature_sessions.insert(codex.clone());
    let err = app
        .watchdog_perform(&codex, WatchAction::DeliverQueued)
        .unwrap_err()
        .to_string();
    assert!(err.contains("is working"), "{err}");
}

#[test]
fn context_critical_at_the_prompt_offers_compact_which_types_it_once() {
    let mut tmux = MockTmuxOps::new();
    tmux.expect_send_literal()
        .times(1)
        .withf(|s, w, text| s == FEATURE && w == "claude" && text == "/compact")
        .returning(|_, _, _| Ok(()));
    let (mut app, claude, _) = app_on(tmux);
    assert!(app.handle_status_line_message(&json!({
        "type": "status_line", "aiw_feature_session_id": claude, "context_used_pct": 92
    })));
    app.watchdog_tick_at(Utc::now());
    assert_eq!(kind_of(&app, &claude), Some(WatchKind::ContextCritical));
    assert_eq!(
        badge(&app, &claude).detail.as_deref(),
        Some("ctx 92% at the prompt · ! recover")
    );
    assert_eq!(
        app.watchdog_actions(&claude)[0],
        WatchAction::Compact,
        "compact first"
    );
    app.watchdog_perform(&claude, WatchAction::Compact).unwrap();
    assert_eq!(kind_of(&app, &claude), None);
}

#[test]
fn replay_delivers_a_prompt_still_waiting_for_the_agent() {
    let mut tmux = MockTmuxOps::new();
    tmux.expect_window_exists().returning(|_, _| true);
    tmux.expect_paste_text()
        .times(1)
        .withf(|_, w, text| w == "claude" && text == "fix the flaky test")
        .returning(|_, _, _| Ok(()));
    let (mut app, claude, _) = app_on(tmux);
    app.queue_pending_prompt(&claude, "fix the flaky test", false, false);
    assert!(
        app.watchdog_actions(&claude)
            .contains(&WatchAction::ReplayPrompt)
    );
    let message = app
        .watchdog_perform(&claude, WatchAction::ReplayPrompt)
        .unwrap();
    assert_eq!(message, "Delivered the waiting prompt to Claude 1");
    assert!(app.pending_prompt_for(&claude).is_none());
}

#[test]
fn dismiss_snoozes_the_flag_for_the_window() {
    let (mut app, _, codex) = app();
    app.config.aiw.watchdog_stuck_minutes = 2.0;
    app.queue_followup(&codex, "run the tests").unwrap();
    let t0 = Utc::now();
    app.watchdog_tick_at(t0);
    app.watchdog_tick_at(t0 + Duration::seconds(45));
    assert_eq!(kind_of(&app, &codex), Some(WatchKind::QueuedIdle));

    let message = app.watchdog_perform(&codex, WatchAction::Dismiss).unwrap();
    assert_eq!(message, "Codex 1: queued, idle dismissed for 2m");
    assert_eq!(kind_of(&app, &codex), None);
    // Within the snooze window: stays quiet. After it: back.
    app.watchdog_tick_at(Utc::now() + Duration::seconds(60));
    assert_eq!(kind_of(&app, &codex), None);
    app.watchdog_tick_at(Utc::now() + Duration::seconds(150));
    assert_eq!(kind_of(&app, &codex), Some(WatchKind::QueuedIdle));
}

#[test]
fn the_needs_you_row_opens_the_actions_and_x_dismisses_it() {
    let (mut app, _, codex) = app();
    app.queue_followup(&codex, "run the tests").unwrap();
    let t0 = Utc::now();
    app.watchdog_tick_at(t0);
    app.watchdog_tick_at(t0 + Duration::seconds(45));
    let rows = app.attention_rows();
    let index = rows
        .iter()
        .position(|r| matches!(r, crate::app::attention::AttentionRow::Watchdog { .. }))
        .unwrap();
    app.mode = AppMode::NotificationPicker(index, None);
    app.handle_notification_select().unwrap();
    match &app.mode {
        AppMode::Watchdog(state) => {
            assert_eq!(state.session_id, codex);
            assert_eq!(state.label, "Codex 1");
            assert_eq!(
                state.reason.as_deref(),
                Some("queued for an idle agent · ! recover")
            );
            assert_eq!(state.actions[0], WatchAction::DeliverQueued);
        }
        _ => panic!("expected the watchdog dialog"),
    }

    app.mode = AppMode::NotificationPicker(index, None);
    crate::handlers::handle_key(
        &mut app,
        crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Char('x')),
    )
    .unwrap();
    assert_eq!(kind_of(&app, &codex), None);
}

#[test]
fn bang_on_a_session_row_opens_its_actions_even_unflagged() {
    let (mut app, claude, _) = app();
    // An empty checkout: no earlier prompt to replay.
    let dir = tempfile::TempDir::new().unwrap();
    app.store.projects[0].features[0].workdir = dir.path().to_path_buf();
    let (pi, fi, si) = app.session_indices_by_id(&claude).unwrap();
    app.selection = Selection::Session(pi, fi, si);
    app.open_watchdog_for_selection();
    match &app.mode {
        AppMode::Watchdog(state) => {
            assert_eq!(state.reason, None);
            assert_eq!(state.actions, vec![WatchAction::Compact]);
        }
        _ => panic!("expected the watchdog dialog"),
    }
    // Keys: Esc closes.
    crate::handlers::handle_key(
        &mut app,
        crossterm::event::KeyEvent::from(crossterm::event::KeyCode::Esc),
    )
    .unwrap();
    assert!(matches!(app.mode, AppMode::Normal));
}

#[test]
fn automatic_restart_is_off_by_default_then_at_most_hourly() {
    let launches = Arc::new(Mutex::new(0));
    let counted = launches.clone();
    let mut tmux = MockTmuxOps::new();
    tmux.expect_session_exists().returning(|_| true);
    tmux.expect_window_exists().returning(|_, _| true);
    tmux.expect_pane_commands()
        .returning(|| vec![pane("claude", "bash", 1001)]);
    tmux.expect_launch_claude().returning(move |_, _, _, _, _| {
        *counted.lock().unwrap() += 1;
        Ok(())
    });
    let (mut app, claude, _) = app_on(tmux);
    mark_exited(&mut app, &claude);
    let now = Utc::now();
    app.watchdog_tick_at(now);
    assert_eq!(*launches.lock().unwrap(), 0, "off by default");

    app.config.aiw.watchdog_auto_restart = true;
    app.watchdog_tick_at(now);
    assert_eq!(*launches.lock().unwrap(), 1);
    // It exits again at once: not again within the hour…
    mark_exited(&mut app, &claude);
    app.watchdog_tick_at(now + Duration::minutes(5));
    assert_eq!(*launches.lock().unwrap(), 1);
    assert_eq!(kind_of(&app, &claude), Some(WatchKind::Exited));
    // …but after it.
    app.watchdog_tick_at(now + Duration::minutes(61));
    assert_eq!(*launches.lock().unwrap(), 2);
}

#[test]
fn turning_the_watchdog_off_clears_every_flag() {
    let (mut app, _, codex) = app();
    app.queue_followup(&codex, "run the tests").unwrap();
    let t0 = Utc::now();
    app.watchdog_tick_at(t0);
    app.watchdog_tick_at(t0 + Duration::seconds(45));
    assert!(app.watchdog_flag(&codex).is_some());
    app.config.aiw.watchdog = false;
    assert!(app.watchdog_tick());
    assert!(app.watchdog_flag(&codex).is_none());
    assert!(window_of(&app, &codex) == "codex");
}
