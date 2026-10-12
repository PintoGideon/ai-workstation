//! FORK: scheduled prompts and commands at the App level (DESIGN.md §19.9):
//! each delivery outcome, deferral while the agent waits on you, catch-up,
//! no stacking, shell schedules, limits, `aiw.json` schedules, the dialog and
//! `aiw automation schedule`.

use std::sync::{Arc, Mutex};

use chrono::{Duration, Utc};
use serde_json::{Value, json};

use super::support::*;
use crate::app::fork_schedules::{EditorField, ScheduleScope, Trigger};
use crate::app::*;
use crate::fork::schedules::{CatchUp, Outcome, Schedule, ScheduleKind};
use crate::fork::session_reconcile::{EXITED_STATUS, is_plain_shell};
use crate::project::SessionKind;
use crate::tmux::PaneCommand;
use crate::traits::{MockTmuxOps, MockWorktreeOps};

const FEATURE: &str = "aiw-my-feat";

/// One feature (`my-feat`) with `Claude 1` (conversation `conv-1`).
fn app_on(tmux: MockTmuxOps) -> (App, String) {
    let mut store = store_with_feature(ProjectStatus::Active);
    let feature = &mut store.projects[0].features[0];
    feature.add_session_named(SessionKind::Claude, "Claude 1".into());
    feature.sessions[0].claude_session_id = Some("conv-1".into());
    let claude = feature.sessions[0].id.clone();
    let mut app = App::new_for_test(store, Box::new(tmux), Box::new(MockWorktreeOps::new()));
    app.config.low_memory_warn_mb = 0; // the resource gate never asks in tests
    app.config.aiw.schedule_min_interval_secs = 1;
    (app, claude)
}

/// Every pasted `(window, text)`.
type Pasted = Arc<Mutex<Vec<(String, String)>>>;

fn recording_tmux() -> (MockTmuxOps, Pasted) {
    let pasted = Arc::new(Mutex::new(Vec::new()));
    let mut tmux = MockTmuxOps::new();
    let log = pasted.clone();
    tmux.expect_paste_text().returning(move |_, w, text| {
        log.lock().unwrap().push((w.to_string(), text.to_string()));
        Ok(())
    });
    (tmux, pasted)
}

/// An interval prompt schedule pinned to `session`, created `ago` seconds
/// before now.
fn add_prompt(app: &mut App, session: &str, spec: &str, text: &str, ago: i64) -> String {
    let mut s = App::new_schedule("feat-1");
    s.session_id = Some(session.to_string());
    s.spec = spec.into();
    s.text = text.into();
    let id = app.save_schedule(s).unwrap();
    let created = Utc::now() - Duration::seconds(ago);
    let item = app.schedules.items.iter_mut().find(|x| x.id == id).unwrap();
    item.created_at = created;
    app.schedules.started = created - Duration::hours(1);
    id
}

fn sched(app: &App, id: &str) -> Schedule {
    app.schedule(id).cloned().unwrap()
}

fn outcomes(app: &App, id: &str) -> Vec<String> {
    app.schedule_history(id)
        .into_iter()
        .rev()
        .map(|r| r.outcome)
        .collect()
}

fn pane(window: &str, command: &str) -> PaneCommand {
    PaneCommand {
        session: FEATURE.to_string(),
        window: window.to_string(),
        pane_id: "%9".into(),
        pid: Some(9),
        command: command.to_string(),
        shell_has_children: is_plain_shell(command).then_some(false),
    }
}

#[test]
fn an_idle_agent_gets_the_prompt_pasted_once_per_fire() {
    let (tmux, pasted) = recording_tmux();
    let (mut app, claude) = app_on(tmux);
    let id = add_prompt(&mut app, &claude, "every 10m", "run the tests", 0);
    let created = sched(&app, &id).created_at;

    // Not yet due.
    assert!(!app.schedules_tick_at(created + Duration::minutes(9), &chrono::Utc));
    assert!(pasted.lock().unwrap().is_empty());

    let t1 = created + Duration::minutes(10);
    assert!(app.schedules_tick_at(t1, &chrono::Utc));
    assert_eq!(
        *pasted.lock().unwrap(),
        vec![("claude".to_string(), "run the tests".to_string())]
    );
    assert_eq!(outcomes(&app, &id), vec!["delivered"]);
    assert_eq!(sched(&app, &id).runs, 1);
    // The same tick again fires nothing (once per fire).
    app.schedules_tick_at(t1 + Duration::seconds(30), &chrono::Utc);
    assert_eq!(pasted.lock().unwrap().len(), 1);
    // The next slot fires again.
    app.schedules_tick_at(t1 + Duration::minutes(10), &chrono::Utc);
    assert_eq!(pasted.lock().unwrap().len(), 2);
    assert!(app.session_has_active_schedule(&claude));
    assert!(app.feature_has_active_schedule("feat-1"));
    assert!(
        app.schedule_info_line(&claude)
            .unwrap()
            .contains("every 10 min")
    );
}

#[test]
fn a_working_agent_gets_it_queued_and_fires_never_stack() {
    let mut tmux = MockTmuxOps::new();
    tmux.expect_paste_text().times(0);
    let (mut app, claude) = app_on(tmux);
    app.ipc_thinking_feature_sessions.insert(claude.clone()); // working
    let id = add_prompt(&mut app, &claude, "every 1m", "lint", 120);

    app.run_schedule_now(&id, Trigger::Timer).unwrap();
    assert_eq!(app.queued_followup_count(&claude), 1);
    assert_eq!(app.followups_for(&claude)[0].text, "lint");
    let queued = sched(&app, &id).queued_followup;
    assert!(queued.is_some());

    // Still waiting in the queue: the next fire is skipped, not stacked.
    let outcome = app.run_schedule_now(&id, Trigger::Timer).unwrap();
    assert_eq!(outcome, Outcome::SkippedStillQueued);
    assert_eq!(app.queued_followup_count(&claude), 1);

    // Delivered (taken off the queue): the next fire queues again.
    app.take_followup(&claude).unwrap();
    assert_eq!(
        app.run_schedule_now(&id, Trigger::Timer).unwrap(),
        Outcome::Queued
    );
    assert_eq!(
        outcomes(&app, &id),
        vec!["queued", "skipped: previous run still queued", "queued"]
    );
    assert_eq!(sched(&app, &id).runs, 2, "skips do not count");
}

#[test]
fn something_already_queued_means_the_prompt_queues_behind_it() {
    let mut tmux = MockTmuxOps::new();
    tmux.expect_paste_text().times(0);
    let (mut app, claude) = app_on(tmux);
    app.queue_followup(&claude, "first").unwrap();
    let id = add_prompt(&mut app, &claude, "every 1m", "second", 120);
    assert_eq!(
        app.run_schedule_now(&id, Trigger::Timer).unwrap(),
        Outcome::Queued
    );
    let texts: Vec<_> = app
        .followups_for(&claude)
        .iter()
        .map(|f| f.text.clone())
        .collect();
    assert_eq!(texts, ["first", "second"]);
}

#[test]
fn waiting_on_you_defers_and_retries_until_it_can_deliver() {
    use crate::app::attention::{AttentionSource, AttentionState};
    let (tmux, pasted) = recording_tmux();
    let (mut app, claude) = app_on(tmux);
    let id = add_prompt(&mut app, &claude, "every 1h", "status?", 0);
    app.record_attention(
        FEATURE,
        &crate::project::AgentKind::Claude,
        AttentionState::Question,
        AttentionSource::Hook,
    );
    let t1 = sched(&app, &id).created_at + Duration::hours(1);
    app.schedules_tick_at(t1, &chrono::Utc);
    assert!(pasted.lock().unwrap().is_empty());
    assert!(app.schedule_is_deferred(&id));
    assert_eq!(outcomes(&app, &id), vec!["skipped: waiting on you"]);

    // Retried quietly while it still waits (no new history lines).
    app.schedules_tick_at(t1 + Duration::seconds(40), &chrono::Utc);
    assert_eq!(outcomes(&app, &id).len(), 1);

    // Answered: the next retry delivers it.
    app.clear_attention(FEATURE);
    app.schedules_tick_at(t1 + Duration::seconds(80), &chrono::Utc);
    assert_eq!(pasted.lock().unwrap().len(), 1);
    assert!(!app.schedule_is_deferred(&id));
    let history = app.schedule_history(&id);
    assert_eq!(history[0].outcome, "delivered");
    assert_eq!(history[0].detail, "after waiting on you");
}

#[test]
fn a_finished_turn_is_not_waiting_on_you_but_a_held_edit_is() {
    let (tmux, pasted) = recording_tmux();
    let (mut app, claude) = app_on(tmux);
    // The generic marker a turn end leaves: the agent is at its prompt.
    app.pending_inputs.push(crate::app::PendingInput {
        feature_name: Some("my-feat".into()),
        project_name: Some("my-project".into()),
        notification_type: "input-request".into(),
        message: "Agent finished and is waiting for input".into(),
        ..Default::default()
    });
    let id = add_prompt(&mut app, &claude, "every 1m", "next step", 120);
    assert_eq!(
        app.run_schedule_now(&id, Trigger::Timer).unwrap(),
        Outcome::Delivered
    );
    assert_eq!(pasted.lock().unwrap().len(), 1);
    // A Vibeless edit held for review blocks the agent: wait.
    app.pending_inputs.push(crate::app::PendingInput {
        feature_name: Some("my-feat".into()),
        project_name: Some("my-project".into()),
        notification_type: "diff-review".into(),
        ..Default::default()
    });
    assert_eq!(
        app.run_schedule_now(&id, Trigger::Timer).unwrap(),
        Outcome::SkippedWaiting
    );
    assert_eq!(pasted.lock().unwrap().len(), 1);
}

#[test]
fn a_deferred_fire_gives_up_when_the_next_slot_comes() {
    use crate::app::attention::{AttentionSource, AttentionState};
    let mut tmux = MockTmuxOps::new();
    tmux.expect_paste_text().times(0);
    let (mut app, claude) = app_on(tmux);
    let id = add_prompt(&mut app, &claude, "every 2m", "x", 0);
    app.record_attention(
        FEATURE,
        &crate::project::AgentKind::Claude,
        AttentionState::Question,
        AttentionSource::Hook,
    );
    let t1 = sched(&app, &id).created_at + Duration::minutes(2);
    app.schedules_tick_at(t1, &chrono::Utc);
    assert!(app.schedule_is_deferred(&id));
    app.schedules_tick_at(t1 + Duration::minutes(3), &chrono::Utc);
    assert!(!app.schedule_is_deferred(&id));
    let history = app.schedule_history(&id);
    assert_eq!(history[0].detail, "gave up");
    assert!(
        sched(&app, &id).last_fire.is_some(),
        "advanced past the slot"
    );
}

#[test]
fn a_stopped_session_is_skipped_unless_start_if_stopped() {
    let mut tmux = MockTmuxOps::new();
    tmux.expect_paste_text().times(0);
    tmux.expect_session_exists().returning(|_| true);
    tmux.expect_window_exists().returning(|_, _| true);
    tmux.expect_pane_commands()
        .returning(|| vec![pane("claude", "bash")]);
    tmux.expect_launch_claude()
        .times(1)
        .withf(|s, w, _, resume, _| {
            s == FEATURE && w == "claude" && resume.as_deref() == Some("conv-1")
        })
        .returning(|_, _, _, _, _| Ok(()));
    let (mut app, claude) = app_on(tmux);
    {
        let session = &mut app.store.projects[0].features[0].sessions[0];
        session.stopped = true;
        session.status_text = Some(EXITED_STATUS.to_string());
    }
    app.exited_sessions.insert(claude.clone());
    let id = add_prompt(&mut app, &claude, "every 1m", "resume work", 120);
    assert_eq!(
        app.run_schedule_now(&id, Trigger::Timer).unwrap(),
        Outcome::SkippedNotRunning
    );

    app.schedule_mut_for_test(&id).start_if_stopped = true;
    assert_eq!(
        app.run_schedule_now(&id, Trigger::Timer).unwrap(),
        Outcome::Started
    );
    let pending = app
        .pending_prompt_for(&claude)
        .expect("the prompt waits for the agent");
    assert_eq!(pending.text, "resume work");
    assert!(pending.send);
}

#[test]
fn missed_fires_follow_catch_up() {
    let (tmux, pasted) = recording_tmux();
    let (mut app, claude) = app_on(tmux);
    // Due an hour before AIW started.
    let skip = add_prompt(&mut app, &claude, "every 1h", "skip me", 3 * 3600);
    let run = add_prompt(&mut app, &claude, "every 1h", "run once", 3 * 3600);
    app.schedule_mut_for_test(&skip).catch_up = CatchUp::None;
    app.schedules.started = Utc::now();
    let now = Utc::now() + Duration::seconds(1);
    app.schedules_tick_at(now, &chrono::Utc);
    assert_eq!(
        outcomes(&app, &skip),
        vec!["missed while aiw was not running"]
    );
    let history = app.schedule_history(&run);
    assert_eq!(history.len(), 1, "caught up once, not once per missed hour");
    assert_eq!(history[0].detail, "catch-up");
    assert_eq!(
        pasted
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, t)| t == "run once")
            .count(),
        1
    );
    assert!(!pasted.lock().unwrap().iter().any(|(_, t)| t == "skip me"));
    // Nothing more until the next slot.
    app.schedules_tick_at(now + Duration::minutes(5), &chrono::Utc);
    assert_eq!(app.schedule_history(&run).len(), 1);
}

#[test]
fn a_shell_schedule_opens_its_terminal_and_runs_the_command_there() {
    let ran = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
    let created = Arc::new(Mutex::new(Vec::<String>::new()));
    let mut tmux = MockTmuxOps::new();
    tmux.expect_session_exists().returning(|_| true);
    let c = created.clone();
    tmux.expect_create_window().returning(move |_, w, _| {
        c.lock().unwrap().push(w.to_string());
        Ok(())
    });
    let c = created.clone();
    tmux.expect_window_exists()
        .returning(move |_, w| c.lock().unwrap().iter().any(|x| x == w));
    let c = created.clone();
    tmux.expect_pane_commands()
        .returning(move || c.lock().unwrap().iter().map(|w| pane(w, "bash")).collect());
    let r = ran.clone();
    tmux.expect_run_shell_command().returning(move |_, w, cmd| {
        r.lock().unwrap().push((w.to_string(), cmd.to_string()));
        Ok(())
    });
    let (mut app, _claude) = app_on(tmux);
    app.selection = Selection::Feature(0, 0);
    let mut s = App::new_schedule("feat-1");
    s.kind = ScheduleKind::Shell;
    s.spec = "every 1m".into();
    s.text = "make test".into();
    let id = app.save_schedule(s).unwrap();

    assert_eq!(
        app.run_schedule_now(&id, Trigger::RunNow).unwrap(),
        Outcome::Ran
    );
    let feature = &app.store.projects[0].features[0];
    let terminal = feature
        .sessions
        .iter()
        .find(|s| s.label == "schedule")
        .expect("a `schedule` terminal was opened");
    assert_eq!(terminal.kind, SessionKind::Terminal);
    assert_eq!(
        *ran.lock().unwrap(),
        vec![(terminal.tmux_window.clone(), "make test".to_string())]
    );
    assert!(
        matches!(app.selection, Selection::Feature(0, 0)),
        "selection kept"
    );
    // The second run reuses the terminal.
    app.run_schedule_now(&id, Trigger::RunNow).unwrap();
    assert_eq!(
        app.store.projects[0].features[0]
            .sessions
            .iter()
            .filter(|s| s.label == "schedule")
            .count(),
        1
    );
    assert_eq!(ran.lock().unwrap().len(), 2);
}

#[test]
fn a_busy_terminal_is_skipped() {
    let mut store = store_with_feature(ProjectStatus::Active);
    let window = store.projects[0].features[0]
        .add_session_named(SessionKind::Terminal, "tests".into())
        .tmux_window
        .clone();
    let mut tmux = MockTmuxOps::new();
    tmux.expect_session_exists().returning(|_| true);
    tmux.expect_window_exists().returning(|_, _| true);
    tmux.expect_run_shell_command().times(0);
    let w = window.clone();
    tmux.expect_pane_commands()
        .returning(move || vec![pane(&w, "vim")]);
    let mut app = App::new_for_test(store, Box::new(tmux), Box::new(MockWorktreeOps::new()));
    app.config.aiw.schedule_min_interval_secs = 1;
    let mut s = App::new_schedule("feat-1");
    s.kind = ScheduleKind::Shell;
    s.target = Some("tests".into());
    s.spec = "every 1m".into();
    s.text = "cargo test".into();
    let id = app.save_schedule(s).unwrap();
    assert_eq!(
        app.run_schedule_now(&id, Trigger::RunNow).unwrap(),
        Outcome::SkippedBusy("vim".into())
    );
    assert!(
        app.session_has_active_schedule(&app.store.projects[0].features[0].sessions[0].id.clone())
    );
}

#[test]
fn a_stopped_feature_skips_a_shell_schedule() {
    let mut tmux = MockTmuxOps::new();
    tmux.expect_session_exists().returning(|_| false);
    tmux.expect_run_shell_command().times(0);
    let (mut app, _) = app_on(tmux);
    app.store.projects[0].features[0].status = ProjectStatus::Stopped;
    let mut s = App::new_schedule("feat-1");
    s.kind = ScheduleKind::Shell;
    s.spec = "every 1m".into();
    s.text = "make".into();
    let id = app.save_schedule(s).unwrap();
    assert_eq!(
        app.run_schedule_now(&id, Trigger::RunNow).unwrap(),
        Outcome::SkippedNotRunning
    );
}

#[test]
fn pause_stops_fires_and_resume_counts_from_now() {
    let (tmux, pasted) = recording_tmux();
    let (mut app, claude) = app_on(tmux);
    let id = add_prompt(&mut app, &claude, "every 1m", "tick", 0);
    app.set_schedule_paused(&id, true).unwrap();
    assert!(!app.session_has_active_schedule(&claude));
    app.schedules_tick_at(Utc::now() + Duration::minutes(10), &chrono::Utc);
    assert!(pasted.lock().unwrap().is_empty());
    app.set_schedule_paused(&id, false).unwrap();
    // Resumed now: not due at once (no burst), due a minute later.
    app.schedules_tick_at(Utc::now() + Duration::seconds(10), &chrono::Utc);
    assert!(pasted.lock().unwrap().is_empty());
    app.schedules_tick_at(Utc::now() + Duration::seconds(70), &chrono::Utc);
    assert_eq!(pasted.lock().unwrap().len(), 1);
}

#[test]
fn max_runs_and_one_shots_finish() {
    let (tmux, pasted) = recording_tmux();
    let (mut app, claude) = app_on(tmux);
    let id = add_prompt(&mut app, &claude, "every 1m", "twice", 0);
    app.schedule_mut_for_test(&id).max_runs = Some(2);
    app.run_schedule_now(&id, Trigger::RunNow).unwrap();
    app.run_schedule_now(&id, Trigger::RunNow).unwrap();
    assert!(sched(&app, &id).finished(Utc::now()));
    app.schedules_tick_at(Utc::now() + Duration::hours(1), &chrono::Utc);
    assert_eq!(pasted.lock().unwrap().len(), 2);
    assert!(!app.session_has_active_schedule(&claude));

    // A one-shot in the past is refused; in the future it fires once.
    let mut past = App::new_schedule("feat-1");
    past.session_id = Some(claude.clone());
    past.spec = "at 2001-01-01 10:00".into();
    past.text = "x".into();
    assert!(
        app.save_schedule(past)
            .unwrap_err()
            .to_string()
            .contains("already passed")
    );
}

#[test]
fn saving_checks_the_schedule() {
    let (mut app, claude) = app_on(MockTmuxOps::new());
    let mut s = App::new_schedule("feat-1");
    s.session_id = Some(claude.clone());
    s.spec = "every 30m".into();
    assert!(
        app.save_schedule(s.clone())
            .unwrap_err()
            .to_string()
            .contains("prompt is empty")
    );
    s.text = "go".into();
    s.spec = "sometimes".into();
    assert!(app.save_schedule(s.clone()).is_err());
    app.config.aiw.schedule_min_interval_secs = 60;
    s.spec = "every 10s".into();
    assert!(
        app.save_schedule(s.clone())
            .unwrap_err()
            .to_string()
            .contains("at least 1 min")
    );
    s.spec = "every 90m".into();
    let id = app.save_schedule(s).unwrap();
    assert_eq!(sched(&app, &id).spec, "every 1h30m", "stored canonical");
}

#[test]
fn deleting_the_session_or_feature_drops_its_schedules() {
    let (mut app, claude) = app_on(MockTmuxOps::new());
    let pinned = add_prompt(&mut app, &claude, "every 1m", "a", 0);
    let mut s = App::new_schedule("feat-1");
    s.kind = ScheduleKind::Shell;
    s.spec = "every 1m".into();
    s.text = "b".into();
    let feature_level = app.save_schedule(s).unwrap();
    app.forget_organize_session(&claude);
    assert!(app.schedule(&pinned).is_none());
    assert!(app.schedule(&feature_level).is_some());
    let feature = app.store.projects[0].features[0].clone();
    app.forget_organize_feature(&feature);
    assert!(app.schedule(&feature_level).is_none());
}

#[test]
fn orphans_are_dropped_by_the_timer() {
    let (mut app, claude) = app_on(MockTmuxOps::new());
    let id = add_prompt(&mut app, &claude, "every 1m", "a", 0);
    app.store.projects[0].features.clear();
    assert!(app.schedules_tick_at(Utc::now(), &chrono::Utc));
    assert!(app.schedule(&id).is_none());
}

#[test]
fn aiw_json_schedules_are_materialized_per_matching_feature_and_kept_in_sync() {
    let repo = tempfile::TempDir::new().unwrap();
    let (mut app, _) = app_on(MockTmuxOps::new());
    app.store.projects[0].repo = repo.path().to_path_buf();
    let write = |schedules: Value| {
        std::fs::write(
            repo.path().join("aiw.json"),
            serde_json::to_string(&json!({ "schedules": schedules })).unwrap(),
        )
        .unwrap();
    };
    write(json!([
        {"name": "nightly", "match": "my-*", "when": "daily 02:00", "command": "make test", "terminal": "ci"},
        {"name": "other", "match": "web-*", "when": "every 1h", "prompt": "x"},
        {"name": "broken", "when": "sometimes", "prompt": "x"}
    ]));
    app.sync_config_schedules();
    let ids: Vec<_> = app.schedules.items.iter().map(|s| s.id.clone()).collect();
    assert_eq!(ids, vec!["cfg:feat-1:nightly"]);
    let s = sched(&app, "cfg:feat-1:nightly");
    assert_eq!(s.kind, ScheduleKind::Shell);
    assert_eq!(s.target.as_deref(), Some("ci"));
    assert_eq!(s.spec, "daily 02:00");
    assert!(
        app.delete_schedule("cfg:feat-1:nightly")
            .unwrap_err()
            .to_string()
            .contains("aiw.json")
    );
    // Pausing it here is allowed and survives a resync.
    app.set_schedule_paused("cfg:feat-1:nightly", true).unwrap();
    write(json!([
        {"name": "nightly", "match": "my-*", "when": "daily 03:00", "command": "make test", "terminal": "ci"}
    ]));
    app.sync_config_schedules();
    let s = sched(&app, "cfg:feat-1:nightly");
    assert_eq!(s.spec, "daily 03:00", "the config change is picked up");
    assert!(s.paused, "the pause is kept");
    // Removed from the config: gone.
    write(json!([]));
    app.sync_config_schedules();
    assert!(app.schedules.items.is_empty());
}

#[test]
fn the_dialog_adds_and_edits_a_schedule() {
    let (mut app, claude) = app_on(MockTmuxOps::new());
    app.selection = Selection::Session(0, 0, 0);
    app.open_schedules_for_selection();
    assert!(
        matches!(&app.mode, AppMode::Schedules(s) if matches!(&s.scope, ScheduleScope::Session { session_id, .. } if *session_id == claude))
    );
    app.open_schedule_editor(None);
    {
        let AppMode::ScheduleEditor(state) = &mut app.mode else {
            panic!("editor not open");
        };
        assert_eq!(state.kind, ScheduleKind::Prompt);
        assert_eq!(
            state.targets[state.target_index].0.as_deref(),
            Some(claude.as_str())
        );
        assert_eq!(state.focus, EditorField::When);
        state.spec = "every 15m".into();
    }
    // Empty prompt: an error, the dialog stays.
    app.submit_schedule_editor();
    assert!(
        matches!(&app.mode, AppMode::ScheduleEditor(s) if s.error.as_deref().is_some_and(|e| e.contains("prompt is empty")))
    );
    if let AppMode::ScheduleEditor(state) = &mut app.mode {
        state.text = "check CI".into();
        state.max_runs = "3".into();
    }
    app.submit_schedule_editor();
    assert!(matches!(app.mode, AppMode::Schedules(_)));
    let s = app.schedules.items[0].clone();
    assert_eq!(s.session_id.as_deref(), Some(claude.as_str()));
    assert_eq!(s.text, "check CI");
    assert_eq!(s.max_runs, Some(3));
    let AppMode::Schedules(list) = &app.mode else {
        unreachable!()
    };
    let rows = app.schedule_rows(list);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].target, "→ Claude 1");
    assert!(rows[0].next.is_some());

    // Edit: switch it to a shell command in `tests`.
    app.open_schedule_editor(Some(s.id.clone()));
    if let AppMode::ScheduleEditor(state) = &mut app.mode {
        assert_eq!(state.text, "check CI");
        state.focus = EditorField::Kind;
        state.cycle(true);
        state.terminal = "tests".into();
        state.text = "cargo test".into();
    }
    app.submit_schedule_editor();
    let edited = app.schedule(&s.id).unwrap();
    assert_eq!(edited.kind, ScheduleKind::Shell);
    assert_eq!(edited.target.as_deref(), Some("tests"));
    assert!(edited.session_id.is_none());
}

#[test]
fn the_project_view_lists_every_feature_and_refuses_add() {
    let (mut app, claude) = app_on(MockTmuxOps::new());
    add_prompt(&mut app, &claude, "every 1m", "a", 0);
    app.selection = Selection::Project(0);
    app.open_schedules_for_selection();
    let AppMode::Schedules(list) = &app.mode else {
        panic!()
    };
    assert_eq!(app.schedule_rows(list).len(), 1);
    app.open_schedule_editor(None);
    assert!(
        matches!(app.mode, AppMode::Schedules(_)),
        "no editor without a feature"
    );
}

#[test]
fn automation_adds_lists_pauses_runs_and_deletes() {
    let (tmux, pasted) = recording_tmux();
    let (mut app, _claude) = app_on(tmux);
    let added = app.automation_schedule(&json!({
        "op": "add", "feature": "my-feat", "spec": "every 30m", "prompt": "status?",
        "target": "Claude 1", "max_runs": 5
    }));
    assert_eq!(added["ok"], true, "{added}");
    let id = added["schedule"]["id"].as_str().unwrap().to_string();
    assert_eq!(added["schedule"]["target"], "→ Claude 1");
    assert_eq!(added["schedule"]["max_runs"], 5);

    let listed = app.automation_schedule(&json!({"op": "list", "feature": "my-project/my-feat"}));
    assert_eq!(listed["schedules"].as_array().unwrap().len(), 1);

    let paused = app.automation_schedule(&json!({"op": "pause", "id": &id[..8]}));
    assert_eq!(paused["paused"], true, "{paused}");
    let run = app.automation_schedule(&json!({"op": "run", "id": id}));
    assert_eq!(run["outcome"], "delivered", "{run}");
    assert_eq!(pasted.lock().unwrap().len(), 1);
    let history = app.automation_schedule(&json!({"op": "history", "id": id}));
    assert_eq!(history["runs"][0]["detail"], "automation");

    let deleted = app.automation_schedule(&json!({"op": "delete", "id": id}));
    assert_eq!(deleted["deleted"], true);
    assert!(app.schedules.items.is_empty());

    let bad = app.automation_schedule(
        &json!({"op": "add", "feature": "my-feat", "spec": "nope", "prompt": "x"}),
    );
    assert_eq!(bad["ok"], false);
    let none = app.automation_schedule(
        &json!({"op": "add", "feature": "nope", "spec": "every 1h", "prompt": "x"}),
    );
    assert!(none["error"].as_str().unwrap().contains("no feature"));
    let unknown = app.automation_schedule(&json!({"op": "frobnicate"}));
    assert!(
        unknown["error"]
            .as_str()
            .unwrap()
            .contains("unknown schedule op")
    );
}

#[test]
fn a_failed_paste_is_recorded_as_failed() {
    let mut tmux = MockTmuxOps::new();
    tmux.expect_paste_text()
        .returning(|_, _, _| Err(anyhow::anyhow!("no such pane")));
    let (mut app, claude) = app_on(tmux);
    let id = add_prompt(&mut app, &claude, "every 1m", "x", 0);
    let outcome = app.run_schedule_now(&id, Trigger::RunNow).unwrap();
    assert!(matches!(outcome, Outcome::Failed(ref why) if why.contains("no such pane")));
    assert_eq!(sched(&app, &id).runs, 0);
}
