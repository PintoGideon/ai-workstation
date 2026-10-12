//! FORK: worktrees Claude made through AIW's `WorktreeCreate` hook, at the
//! App level (DESIGN.md §19.6): loading and ownership, the hook's socket
//! message, sidebar rows, and the `W` dialog's terminal and removal.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::json;

use super::support::*;
use crate::app::*;
use crate::fork::claude_worktrees::{self, ClaudeWorktree, CreateRequest, HookPaths};
use crate::project::SessionKind;
use crate::traits::{MockTmuxOps, MockWorktreeOps};

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

struct Fixture {
    _dir: tempfile::TempDir,
    app: App,
    repo: PathBuf,
    feature_dir: PathBuf,
    claude_session: String,
}

/// A real repo with a feature worktree (`my-feat`, running `Claude 1`), and
/// AIW's store in the temp dir.
fn fixture(tmux: MockTmuxOps) -> Fixture {
    let dir = tempfile::TempDir::new().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    std::fs::write(repo.join("README.md"), "hi\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-qm", "init"]);
    let feature_dir = root.join("feat");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "my-feat",
            &feature_dir.to_string_lossy(),
        ],
    );

    let mut store = store_with_feature(ProjectStatus::Active);
    store.projects[0].repo = repo.clone();
    store.projects[0].is_git = true;
    let feature = &mut store.projects[0].features[0];
    feature.workdir = feature_dir.clone();
    feature.is_worktree = true;
    feature.add_session_named(SessionKind::Claude, "Claude 1".into());
    let claude_session = feature.sessions[0].id.clone();
    let mut app = App::new_for_test(store, Box::new(tmux), Box::new(MockWorktreeOps::new()));
    app.db = Some(crate::db::AiwDb::open(&root.join("aiw.db")).unwrap());
    app.viewport_cols = 40;
    app.viewport_rows = 57;
    app.viewport_total_rows = 60;
    Fixture {
        _dir: dir,
        app,
        repo,
        feature_dir,
        claude_session,
    }
}

fn press(app: &mut App, code: KeyCode) {
    crate::handlers::handle_key(app, KeyEvent::new(code, KeyModifiers::NONE)).unwrap();
}

fn hook_paths(fx: &Fixture) -> HookPaths {
    HookPaths {
        db: fx.app.db.as_ref().unwrap().path.clone(),
        socket: fx.repo.parent().unwrap().join("no.sock"),
    }
}

/// What Claude's hook does: create the worktree and announce it.
fn claude_creates(fx: &mut Fixture, name: &str, aiw_session: Option<String>) -> ClaudeWorktree {
    let request = CreateRequest {
        name: format!("{name}-{}", &uuid::Uuid::new_v4().simple().to_string()[..6]),
        cwd: fx.feature_dir.clone(),
        claude_session_id: Some("conv-1".into()),
        agent_type: None,
        aiw_session_id: aiw_session,
    };
    let wt = claude_worktrees::create(&request, &hook_paths(fx)).unwrap();
    fx.app.handle_ipc_message_value(json!({
        "type": claude_worktrees::IPC_TYPE,
        "event": "created",
        "id": wt.id,
        "path": wt.path.to_string_lossy(),
    }));
    wt
}

#[test]
fn a_created_worktree_is_listed_under_the_session_feature_and_announced() {
    let mut fx = fixture(MockTmuxOps::new());
    let session = fx.claude_session.clone();
    let before = fx.app.workspace_item_height(&VisibleItem::Feature(0, 0));
    let wt = claude_creates(&mut fx, "auth", Some(session));
    let listed = fx.app.claude_worktrees_for_feature("feat-1");
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].path, wt.path);
    assert!(fx.app.claude_worktrees_for_project("my-project").is_empty());
    // One more sidebar line under the feature.
    assert_eq!(
        fx.app.workspace_item_height(&VisibleItem::Feature(0, 0)),
        before + 1
    );
    // A toast says so.
    assert!(
        fx.app
            .toasts
            .iter()
            .any(|t| t.message.contains("Claude made worktree") && t.message.contains("my-feat")),
        "{:?}",
        fx.app
            .toasts
            .iter()
            .map(|t| t.message.clone())
            .collect::<Vec<_>>()
    );
    claude_worktrees::remove(&wt.path, &hook_paths(&fx)).unwrap();
}

#[test]
fn without_a_session_the_requesting_checkout_finds_the_feature() {
    let mut fx = fixture(MockTmuxOps::new());
    let wt = claude_creates(&mut fx, "nosess", None);
    assert_eq!(fx.app.claude_worktrees_for_feature("feat-1").len(), 1);
    // The feature goes away: the worktree falls back to its project.
    fx.app.store.projects[0].features.clear();
    assert_eq!(fx.app.claude_worktrees_for_project("my-project").len(), 1);
    claude_worktrees::remove(&wt.path, &hook_paths(&fx)).unwrap();
}

#[test]
fn a_worktree_made_while_aiw_was_closed_shows_up_at_start_and_a_vanished_one_is_forgotten() {
    let mut fx = fixture(MockTmuxOps::new());
    // Created with no dashboard listening: only the store knows.
    let request = CreateRequest {
        name: format!("closed-{}", &uuid::Uuid::new_v4().simple().to_string()[..6]),
        cwd: fx.feature_dir.clone(),
        claude_session_id: None,
        agent_type: None,
        aiw_session_id: Some(fx.claude_session.clone()),
    };
    let kept = claude_worktrees::create(&request, &hook_paths(&fx)).unwrap();
    let gone = claude_worktrees::create(
        &CreateRequest {
            name: format!("{}-b", request.name),
            ..request.clone()
        },
        &hook_paths(&fx),
    )
    .unwrap();
    std::fs::remove_dir_all(&gone.path).unwrap();
    fx.app.load_claude_worktrees();
    let ids: Vec<&str> = fx
        .app
        .claude_worktrees
        .items
        .iter()
        .map(|wt| wt.id.as_str())
        .collect();
    assert_eq!(ids, [kept.id.as_str()]);
    let stored = fx.app.db.as_ref().unwrap().all_claude_worktrees().unwrap();
    assert_eq!(
        stored.len(),
        1,
        "the vanished one is dropped from the store too"
    );
    claude_worktrees::remove(&kept.path, &hook_paths(&fx)).unwrap();
}

#[test]
fn w_lists_them_and_removes_a_clean_one_after_confirmation() {
    let mut fx = fixture(MockTmuxOps::new());
    let session = fx.claude_session.clone();
    let wt = claude_creates(&mut fx, "rm", Some(session));
    fx.app.selection = Selection::Feature(0, 0);
    press(&mut fx.app, KeyCode::Char('W'));
    let AppMode::ClaudeWorktrees(state) = &fx.app.mode else {
        panic!("W did not open the list");
    };
    assert_eq!(state.ids, std::slice::from_ref(&wt.id));
    assert_eq!(state.title, "my-feat");

    // A dirty worktree is kept, with the reason.
    std::fs::write(wt.path.join("wip.txt"), "x").unwrap();
    press(&mut fx.app, KeyCode::Char('d'));
    assert!(matches!(&fx.app.mode, AppMode::ClaudeWorktrees(s) if s.confirm_remove));
    press(&mut fx.app, KeyCode::Char('y'));
    assert!(wt.path.is_dir());
    assert!(
        fx.app
            .toasts
            .iter()
            .any(|t| t.message.contains("Kept") && t.message.contains("wip.txt"))
    );
    // Any other key cancels a pending removal.
    press(&mut fx.app, KeyCode::Char('d'));
    press(&mut fx.app, KeyCode::Char('n'));
    assert!(matches!(&fx.app.mode, AppMode::ClaudeWorktrees(s) if !s.confirm_remove));
    assert!(wt.path.is_dir());

    // Clean: removed, branch deleted, the list closes when empty.
    std::fs::remove_file(wt.path.join("wip.txt")).unwrap();
    press(&mut fx.app, KeyCode::Char('d'));
    press(&mut fx.app, KeyCode::Char('y'));
    assert!(!wt.path.exists());
    assert!(matches!(fx.app.mode, AppMode::Normal));
    assert!(fx.app.claude_worktrees_for_feature("feat-1").is_empty());
    assert!(
        fx.app
            .toasts
            .iter()
            .any(|t| t.message.contains("deleted branch"))
    );
}

#[test]
fn w_with_nothing_to_list_only_says_so() {
    let mut fx = fixture(MockTmuxOps::new());
    fx.app.selection = Selection::Project(0);
    fx.app.open_claude_worktrees();
    assert!(matches!(fx.app.mode, AppMode::Normal));
    assert!(
        fx.app
            .toasts
            .iter()
            .any(|t| t.message.contains("No worktrees Claude created here"))
    );
}

#[test]
fn t_opens_a_terminal_in_the_worktree_with_its_ports() {
    let typed = Arc::new(Mutex::new(Vec::<String>::new()));
    let mut tmux = MockTmuxOps::new();
    tmux.expect_session_exists().returning(|_| true);
    tmux.expect_create_window().returning(|_, _, _| Ok(()));
    let sink = typed.clone();
    tmux.expect_send_literal().returning(move |_, _, text| {
        sink.lock().unwrap().push(text.to_string());
        Ok(())
    });
    tmux.expect_send_key_name().returning(|_, _, _| Ok(()));
    let mut fx = fixture(tmux);
    let session = fx.claude_session.clone();
    let wt = claude_creates(&mut fx, "term", Some(session));
    fx.app.selection = Selection::Session(0, 0, 0);
    fx.app.open_claude_worktrees();
    press(&mut fx.app, KeyCode::Char('t'));

    let feature = &fx.app.store.projects[0].features[0];
    let terminal = feature
        .sessions
        .iter()
        .find(|s| s.kind == SessionKind::Terminal)
        .expect("a terminal session in the feature");
    assert_eq!(terminal.label, format!("⎇ {}", wt.name));
    let typed = typed.lock().unwrap();
    assert_eq!(typed.len(), 1);
    assert!(typed[0].starts_with(&format!("cd '{}'", wt.path.display())));
    assert!(typed[0].ends_with(". ./.aiw/ports.env"));
    assert!(matches!(fx.app.mode, AppMode::Normal));
    drop(typed);
    claude_worktrees::remove(&wt.path, &hook_paths(&fx)).unwrap();
}
