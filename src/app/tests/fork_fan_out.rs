//! FORK: one prompt, several agents at the App level (DESIGN.md §17, §19.3).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::json;
use tempfile::TempDir;

use super::support::*;
use crate::app::*;
use crate::automation::CreateFeatureRequest;
use crate::fork::fan_out::{FanOutLayout, READY_SETTLE};
use crate::project::{AgentKind, SessionKind, VibeMode};
use crate::traits::{MockTmuxOps, MockWorktreeOps};

/// Everything the lenient tmux saw.
#[derive(Default)]
pub(super) struct Calls {
    pub(super) sessions: HashSet<String>,
    pub(super) launches: Vec<(String, String, String)>, // (harness, session, window)
    pub(super) pastes: Vec<(String, String, String)>,   // (session, window, text)
    pub(super) keys: Vec<(String, String, String)>,     // (session, window, key)
    pub(super) panes: Vec<crate::tmux::PaneCommand>,
    pub(super) shell: Vec<(String, String, String)>, // (session, window, command)
}

pub(super) type Shared = Arc<Mutex<Calls>>;

/// A tmux mock that accepts every call, remembers sessions it created and
/// records launches, pastes and keys.
pub(super) fn lenient_tmux(calls: Shared) -> MockTmuxOps {
    let mut tmux = MockTmuxOps::new();
    tmux.expect_check_harness_available().returning(|_| Ok(()));
    let c = calls.clone();
    tmux.expect_session_exists()
        .returning(move |s| c.lock().unwrap().sessions.contains(s));
    let c = calls.clone();
    tmux.expect_window_exists()
        .returning(move |s, _| c.lock().unwrap().sessions.contains(s));
    tmux.expect_list_sessions().returning(|| Ok(Vec::new()));
    tmux.expect_window_activity().returning(Vec::new);
    tmux.expect_list_panes().returning(Vec::new);
    let c = calls.clone();
    tmux.expect_pane_commands()
        .returning(move || c.lock().unwrap().panes.clone());
    tmux.expect_select_pane().returning(|_| Ok(()));
    let c = calls.clone();
    tmux.expect_create_session_with_window()
        .returning(move |s, _, _| {
            c.lock().unwrap().sessions.insert(s.to_string());
            Ok(())
        });
    tmux.expect_set_session_env().returning(|_, _, _| Ok(()));
    tmux.expect_create_window().returning(|_, _, _| Ok(()));
    let c = calls.clone();
    tmux.expect_launch_claude().returning(move |s, w, _, _, _| {
        c.lock()
            .unwrap()
            .launches
            .push(("claude".into(), s.into(), w.into()));
        Ok(())
    });
    let c = calls.clone();
    tmux.expect_launch_codex().returning(move |s, w, _, _, _| {
        c.lock()
            .unwrap()
            .launches
            .push(("codex".into(), s.into(), w.into()));
        Ok(())
    });
    tmux.expect_launch_pi().returning(|_, _, _| Ok(()));
    tmux.expect_launch_program()
        .returning(|_, _, _, _, _| Ok(()));
    let c = calls.clone();
    tmux.expect_run_shell_command().returning(move |s, w, cmd| {
        c.lock()
            .unwrap()
            .shell
            .push((s.into(), w.into(), cmd.into()));
        Ok(())
    });
    tmux.expect_send_keys().returning(|_, _, _| Ok(()));
    tmux.expect_send_literal().returning(|_, _, _| Ok(()));
    let c = calls.clone();
    tmux.expect_paste_text().returning(move |s, w, t| {
        c.lock()
            .unwrap()
            .pastes
            .push((s.into(), w.into(), t.into()));
        Ok(())
    });
    let c = calls.clone();
    tmux.expect_send_key_name().returning(move |s, w, k| {
        c.lock().unwrap().keys.push((s.into(), w.into(), k.into()));
        Ok(())
    });
    tmux.expect_resize_pane().returning(|_, _, _, _| Ok(()));
    tmux.expect_select_window().returning(|_, _| Ok(()));
    tmux.expect_kill_window().returning(|_, _| Ok(()));
    tmux.expect_kill_session().returning(|_| Ok(()));
    tmux
}

/// Worktrees become folders under `root`.
pub(super) fn folder_worktrees(root: PathBuf) -> MockWorktreeOps {
    let mut wt = MockWorktreeOps::new();
    let r = root.clone();
    wt.expect_create().returning(move |_, name, _| {
        let path = r.join(name);
        std::fs::create_dir_all(&path).unwrap();
        Ok(path)
    });
    wt.expect_repo_root().returning(|p| Ok(p.to_path_buf()));
    wt.expect_remove().returning(|_, _| Ok(()));
    wt
}

pub(super) struct Fixture {
    pub(super) app: App,
    pub(super) calls: Shared,
    _repo: TempDir,
    _wt: TempDir,
    _store: tempfile::NamedTempFile,
    _db: TempDir,
}

/// An app with one empty git project that already has a feature in the
/// repo dir (so new features default to worktrees), a lenient tmux and a
/// real database.
pub(super) fn fixture() -> Fixture {
    let repo = TempDir::new().unwrap();
    let wt = TempDir::new().unwrap();
    let calls: Shared = Arc::default();
    let store = store_with_empty_project(repo.path().to_path_buf(), true);
    let mut app = App::new_for_test(
        store,
        Box::new(lenient_tmux(calls.clone())),
        Box::new(folder_worktrees(wt.path().to_path_buf())),
    );
    let store_file = tempfile::NamedTempFile::new().unwrap();
    app.store_path = store_file.path().to_path_buf();
    let db_dir = TempDir::new().unwrap();
    app.db = Some(crate::db::AiwDb::open(&db_dir.path().join("aiw.db")).unwrap());
    app.config.max_agent_autostart_sessions = 0;
    app.selection = Selection::Project(0);
    Fixture {
        app,
        calls,
        _repo: repo,
        _wt: wt,
        _store: store_file,
        _db: db_dir,
    }
}

pub(super) fn key(app: &mut App, code: KeyCode) {
    crate::handlers::handle_create_feature_key_event(app, KeyEvent::from(code)).unwrap();
}

fn ctrl(app: &mut App, c: char) {
    crate::handlers::handle_create_feature_key_event(
        app,
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL),
    )
    .unwrap();
}

pub(super) fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        key(app, KeyCode::Char(c));
    }
}

pub(super) fn step(app: &App) -> CreateFeatureStep {
    match &app.mode {
        AppMode::CreatingFeature(state) => state.step.clone(),
        other => panic!(
            "expected the dialog, got {:?}",
            std::mem::discriminant(other)
        ),
    }
}

/// Open `n`, name the branch, choose worktree yes/no, then stop on the
/// harness list with Claude highlighted.
pub(super) fn open_dialog(app: &mut App, branch: &str, worktree: bool) {
    app.start_create_feature();
    let AppMode::CreatingFeature(state) = &mut app.mode else {
        panic!("dialog did not open");
    };
    state.allowed_agents = vec![AgentKind::Claude, AgentKind::Codex];
    state.agent_index = 0;
    state.agent = AgentKind::Claude;
    state.use_worktree = worktree;
    state.step = CreateFeatureStep::Branch;
    type_text(app, branch);
    key(app, KeyCode::Enter); // → Worktree
    key(app, KeyCode::Enter); // → Mode (focus 0: harness)
    assert_eq!(step(app), CreateFeatureStep::Mode);
}

/// Tick Claude and Codex on the harness list.
pub(super) fn tick_claude_and_codex(app: &mut App) {
    key(app, KeyCode::Char(' ')); // Claude
    key(app, KeyCode::Char('j')); // → Codex
    key(app, KeyCode::Char(' ')); // Codex
}

/// Walk the Mode step's fields to its end.
fn finish_mode(app: &mut App) {
    for _ in 0..3 {
        if step(app) != CreateFeatureStep::Mode {
            return;
        }
        key(app, KeyCode::Enter);
    }
}

pub(super) fn session_ids(app: &App, feature: &str) -> Vec<(String, SessionKind, String)> {
    app.store.projects[0]
        .features
        .iter()
        .find(|f| f.name == feature)
        .unwrap_or_else(|| panic!("no feature {feature}"))
        .sessions
        .iter()
        .map(|s| (s.label.clone(), s.kind.clone(), s.id.clone()))
        .collect()
}

/// The hook says the session is up; move its ready time into the past.
fn ready_now(app: &mut App, session_id: &str) {
    app.handle_ipc_message_value(json!({
        "type": "hook-event", "kind": "session-start", "harness": "claude",
        "aiw_feature_session_id": session_id, "source": "startup", "cwd": "/tmp"
    }));
    let prompt = app.fan_out.pending.get_mut(session_id).unwrap();
    let ready = prompt.ready_at.expect("marked ready");
    assert!(ready >= Instant::now() + READY_SETTLE - Duration::from_millis(200));
    prompt.ready_at = Some(Instant::now() - Duration::from_millis(1));
}

#[test]
fn ticking_two_harnesses_with_a_worktree_offers_the_layout_then_the_prompt() {
    let mut f = fixture();
    open_dialog(&mut f.app, "auth-fix", true);
    tick_claude_and_codex(&mut f.app);
    if let AppMode::CreatingFeature(state) = &f.app.mode {
        assert_eq!(state.ticked, [AgentKind::Claude, AgentKind::Codex]);
        assert_eq!(
            state.agent,
            AgentKind::Claude,
            "the first ticked is primary"
        );
        assert!(state.is_fan_out());
    }
    finish_mode(&mut f.app);
    assert_eq!(step(&f.app), CreateFeatureStep::FanOut);
    if let AppMode::CreatingFeature(state) = &f.app.mode {
        // Codex has no Vibeless: the set runs in Vibe, with a note.
        assert_eq!(state.mode, VibeMode::Vibe);
        assert!(state.mode_note.is_some());
        assert_eq!(state.fan_out, FanOutLayout::WorktreeEach);
    }
    key(&mut f.app, KeyCode::Char('j'));
    if let AppMode::CreatingFeature(state) = &f.app.mode {
        assert_eq!(state.fan_out, FanOutLayout::Shared);
    }
    key(&mut f.app, KeyCode::Enter);
    assert_eq!(step(&f.app), CreateFeatureStep::Prompt);
    key(&mut f.app, KeyCode::Esc);
    assert_eq!(step(&f.app), CreateFeatureStep::FanOut);
}

#[test]
fn the_repo_dir_skips_the_layout_step_and_shares() {
    let mut f = fixture();
    open_dialog(&mut f.app, "explore", false);
    tick_claude_and_codex(&mut f.app);
    finish_mode(&mut f.app);
    assert_eq!(step(&f.app), CreateFeatureStep::Prompt);
    if let AppMode::CreatingFeature(state) = &f.app.mode {
        assert_eq!(state.effective_fan_out(), FanOutLayout::Shared);
    }
}

#[test]
fn unticking_back_to_one_harness_is_an_ordinary_single_feature() {
    let mut f = fixture();
    open_dialog(&mut f.app, "solo", true);
    tick_claude_and_codex(&mut f.app);
    key(&mut f.app, KeyCode::Char(' ')); // untick Codex
    if let AppMode::CreatingFeature(state) = &f.app.mode {
        assert!(!state.is_fan_out());
        assert_eq!(state.selected_agents(), [AgentKind::Claude]);
    }
    finish_mode(&mut f.app);
    assert_eq!(step(&f.app), CreateFeatureStep::SessionName);
    key(&mut f.app, KeyCode::Enter);
    assert_eq!(step(&f.app), CreateFeatureStep::Prompt);
}

#[test]
fn worktree_each_creates_grouped_siblings_each_with_a_pending_prompt() {
    let mut f = fixture();
    open_dialog(&mut f.app, "auth-fix", true);
    tick_claude_and_codex(&mut f.app);
    finish_mode(&mut f.app);
    key(&mut f.app, KeyCode::Enter); // layout: one worktree each
    type_text(&mut f.app, "Fix the login redirect");
    key(&mut f.app, KeyCode::Enter);

    assert!(matches!(f.app.mode, AppMode::Normal), "the dialog closed");
    let names: Vec<String> = f.app.store.projects[0]
        .features
        .iter()
        .map(|f| f.name.clone())
        .collect();
    assert_eq!(names, ["auth-fix-claude", "auth-fix-codex"]);
    let claude = session_ids(&f.app, "auth-fix-claude");
    let codex = session_ids(&f.app, "auth-fix-codex");
    assert_eq!(claude[0].1, SessionKind::Claude);
    assert_eq!(codex[0].1, SessionKind::Codex);
    for feature in &f.app.store.projects[0].features {
        assert_eq!(feature.mode, VibeMode::Vibe);
        assert!(feature.is_worktree);
    }

    // Both siblings share one group; the first heads it.
    assert_eq!(f.app.fan_out_siblings(0, 0), [0, 1]);
    assert!(f.app.fan_out_heads_group(0, 0));
    assert!(!f.app.fan_out_heads_group(0, 1));
    let (header, needs) = f.app.fan_out_header(0, 0, chrono::Utc::now()).unwrap();
    assert_eq!(header, "⇶ auth-fix · claude / codex");
    assert_eq!(needs, 0);

    // Each agent's prompt waits for its pane; nothing was typed blind.
    for id in [&claude[0].2, &codex[0].2] {
        let pending = f.app.pending_prompt_for(id).expect("pending");
        assert_eq!(pending.text, "Fix the login redirect");
        assert!(!pending.send);
    }
    assert!(f.calls.lock().unwrap().pastes.is_empty());
    let launches = f.calls.lock().unwrap().launches.clone();
    assert!(launches.iter().any(|(h, _, _)| h == "claude"));
    assert!(launches.iter().any(|(h, _, _)| h == "codex"));

    // The primary is selected; the group seats and prompts are stored.
    assert!(matches!(f.app.selection, Selection::Session(0, 0, _)));
    let db = f.app.db.as_ref().unwrap();
    assert_eq!(db.all_fanout_members().unwrap().len(), 2);
    assert_eq!(db.all_pending_prompts().unwrap().len(), 2);
}

#[test]
fn sibling_session_names_stay_unique() {
    let mut f = fixture();
    open_dialog(&mut f.app, "dup", true);
    // Claude twice: siblings dup-claude and dup-claude-2.
    key(&mut f.app, KeyCode::Char(' '));
    if let AppMode::CreatingFeature(state) = &mut f.app.mode {
        state.ticked.push(AgentKind::Claude);
    }
    finish_mode(&mut f.app);
    key(&mut f.app, KeyCode::Enter);
    key(&mut f.app, KeyCode::Enter);
    let ids: Vec<String> = f.app.store.projects[0]
        .features
        .iter()
        .map(|f| f.sessions[0].id.clone())
        .collect();
    assert_eq!(ids.len(), 2);
    let names: HashSet<String> = ids
        .iter()
        .map(|id| f.app.session_harness_name(id).expect("named").to_string())
        .collect();
    assert_eq!(names.len(), 2, "{names:?}");
}

#[test]
fn shared_layout_opens_a_window_per_agent_with_unique_labels() {
    let mut f = fixture();
    open_dialog(&mut f.app, "review", true);
    tick_claude_and_codex(&mut f.app);
    finish_mode(&mut f.app);
    key(&mut f.app, KeyCode::Char('j')); // shared
    key(&mut f.app, KeyCode::Enter);
    type_text(&mut f.app, "Review the auth module");
    ctrl(&mut f.app, 't'); // send when ready
    key(&mut f.app, KeyCode::Enter);

    assert!(matches!(f.app.mode, AppMode::Normal));
    assert_eq!(f.app.store.projects[0].features.len(), 1);
    let sessions = session_ids(&f.app, "review");
    let labels: Vec<&str> = sessions.iter().map(|(l, _, _)| l.as_str()).collect();
    assert_eq!(labels, ["Claude 1", "Codex 1"]);
    assert!(
        f.app.fan_out_siblings(0, 0).is_empty(),
        "no group for shared"
    );
    for (_, _, id) in &sessions {
        let pending = f.app.pending_prompt_for(id).expect("pending");
        assert!(pending.send);
    }
    // Only the primary takes the focus on delivery.
    assert!(f.app.pending_prompt_for(&sessions[0].2).unwrap().focus);
    assert!(!f.app.pending_prompt_for(&sessions[1].2).unwrap().focus);
}

#[test]
fn a_taken_sibling_name_is_refused_before_any_worktree_exists() {
    let mut f = fixture();
    open_dialog(&mut f.app, "taken", true);
    tick_claude_and_codex(&mut f.app);
    finish_mode(&mut f.app);
    key(&mut f.app, KeyCode::Enter);
    // Someone created `taken-codex` meanwhile.
    let repo = f.app.store.projects[0].repo.clone();
    let mut other = crate::project::Feature::new_for_project(
        "automation-project",
        "taken-codex".into(),
        "taken-codex".into(),
        repo,
        true,
        VibeMode::Vibe,
        AgentKind::Codex,
        false,
    );
    other.add_session(SessionKind::Codex);
    f.app.store.add_feature("automation-project", other);
    key(&mut f.app, KeyCode::Enter);

    assert_eq!(f.app.store.projects[0].features.len(), 1, "nothing created");
    match &f.app.mode {
        AppMode::CreatingFeature(state) => {
            assert_eq!(state.branch, "taken", "the dialog keeps the base name");
            assert!(
                state
                    .branch_error
                    .as_deref()
                    .unwrap()
                    .contains("taken-codex")
            );
        }
        _ => panic!("the dialog should stay open"),
    }
    assert!(f.app.fan_out.run.is_none());
}

#[test]
fn a_single_agent_prompt_waits_for_the_session_start_hook_and_is_pasted_once() {
    let mut f = fixture();
    open_dialog(&mut f.app, "solo", true);
    finish_mode(&mut f.app);
    key(&mut f.app, KeyCode::Enter); // session name
    type_text(&mut f.app, "Do the thing");
    key(&mut f.app, KeyCode::Enter);
    let id = session_ids(&f.app, "solo")[0].2.clone();
    assert!(f.app.pending_prompt_for(&id).is_some());

    // Nothing before the hook (and well before the fallback).
    f.app.pending_prompts_tick();
    assert!(f.calls.lock().unwrap().pastes.is_empty());

    ready_now(&mut f.app, &id);
    assert!(f.app.pending_prompts_tick());
    f.app.pending_prompts_tick();
    let pastes = f.calls.lock().unwrap().pastes.clone();
    assert_eq!(pastes.len(), 1, "{pastes:?}");
    assert_eq!(pastes[0].2, "Do the thing");
    assert!(f.calls.lock().unwrap().keys.is_empty(), "unsent: no Enter");
    assert!(f.app.pending_prompt_for(&id).is_none());
    assert!(
        f.app
            .db
            .as_ref()
            .unwrap()
            .all_pending_prompts()
            .unwrap()
            .is_empty()
    );
    // A second SessionStart (a /clear) finds nothing to send.
    f.app.handle_ipc_message_value(json!({
        "type": "hook-event", "kind": "session-start", "harness": "claude",
        "aiw_feature_session_id": id, "source": "clear", "cwd": "/tmp"
    }));
    f.app.pending_prompts_tick();
    assert_eq!(f.calls.lock().unwrap().pastes.len(), 1);
}

#[test]
fn send_when_ready_presses_enter_once_after_the_paste() {
    let mut f = fixture();
    f.app.store.projects[0].features.clear();
    let id = {
        open_dialog(&mut f.app, "sender", true);
        finish_mode(&mut f.app);
        key(&mut f.app, KeyCode::Enter);
        type_text(&mut f.app, "Ship it");
        ctrl(&mut f.app, 't');
        key(&mut f.app, KeyCode::Enter);
        session_ids(&f.app, "sender")[0].2.clone()
    };
    ready_now(&mut f.app, &id);
    f.app.pending_prompts_tick();
    assert_eq!(f.calls.lock().unwrap().pastes.len(), 1);
    assert!(
        f.calls.lock().unwrap().keys.is_empty(),
        "Enter waits a beat"
    );
    std::thread::sleep(crate::fork::fan_out::ENTER_DELAY + Duration::from_millis(50));
    f.app.pending_prompts_tick();
    f.app.pending_prompts_tick();
    let keys = f.calls.lock().unwrap().keys.clone();
    assert_eq!(keys.len(), 1, "{keys:?}");
    assert_eq!(keys[0].2, "Enter");
}

#[test]
fn without_a_hook_the_prompt_goes_once_the_pane_runs_the_agent() {
    let mut f = fixture();
    f.app.config.aiw.fan_out_ready_fallback_secs = 1;
    open_dialog(&mut f.app, "nohook", true);
    finish_mode(&mut f.app);
    key(&mut f.app, KeyCode::Enter);
    type_text(&mut f.app, "Fallback prompt");
    key(&mut f.app, KeyCode::Enter);
    let (session, window, id) = {
        let feature = &f.app.store.projects[0]
            .features
            .iter()
            .find(|f| f.name == "nohook")
            .unwrap();
        (
            feature.tmux_session.clone(),
            feature.sessions[0].tmux_window.clone(),
            feature.sessions[0].id.clone(),
        )
    };
    let pane = |command: &str| crate::tmux::PaneCommand {
        session: session.clone(),
        window: window.clone(),
        pane_id: "%1".into(),
        pid: Some(1),
        command: command.into(),
        shell_has_children: Some(false),
    };
    // The fallback is due, but the pane still runs the launch shell.
    f.app.fan_out.pending.get_mut(&id).unwrap().queued_at = Instant::now() - Duration::from_secs(5);
    f.calls.lock().unwrap().panes = vec![pane("bash")];
    f.app.pending_prompts_tick();
    assert!(f.calls.lock().unwrap().pastes.is_empty());

    // The agent runs, but it must have run for a moment first.
    f.calls.lock().unwrap().panes = vec![pane("claude")];
    std::thread::sleep(Duration::from_millis(1100));
    f.app.pending_prompts_tick();
    assert!(f.calls.lock().unwrap().pastes.is_empty());
    std::thread::sleep(Duration::from_millis(3100));
    f.app.pending_prompts_tick();
    let pastes = f.calls.lock().unwrap().pastes.clone();
    assert_eq!(pastes.len(), 1);
    assert_eq!(pastes[0].2, "Fallback prompt");
}

#[test]
fn pending_prompts_and_groups_survive_a_restart() {
    let dir = TempDir::new().unwrap();
    let db_path = dir.path().join("aiw.db");
    {
        let db = crate::db::AiwDb::open(&db_path).unwrap();
        db.save_pending_prompt("s-1", "Resume me", true).unwrap();
        db.save_fanout_member(&crate::db::fan_out::FanOutMember {
            feature_id: "f-1".into(),
            group_id: "g".into(),
            base_branch: "x".into(),
            agent: AgentKind::Codex,
            position: 0,
        })
        .unwrap();
    }
    let mut app = App::new_for_test(
        store_with_feature(ProjectStatus::Stopped),
        Box::new(MockTmuxOps::new()),
        Box::new(MockWorktreeOps::new()),
    );
    app.db = Some(crate::db::AiwDb::open(&db_path).unwrap());
    app.load_fan_out_state();
    let pending = app.pending_prompt_for("s-1").unwrap();
    assert_eq!(pending.text, "Resume me");
    assert!(pending.send);
    assert!(!pending.focus);
    assert_eq!(app.fan_out.members["f-1"].agent, AgentKind::Codex);
}

#[test]
fn deleting_a_feature_drops_its_prompt_and_group_seat() {
    let mut f = fixture();
    open_dialog(&mut f.app, "gone", true);
    tick_claude_and_codex(&mut f.app);
    finish_mode(&mut f.app);
    key(&mut f.app, KeyCode::Enter);
    type_text(&mut f.app, "p");
    key(&mut f.app, KeyCode::Enter);
    let feature = f.app.store.projects[0].features[0].clone();
    f.app.forget_fan_out_feature(&feature);
    assert!(f.app.pending_prompt_for(&feature.sessions[0].id).is_none());
    // The other sibling is alone in its group now: no header.
    assert_eq!(f.app.fan_out_siblings(0, 1), [1]);
    assert!(!f.app.fan_out_heads_group(0, 1));
    let db = f.app.db.as_ref().unwrap();
    assert_eq!(db.all_fanout_members().unwrap().len(), 1);
    assert_eq!(db.all_pending_prompts().unwrap().len(), 1);
}

#[test]
fn the_prompt_library_inserts_a_template_into_the_prompt() {
    let mut f = fixture();
    // No store file: the library reads the in-memory global templates.
    f.app.store_path = PathBuf::new();
    f.app.config.extension.prompt_templates = vec![crate::prompt_library::PromptTemplate::new(
        "triage".into(),
        "Triage the failing tests".into(),
    )];
    open_dialog(&mut f.app, "lib", true);
    finish_mode(&mut f.app);
    key(&mut f.app, KeyCode::Enter);
    type_text(&mut f.app, "First:");
    ctrl(&mut f.app, 'l');
    key(&mut f.app, KeyCode::Enter);
    match &f.app.mode {
        AppMode::CreatingFeature(state) => {
            assert!(state.prompt_picker.is_none());
            assert_eq!(state.task_prompt, "First:\nTriage the failing tests");
        }
        _ => panic!("dialog"),
    }
    // Alt+Enter is a newline, not create.
    crate::handlers::handle_create_feature_key_event(
        &mut f.app,
        KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT),
    )
    .unwrap();
    assert_eq!(step(&f.app), CreateFeatureStep::Prompt);
}

#[test]
fn automation_fans_out_worktree_each_with_a_prompt() {
    let mut f = fixture();
    let request = CreateFeatureRequest {
        project_name: "automation-project".into(),
        branch: "api".into(),
        agents: vec![AgentKind::Claude, AgentKind::Codex],
        fan_out: Some(FanOutLayout::WorktreeEach),
        use_worktree: Some(true),
        prompt: Some("Add pagination\n".into()),
        send: true,
        ..Default::default()
    };
    let response = f.app.create_feature_from_request_fan_out(&request).unwrap();
    assert!(response.ok);
    assert_eq!(response.features.len(), 2);
    assert_eq!(response.features[0].branch, "api-claude");
    assert_eq!(response.features[1].branch, "api-codex");
    let names: Vec<String> = f.app.store.projects[0]
        .features
        .iter()
        .map(|f| f.name.clone())
        .collect();
    assert_eq!(names, ["api-claude", "api-codex"]);
    assert_eq!(f.app.fan_out_siblings(0, 1), [0, 1]);
    for feature in &f.app.store.projects[0].features {
        let pending = f.app.pending_prompt_for(&feature.sessions[0].id).unwrap();
        assert_eq!(pending.text, "Add pagination");
        assert!(pending.send);
    }
    let rendered = serde_json::to_value(&response).unwrap();
    assert_eq!(rendered["features"][1]["agent"], "codex");
}

#[test]
fn automation_fans_out_shared_and_dry_runs_change_nothing() {
    let mut f = fixture();
    let mut request = CreateFeatureRequest {
        project_name: "automation-project".into(),
        branch: "pair".into(),
        agents: vec![AgentKind::Claude, AgentKind::Codex],
        fan_out: Some(FanOutLayout::Shared),
        use_worktree: Some(true),
        prompt: Some("Pair on this".into()),
        dry_run: true,
        ..Default::default()
    };
    let dry = f.app.create_feature_from_request_fan_out(&request).unwrap();
    assert!(dry.dry_run);
    assert!(f.app.store.projects[0].features.is_empty());
    assert!(f.app.fan_out.pending.is_empty());

    request.dry_run = false;
    let response = f.app.create_feature_from_request_fan_out(&request).unwrap();
    assert_eq!(response.features.len(), 1);
    let labels: Vec<String> = response.features[0]
        .sessions
        .iter()
        .map(|(l, _)| l.clone())
        .collect();
    assert_eq!(labels, ["Claude 1", "Codex 1"]);
    assert_eq!(f.app.fan_out.pending.len(), 2);
}

#[test]
fn automation_refuses_worktree_each_without_worktrees() {
    let mut f = fixture();
    let request = CreateFeatureRequest {
        project_name: "automation-project".into(),
        branch: "nope".into(),
        agents: vec![AgentKind::Claude, AgentKind::Codex],
        fan_out: Some(FanOutLayout::WorktreeEach),
        use_worktree: Some(false),
        ..Default::default()
    };
    let error = f
        .app
        .create_feature_from_request_fan_out(&request)
        .unwrap_err();
    assert!(error.to_string().contains("worktree_each"));
    assert!(f.app.store.projects[0].features.is_empty());
}

#[test]
fn a_preset_can_tick_several_agents_and_pick_the_layout() {
    let mut f = fixture();
    open_dialog(&mut f.app, "x", true);
    if let AppMode::CreatingFeature(state) = &mut f.app.mode {
        state.feature_presets = vec![crate::extension::FeaturePreset {
            name: "duo".into(),
            agents: vec![AgentKind::Codex, AgentKind::Claude],
            fan_out: Some(FanOutLayout::Shared),
            mode: VibeMode::Vibe,
            ..Default::default()
        }];
        state.step = CreateFeatureStep::SelectPreset;
    }
    key(&mut f.app, KeyCode::Enter);
    match &f.app.mode {
        AppMode::CreatingFeature(state) => {
            assert_eq!(state.ticked, [AgentKind::Codex, AgentKind::Claude]);
            assert_eq!(state.agent, AgentKind::Codex);
            assert_eq!(state.fan_out, FanOutLayout::Shared);
        }
        _ => panic!("dialog"),
    }
}

#[test]
fn v_on_a_sibling_tiles_the_whole_group() {
    let mut f = fixture();
    open_dialog(&mut f.app, "tiles", true);
    tick_claude_and_codex(&mut f.app);
    finish_mode(&mut f.app);
    key(&mut f.app, KeyCode::Enter);
    key(&mut f.app, KeyCode::Enter);
    f.app.cockpit_attach(
        crate::fork::cockpit::Link::recording(),
        "/bin/aiw".into(),
        "/s/cockpit.sock".into(),
    );
    f.app.fork_cockpit.link.as_ref().unwrap().clear_recorded();
    f.app.selection = Selection::Feature(0, 1);
    f.app.cockpit_show_grid();
    let requests = f.app.fork_cockpit.link.as_ref().unwrap().recorded();
    let command = requests
        .iter()
        .find_map(|r| match r {
            crate::fork::cockpit::Request::Respawn { command, title } if title.contains("⇶") => {
                Some(command.clone())
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("no group grid in {requests:?}"));
    assert!(
        command
            .contains("grid automation-project/tiles-claude --with automation-project/tiles-codex"),
        "{command}"
    );
}
