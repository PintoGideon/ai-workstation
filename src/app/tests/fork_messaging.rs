//! FORK: `@` messages and `aiw automation message` at the App level
//! (DESIGN.md §19.5).

use std::io::{BufRead, BufReader};
use std::os::unix::net::UnixListener;
use std::path::Path;

use serde_json::{Value, json};

use super::support::*;
use crate::app::*;
use crate::fork::messaging::Delivery;
use crate::project::SessionKind;
use crate::traits::{MockTmuxOps, MockWorktreeOps};

const TOKEN: &str = "0123456789abcdef0123456789abcdef";

/// One feature (`my-feat`) with `Claude 1` and `Codex 1`.
fn app() -> (App, String, String) {
    let mut store = store_with_feature(ProjectStatus::Active);
    let feature = &mut store.projects[0].features[0];
    feature.add_session_named(SessionKind::Claude, "Claude 1".into());
    feature.add_session_named(SessionKind::Codex, "Codex 1".into());
    feature.sessions[0].claude_session_id = Some("conv-1".into());
    let claude = feature.sessions[0].id.clone();
    let codex = feature.sessions[1].id.clone();
    let app = App::new_for_test(
        store,
        Box::new(MockTmuxOps::new()),
        Box::new(MockWorktreeOps::new()),
    );
    (app, claude, codex)
}

fn session_start(app: &mut App, session_id: &str, socket: &Path) {
    app.handle_ipc_message_value(json!({
        "type": "hook-event",
        "kind": "session-start",
        "harness": "claude",
        "session_id": "conv-1",
        "aiw_feature_session_id": session_id,
        "messaging_socket": socket.to_string_lossy(),
        "messaging_token": TOKEN,
    }));
}

/// A listening inbox that collects the lines of the first connection.
fn inbox(dir: &Path) -> (std::path::PathBuf, std::thread::JoinHandle<Vec<Value>>) {
    let path = dir.join("inbox.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let reader = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        BufReader::new(stream)
            .lines()
            .map(|l| serde_json::from_str(&l.unwrap()).unwrap())
            .collect()
    });
    (path, reader)
}

#[test]
fn a_claude_session_with_an_inbox_gets_the_message_there_and_nothing_is_queued() {
    let (mut app, claude, _) = app();
    let dir = tempfile::TempDir::new().unwrap();
    let (socket, reader) = inbox(dir.path());
    session_start(&mut app, &claude, &socket);
    assert!(app.inbox_for(&claude).is_some());
    assert!(matches!(
        app.message_route(&claude),
        crate::fork::messaging::Route::Inbox
    ));

    let delivery = app.send_session_message(&claude, "  rebase on main  ", "aiw");
    assert_eq!(delivery, Delivery::Inbox);
    let lines = reader.join().unwrap();
    assert_eq!(lines.len(), 2, "auth line, then the message: {lines:?}");
    assert_eq!(lines[0]["type"], "auth");
    assert_eq!(lines[0]["token"], TOKEN);
    assert_eq!(lines[1]["type"], "user");
    assert_eq!(lines[1]["message"]["content"], "rebase on main");
    assert_eq!(lines[1]["from"], "aiw");
    assert_eq!(lines[1]["session_id"], "conv-1");
    assert_eq!(app.queued_followup_count(&claude), 0);
}

#[test]
fn codex_and_a_claude_without_an_inbox_fall_back_to_the_follow_up_queue() {
    let (mut app, claude, codex) = app();
    let delivery = app.send_session_message(&codex, "run the tests", "my-feat-claude-1");
    match &delivery {
        Delivery::Queued { position, reason } => {
            assert_eq!(*position, 1);
            assert!(reason.contains("no inbox"), "{reason}");
        }
        other => panic!("expected queued, got {other:?}"),
    }
    // The queue pastes the text as a prompt, so the sender is named.
    let taken = app.take_followup(&codex).unwrap().unwrap();
    assert_eq!(taken.text, "Message from my-feat-claude-1:\nrun the tests");

    let delivery = app.send_session_message(&claude, "hello", "aiw");
    assert_eq!(delivery.status(), "queued");
    assert_eq!(app.followups_for(&claude)[0].text, "hello");
}

#[test]
fn a_stale_inbox_is_forgotten_and_the_message_is_queued_instead() {
    let (mut app, claude, _) = app();
    let dir = tempfile::TempDir::new().unwrap();
    let socket = dir.path().join("gone.sock");
    drop(UnixListener::bind(&socket).unwrap()); // a socket file nobody listens on
    session_start(&mut app, &claude, &socket);
    let delivery = app.send_session_message(&claude, "still there?", "aiw");
    match delivery {
        Delivery::Queued { reason, .. } => {
            assert!(reason.contains("inbox unreachable"), "{reason}")
        }
        other => panic!("expected queued, got {other:?}"),
    }
    assert!(app.inbox_for(&claude).is_none());
    assert_eq!(app.queued_followup_count(&claude), 1);
}

#[test]
fn inbox_delivery_can_be_turned_off() {
    let (mut app, claude, _) = app();
    let dir = tempfile::TempDir::new().unwrap();
    let (socket, _reader) = inbox(dir.path());
    session_start(&mut app, &claude, &socket);
    app.config.aiw.messaging_inbox = false;
    let delivery = app.send_session_message(&claude, "via the queue", "aiw");
    assert_eq!(delivery.status(), "queued");
}

#[test]
fn session_end_and_deleting_the_session_forget_the_inbox() {
    let (mut app, claude, _) = app();
    let dir = tempfile::TempDir::new().unwrap();
    let socket = dir.path().join("s.sock");
    let _listener = UnixListener::bind(&socket).unwrap();
    session_start(&mut app, &claude, &socket);
    app.handle_ipc_message_value(json!({
        "type": "hook-event", "kind": "session-end", "harness": "claude",
        "session_id": "conv-1", "aiw_feature_session_id": claude, "reason": "exit",
    }));
    assert!(app.inbox_for(&claude).is_none());

    session_start(&mut app, &claude, &socket);
    app.forget_session_identity(&claude);
    assert!(app.inbox_for(&claude).is_none());
}

#[test]
fn bad_input_fails_without_queuing() {
    let (mut app, claude, _) = app();
    assert_eq!(
        app.send_session_message(&claude, "   ", "aiw").status(),
        "failed"
    );
    assert_eq!(
        app.send_session_message("nope", "hi", "aiw").status(),
        "failed"
    );
    assert_eq!(app.queued_followup_count(&claude), 0);
}

#[test]
fn inboxes_survive_a_restart_without_their_token() {
    let (mut app, claude, _) = app();
    let dir = tempfile::TempDir::new().unwrap();
    let db_path = dir.path().join("aiw.db");
    app.db = Some(crate::db::AiwDb::open(&db_path).unwrap());
    let socket = dir.path().join("s.sock");
    let _listener = UnixListener::bind(&socket).unwrap();
    session_start(&mut app, &claude, &socket);

    // A second App over the same store, as after a restart.
    let mut again = App::new_for_test(
        app.store.clone(),
        Box::new(MockTmuxOps::new()),
        Box::new(MockWorktreeOps::new()),
    );
    again.db = Some(crate::db::AiwDb::open(&db_path).unwrap());
    again.load_inbox_addresses();
    let address = again.inbox_for(&claude).expect("reloaded");
    assert_eq!(address.socket, socket);
    assert!(address.token.is_none());
    let raw = std::fs::read(&db_path).unwrap();
    assert!(
        !raw.windows(TOKEN.len()).any(|w| w == TOKEN.as_bytes()),
        "the token reached the database file"
    );
}

#[test]
fn the_dialog_opens_on_the_selected_agent_and_sends_to_it() {
    let (mut app, claude, codex) = app();
    app.selection = Selection::Session(0, 0, 1);
    app.open_message_dialog();
    let AppMode::MessageSession(state) = &mut app.mode else {
        panic!("dialog did not open");
    };
    assert_eq!(state.targets.len(), 2);
    assert_eq!(state.target().unwrap().session_id, codex);
    assert!(state.target().unwrap().route.starts_with("queue"));
    state.move_target(1);
    assert_eq!(state.target().unwrap().session_id, claude, "wraps around");
    state.move_target(-1);
    state.text = "check the build".into();

    app.submit_message_dialog();
    assert!(matches!(app.mode, AppMode::Normal));
    assert_eq!(app.followups_for(&codex)[0].text, "check the build");
}

#[test]
fn an_empty_message_keeps_the_dialog_open() {
    let (mut app, _, _) = app();
    app.selection = Selection::Session(0, 0, 0);
    app.open_message_dialog();
    app.submit_message_dialog();
    assert!(matches!(app.mode, AppMode::MessageSession(_)));
}

#[test]
fn automation_addresses_sessions_by_name_and_reports_the_route() {
    let (mut app, claude, codex) = app();
    app.session_identity
        .names
        .assign(&claude, "my-feat", "Claude 1");
    let dir = tempfile::TempDir::new().unwrap();
    let (socket, reader) = inbox(dir.path());
    session_start(&mut app, &claude, &socket);

    let reply = app.automation_message(&json!({
        "action": "message_session", "to": "my-feat-claude-1", "text": "ping", "from": "ci",
    }));
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(reply["status"], "delivered");
    assert_eq!(reply["via"], "inbox");
    assert_eq!(reply["session_id"], claude);
    assert_eq!(reader.join().unwrap()[1]["from"], "ci");

    let reply = app.automation_message(&json!({
        "action": "message_session", "to": "my-feat/Codex 1", "text": "pong",
    }));
    assert_eq!(reply["status"], "queued");
    assert_eq!(reply["via"], "followup-queue");
    assert_eq!(reply["queued"], 1);
    assert_eq!(reply["from"], "aiw");
    assert_eq!(reply["session_id"], codex);

    let reply = app.automation_message(&json!({
        "action": "message_session", "to": "nobody", "text": "x",
    }));
    assert_eq!(reply["ok"], false);
    assert_eq!(reply["status"], "failed");
    assert!(reply["error"].as_str().unwrap().contains("nobody"));
}

#[test]
fn automation_message_goes_through_the_ipc_dispatcher() {
    let (mut app, _, codex) = app();
    app.handle_ipc_message_value(json!({
        "type": crate::automation::AUTOMATION_REQUEST_TYPE,
        "action": "message_session",
        "to": codex,
        "text": "via ipc",
    }));
    assert_eq!(app.followups_for(&codex)[0].text, "via ipc");
}
