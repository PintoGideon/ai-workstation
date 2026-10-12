//! FORK: session names, conversation forks and the Claude roster at the
//! App level (DESIGN.md §19.2).

use super::support::*;
use crate::app::*;
use crate::fork::session_identity::ForkSource;
use crate::project::{AgentKind, SessionKind};
use crate::token_tracking::{TokenUsageProvider, TokenUsageSource};
use crate::traits::{MockTmuxOps, MockWorktreeOps};

fn has_pair(args: &[String], flag: &str, value_pred: impl Fn(&str) -> bool) -> bool {
    args.windows(2).any(|w| w[0] == flag && value_pred(&w[1]))
}

fn claude_source(id: &str) -> ForkSource {
    ForkSource {
        agent: AgentKind::Claude,
        harness_session_id: id.into(),
        source_label: "Agent".into(),
    }
}

#[test]
fn a_fork_in_the_same_feature_resumes_the_source_with_fork_session_and_its_own_name() {
    let mut store = store_with_feature(ProjectStatus::Active);
    store.projects[0].features[0].add_session_named(SessionKind::Claude, "Agent".into());
    store.projects[0].features[0].sessions[0].claude_session_id = Some("conv-1".into());

    let mut tmux = MockTmuxOps::new();
    tmux.expect_session_exists().return_const(true);
    tmux.expect_create_window()
        .times(1)
        .returning(|_, _, _| Ok(()));
    tmux.expect_launch_claude()
        .withf(|session, window, _, resume_id, args| {
            session == "aiw-my-feat"
                && window != "claude"
                && resume_id.as_deref() == Some("conv-1")
                && args.iter().any(|a| a == "--fork-session")
                && has_pair(args, "--name", |n| n == "my-feat-agent-fork")
        })
        .times(1)
        .returning(|_, _, _, _, _| Ok(()));
    tmux.expect_kill_window().never();
    let mut app = App::new_for_test(store, Box::new(tmux), Box::new(MockWorktreeOps::new()));
    let tmp = tempfile::NamedTempFile::new().unwrap();
    app.store_path = tmp.path().to_path_buf();

    app.fork_session_here(0, 0, claude_source("conv-1"))
        .unwrap();

    let feature = &app.store.projects[0].features[0];
    assert_eq!(feature.sessions.len(), 2);
    assert_eq!(feature.sessions[1].label, "Agent fork");
    // The source keeps its own conversation.
    assert_eq!(
        feature.sessions[0].claude_session_id.as_deref(),
        Some("conv-1")
    );
    assert!(matches!(app.selection, Selection::Session(0, 0, 1)));
    assert!(app.session_identity.pending_forks.is_empty());
}

#[test]
fn a_codex_fork_runs_codex_fork_with_the_thread_id() {
    let mut store = store_with_feature(ProjectStatus::Active);
    {
        let feature = &mut store.projects[0].features[0];
        feature.mode = VibeMode::Vibe;
        feature.add_session_named(SessionKind::Codex, "Codex 1".into());
        feature.sessions[0].set_token_usage_source_exact(TokenUsageSource {
            provider: TokenUsageProvider::Codex,
            id: "thread-7".into(),
        });
    }
    let mut tmux = MockTmuxOps::new();
    tmux.expect_session_exists().return_const(true);
    tmux.expect_create_window()
        .times(1)
        .returning(|_, _, _| Ok(()));
    tmux.expect_launch_codex().never();
    tmux.expect_launch_program()
        .withf(|session, _, _, program, args| {
            session == "aiw-my-feat"
                && program == "codex"
                && args.len() >= 2
                && args[args.len() - 2] == "fork"
                && args[args.len() - 1] == "thread-7"
        })
        .times(1)
        .returning(|_, _, _, _, _| Ok(()));
    let mut app = App::new_for_test(store, Box::new(tmux), Box::new(MockWorktreeOps::new()));
    let tmp = tempfile::NamedTempFile::new().unwrap();
    app.store_path = tmp.path().to_path_buf();

    let source = app.fork_source_for(0, 0, 0).expect("a saved Codex thread");
    assert_eq!(source.harness_session_id, "thread-7");
    app.fork_session_here(0, 0, source).unwrap();
    assert_eq!(
        app.store.projects[0].features[0].sessions[1].label,
        "Codex 1 fork"
    );
}

#[test]
fn every_new_claude_session_is_launched_with_a_stable_unique_name() {
    let mut store = store_with_feature(ProjectStatus::Active);
    store.projects[0].features[0].add_session_named(SessionKind::Claude, "Agent".into());
    let mut tmux = MockTmuxOps::new();
    tmux.expect_session_exists().return_const(true);
    tmux.expect_create_window().returning(|_, _, _| Ok(()));
    tmux.expect_launch_claude()
        .withf(|_, _, _, resume_id, args| {
            resume_id.is_none()
                && !args.iter().any(|a| a == "--fork-session")
                && has_pair(args, "--name", |n| n.starts_with("my-feat-review"))
        })
        .times(2)
        .returning(|_, _, _, _, _| Ok(()));
    let mut app = App::new_for_test(store, Box::new(tmux), Box::new(MockWorktreeOps::new()));
    let tmp = tempfile::NamedTempFile::new().unwrap();
    app.store_path = tmp.path().to_path_buf();
    let dir = tempfile::TempDir::new().unwrap();
    app.db = Some(crate::db::AiwDb::open(&dir.path().join("aiw.db")).unwrap());

    let (_, first, _) = app
        .create_agent_session_labeled_identified(0, 0, "Review", None, StartIntent::Approved)
        .unwrap();
    let (_, second, _) = app
        .create_agent_session_labeled_identified(0, 0, "Review", None, StartIntent::Approved)
        .unwrap();
    let a = app.session_harness_name(&first).unwrap().to_string();
    let b = app.session_harness_name(&second).unwrap().to_string();
    assert_eq!(a, "my-feat-review");
    assert_ne!(a, b);
    app.persist_session_names();
    let stored = app.db.as_ref().unwrap().all_session_names().unwrap();
    assert_eq!(stored.get(&first), Some(&a));
    assert_eq!(stored.get(&second), Some(&b));
}

#[test]
fn a_new_worktree_fork_hands_the_conversation_to_the_features_first_launch() {
    // A feature with no sessions yet, as `create_forked_feature` leaves it.
    let store = store_with_feature(ProjectStatus::Stopped);
    let mut tmux = MockTmuxOps::new();
    tmux.expect_session_exists().return_const(false);
    tmux.expect_create_session_with_window()
        .times(1)
        .returning(|_, _, _| Ok(()));
    tmux.expect_set_session_env()
        .times(1)
        .returning(|_, _, _| Ok(()));
    tmux.expect_launch_claude()
        .withf(|session, window, _, resume_id, args| {
            session == "aiw-my-feat"
                && window == "claude"
                && resume_id.as_deref() == Some("conv-src")
                && args.iter().any(|a| a == "--fork-session")
                && has_pair(args, "--name", |n| n == "my-feat-claude-1")
        })
        .times(1)
        .returning(|_, _, _, _, _| Ok(()));
    tmux.expect_select_window().returning(|_, _| Ok(()));
    let mut app = App::new_for_test(store, Box::new(tmux), Box::new(MockWorktreeOps::new()));
    app.store.projects[0].features[0].agent = AgentKind::Claude;
    app.session_identity.pending_feature_forks.insert(
        ("my-project".into(), "my-feat".into()),
        claude_source("conv-src"),
    );

    app.do_start_feature(0, 0, None).unwrap();

    assert!(app.session_identity.pending_feature_forks.is_empty());
    assert!(app.session_identity.pending_forks.is_empty(), "used up");
}

#[test]
fn a_restart_resumes_the_saved_conversation_not_the_fork_source() {
    // After its first launch a forked session has its own conversation; the
    // fork source is never applied twice.
    let mut store = store_with_feature(ProjectStatus::Stopped);
    store.projects[0].features[0].add_session_named(SessionKind::Claude, "Agent fork".into());
    store.projects[0].features[0].sessions[0].claude_session_id = Some("conv-own".into());
    let mut tmux = MockTmuxOps::new();
    tmux.expect_session_exists().return_const(false);
    tmux.expect_create_session_with_window()
        .returning(|_, _, _| Ok(()));
    tmux.expect_set_session_env().returning(|_, _, _| Ok(()));
    tmux.expect_launch_claude()
        .withf(|_, _, _, resume_id, args| {
            resume_id.as_deref() == Some("conv-own") && !args.iter().any(|a| a == "--fork-session")
        })
        .times(1)
        .returning(|_, _, _, _, _| Ok(()));
    tmux.expect_select_window().returning(|_, _| Ok(()));
    let mut app = App::new_for_test(store, Box::new(tmux), Box::new(MockWorktreeOps::new()));
    app.do_start_feature(0, 0, None).unwrap();
}

#[test]
fn a_roster_wait_turns_an_aiw_session_into_needs_you() {
    use crate::fork::badges::{BadgeKind, session_badge};
    use crate::fork::claude_roster::RosterEntry;

    let mut store = store_with_feature(ProjectStatus::Active);
    store.projects[0].features[0].add_session_named(SessionKind::Claude, "Agent".into());
    store.projects[0].features[0].sessions[0].claude_session_id = Some("conv-1".into());
    let mut tmux = MockTmuxOps::new();
    tmux.expect_session_exists().return_const(true);
    tmux.expect_window_exists().return_const(true);
    let mut app = App::new_for_test(store, Box::new(tmux), Box::new(MockWorktreeOps::new()));
    let now = chrono::Utc::now();
    assert_ne!(session_badge(&app, 0, 0, 0, now).kind, BadgeKind::NeedsYou);
    app.apply_claude_roster(vec![RosterEntry {
        cwd: "/tmp/test-workdir".into(),
        kind: "interactive".into(),
        session_id: Some("conv-1".into()),
        status: Some("waiting".into()),
        waiting_for: Some("dialog open".into()),
        ..Default::default()
    }]);
    let badge = session_badge(&app, 0, 0, 0, now);
    assert_eq!(badge.kind, BadgeKind::NeedsYou);
    assert_eq!(badge.detail.as_deref(), Some("dialog open"));
    // Its own session never shows as an external line.
    assert!(app.external_sessions_for_feature("feat-1").is_empty());

    // An older "finished" record does not hide a session that waits now…
    use crate::app::attention::{AttentionRecord, AttentionState};
    app.attention.insert(
        "aiw-my-feat".into(),
        AttentionRecord::new(
            AgentKind::Claude,
            AttentionState::CompletedAwaitingReview,
            now,
        ),
    );
    let badge = session_badge(&app, 0, 0, 0, now);
    assert_eq!(badge.kind, BadgeKind::NeedsYou);
    assert_eq!(badge.detail.as_deref(), Some("dialog open"));
    // …while a hook-reported question keeps its own detail.
    app.attention.insert(
        "aiw-my-feat".into(),
        AttentionRecord::new(AgentKind::Claude, AttentionState::Question, now),
    );
    let badge = session_badge(&app, 0, 0, 0, now);
    assert_eq!(badge.kind, BadgeKind::NeedsYou);
    assert_ne!(badge.detail.as_deref(), Some("dialog open"));
}
