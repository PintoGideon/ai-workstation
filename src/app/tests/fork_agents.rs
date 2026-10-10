//! FORK: Gemini CLI / Cursor agent wiring through AIW's own paths.

use super::support::*;
use crate::app::attention::{HarnessCapabilities, classify_notification};
use crate::app::*;
use crate::project::{AgentKind, SessionKind};
use crate::traits::{MockTmuxOps, MockWorktreeOps};
use tempfile::{NamedTempFile, TempDir};

fn start_feature_with(agent: AgentKind, window: &'static str, expected: Vec<&'static str>) {
    let workdir = TempDir::new().unwrap();
    let mut store = store_with_feature(ProjectStatus::Stopped);
    store.projects[0].features[0].agent = agent.clone();
    store.projects[0].features[0].mode = VibeMode::SuperVibe;
    store.projects[0].features[0].workdir = workdir.path().to_path_buf();
    store.projects[0].preferred_agent = agent.clone();

    let mut tmux = MockTmuxOps::new();
    tmux.expect_session_exists().return_const(false);
    tmux.expect_create_session_with_window()
        .withf(move |session, w, _| session == "aiw-my-feat" && w == window)
        .times(1)
        .returning(|_, _, _| Ok(()));
    tmux.expect_set_session_env().returning(|_, _, _| Ok(()));
    tmux.expect_launch_program()
        .withf(move |session, w, feature_session_id, _program, args| {
            session == "aiw-my-feat"
                && w == window
                && !feature_session_id.is_empty()
                && args.iter().map(String::as_str).collect::<Vec<_>>() == expected
        })
        .times(1)
        .returning(|_, _, _, _, _| Ok(()));
    tmux.expect_select_window().returning(|_, _| Ok(()));

    let tmp = NamedTempFile::new().unwrap();
    let mut app = App::new_for_test(store, Box::new(tmux), Box::new(MockWorktreeOps::new()));
    app.store_path = tmp.path().to_path_buf();
    app.selection = Selection::Feature(0, 0);
    app.enter_view_without_auto_compose().unwrap();

    match &app.mode {
        AppMode::Viewing(view) => {
            assert_eq!(
                view.session_kind,
                crate::app::session_ops::session_kind_for_agent(&agent)
            );
            assert_eq!(view.window, window);
        }
        _ => panic!("expected Viewing mode"),
    }
}

#[test]
fn starting_a_gemini_feature_launches_gemini_in_yolo_mode() {
    start_feature_with(AgentKind::Gemini, "gemini", vec!["--approval-mode", "yolo"]);
}

#[test]
fn starting_a_cursor_feature_launches_agent_with_force() {
    start_feature_with(AgentKind::Cursor, "cursor", vec!["--force"]);
}

#[test]
fn hook_reported_session_id_becomes_the_resume_id() {
    for (agent, kind, window) in [
        (AgentKind::Gemini, SessionKind::Gemini, "gemini"),
        (AgentKind::Cursor, SessionKind::Cursor, "cursor"),
    ] {
        let workdir = TempDir::new().unwrap();
        let store = super::status_sidebar::store_with_single_agent_session(
            workdir.path(),
            agent,
            kind,
            "fs-1",
            window,
        );
        let mut app = App::new_for_test(
            store,
            Box::new(MockTmuxOps::new()),
            Box::new(MockWorktreeOps::new()),
        );
        app.handle_ipc_message_value(serde_json::json!({
            "type": "thinking-start",
            "session_id": "aiw-my-feat",
            "provider_session_id": "provider-abc",
            "aiw_feature_session_id": "fs-1",
            "cwd": workdir.path().display().to_string()
        }));
        let session = &app.store.projects[0].features[0].sessions[0];
        assert_eq!(session.claude_session_id.as_deref(), Some("provider-abc"));
        assert_eq!(
            crate::app::session_ops::persisted_resume_id(session).as_deref(),
            Some("provider-abc")
        );
        assert!(session.token_usage_source.is_none());
    }
}

#[test]
fn cursor_conversation_id_is_the_provider_session_id() {
    let env = crate::hook_payload::HookEnv {
        aiw_session: Some("aiw-my-feat".into()),
        ..Default::default()
    };
    let payload = crate::hook_payload::build_payload(
        r#"{"conversation_id":"chat-9","hook_event_name":"stop","status":"completed"}"#,
        &env,
        &crate::hook_payload::NotifyOptions::default(),
    )
    .unwrap();
    assert_eq!(payload["provider_session_id"], "chat-9");
    assert_eq!(payload["session_id"], "aiw-my-feat");
}

#[test]
fn attention_states_per_agent() {
    use crate::app::attention::AttentionState::*;
    let gemini = HarnessCapabilities::for_agent(&AgentKind::Gemini);
    assert_eq!(gemini.resolve(Question), Question);
    assert_eq!(
        gemini.resolve(CompletedAwaitingReview),
        CompletedAwaitingReview
    );
    assert!(!gemini.clears_on_open());
    // Gemini's Notification payload for a permission prompt.
    assert_eq!(
        classify_notification(Some("ToolPermission"), Some("Allow run_shell_command?")),
        Some(Question)
    );

    let cursor = HarnessCapabilities::for_agent(&AgentKind::Cursor);
    assert_eq!(cursor.resolve(Question), Waiting);
    assert_eq!(
        cursor.resolve(CompletedAwaitingReview),
        CompletedAwaitingReview
    );
    assert!(!cursor.clears_on_open());
}

#[test]
fn gemini_hooks_land_in_gemini_settings_not_claude_settings() {
    let dir = TempDir::new().unwrap();
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(dir.path())
        .args(["init", "-q"])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .unwrap();
    assert!(status.success());

    crate::app::setup::ensure_notification_hooks(
        dir.path(),
        dir.path(),
        &VibeMode::Vibeless,
        &AgentKind::Gemini,
        false,
    );
    crate::app::setup::ensure_notification_hooks(
        dir.path(),
        dir.path(),
        &VibeMode::Vibeless,
        &AgentKind::Cursor,
        false,
    );

    let settings: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join(".gemini/settings.json")).unwrap(),
    )
    .unwrap();
    assert!(settings["hooks"]["AfterAgent"].is_array());
    assert!(dir.path().join(".cursor/hooks.json").exists());
    assert!(!dir.path().join(".claude/settings.local.json").exists());

    let exclude = std::fs::read_to_string(dir.path().join(".git/info/exclude")).unwrap();
    assert!(exclude.lines().any(|l| l == ".gemini/settings.json"));
    assert!(exclude.lines().any(|l| l == ".cursor/hooks.json"));

    crate::app::setup::cleanup_agent_injected_files(dir.path(), &AgentKind::Gemini);
    let cleaned: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join(".gemini/settings.json")).unwrap(),
    )
    .unwrap();
    assert!(cleaned.get("hooks").is_none());
}

#[test]
fn tracked_gemini_settings_are_not_rewritten() {
    let dir = TempDir::new().unwrap();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(out.status.success(), "{args:?}: {out:?}");
    };
    git(&["init", "-q"]);
    std::fs::create_dir_all(dir.path().join(".gemini")).unwrap();
    std::fs::write(dir.path().join(".gemini/settings.json"), "{}\n").unwrap();
    git(&["add", ".gemini/settings.json"]);

    crate::fork::extra_agents::ensure_hooks(dir.path(), &AgentKind::Gemini);
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".gemini/settings.json")).unwrap(),
        "{}\n"
    );
}
