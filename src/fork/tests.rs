//! App-level tests for the work-safety fixes (DESIGN.md §7).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::Utc;
use crossterm::event::KeyCode;
use tempfile::TempDir;

use crate::app::{App, AppMode, CreateFeatureState};
use crate::automation::CreateProjectRequest;
use crate::extension::HookConfig;
use crate::fork::hook_approval::{HookName, HookResume, handle_key};
use crate::project::{AgentKind, Project, ProjectStore};
use crate::traits::{MockTmuxOps, MockWorktreeOps};

fn store_with_repo(repo: PathBuf) -> ProjectStore {
    ProjectStore {
        version: 4,
        projects: vec![Project {
            id: "proj-safety".to_string(),
            name: "safety".to_string(),
            repo,
            collapsed: false,
            features: vec![],
            created_at: Utc::now(),
            preferred_agent: AgentKind::default(),
            is_git: true,
        }],
        session_bookmarks: vec![],
        available_harnesses: vec![],
        prompt_templates: Vec::new(),
        extra: HashMap::new(),
    }
}

fn app_for(repo: &Path, worktree: MockWorktreeOps) -> App {
    App::new_for_test(
        store_with_repo(repo.to_path_buf()),
        Box::new(MockTmuxOps::new()),
        Box::new(worktree),
    )
}

fn write_hooks(repo: &Path, hooks: serde_json::Value) {
    std::fs::write(
        repo.join("aiw.json"),
        serde_json::json!({ "lifecycle_hooks": hooks }).to_string(),
    )
    .unwrap();
}

fn on_start(app: &App) -> Option<HookConfig> {
    crate::extension::merge_project_extension_config(
        &app.config.extension,
        &app.store.projects[0].repo,
    )
    .lifecycle_hooks
    .on_start
}

// ── Item 7: repo hook approval ───────────────────────────────────────────

#[test]
fn repo_hook_asks_once_and_is_pinned_to_its_content() {
    let repo = TempDir::new().unwrap();
    write_hooks(
        repo.path(),
        serde_json::json!({ "on_start": "echo first-safety-hook" }),
    );
    let mut app = app_for(repo.path(), MockWorktreeOps::new());
    let path = repo.path().to_path_buf();

    // Unapproved: the dialog opens and nothing proceeds.
    let gate = app.gate_repo_hook(
        &path,
        HookName::Start,
        on_start(&app),
        &path,
        HookResume::Stop,
    );
    assert!(gate.is_none());
    assert!(matches!(app.mode, AppMode::ApprovingRepoHook(_)));

    // Approve: the dialog closes and the previous mode comes back.
    handle_key(&mut app, KeyCode::Char('y')).unwrap();
    assert!(matches!(app.mode, AppMode::Normal));

    let gate = app.gate_repo_hook(
        &path,
        HookName::Start,
        on_start(&app),
        &path,
        HookResume::Stop,
    );
    assert_eq!(
        gate.flatten().map(|cfg| cfg.script().to_string()),
        Some("echo first-safety-hook".to_string())
    );
    assert!(
        app.approved_repo_hook_or_skip(&path, HookName::Start, on_start(&app), &path)
            .is_some()
    );

    // Changed content asks again.
    write_hooks(
        repo.path(),
        serde_json::json!({ "on_start": "echo changed-safety-hook" }),
    );
    assert!(
        app.gate_repo_hook(
            &path,
            HookName::Start,
            on_start(&app),
            &path,
            HookResume::Stop
        )
        .is_none()
    );
    // Cancel leaves it unapproved.
    handle_key(&mut app, KeyCode::Esc).unwrap();
    assert!(matches!(app.mode, AppMode::Normal));
    assert!(
        app.approved_repo_hook_or_skip(&path, HookName::Start, on_start(&app), &path)
            .is_none()
    );
}

#[test]
fn skip_runs_without_the_hook_once_and_asks_again_later() {
    let repo = TempDir::new().unwrap();
    write_hooks(
        repo.path(),
        serde_json::json!({ "on_stop": "echo skip-safety-hook" }),
    );
    let mut app = app_for(repo.path(), MockWorktreeOps::new());
    let path = repo.path().to_path_buf();
    let hook = Some(HookConfig::Script("echo skip-safety-hook".into()));

    assert!(
        app.gate_repo_hook(&path, HookName::Stop, hook.clone(), &path, HookResume::Stop)
            .is_none()
    );
    handle_key(&mut app, KeyCode::Char('s')).unwrap();
    assert!(matches!(app.mode, AppMode::Normal));
    // The replay (stop_feature with nothing selected) didn't consume the
    // skip, and it must not leak into the next run.
    assert!(
        app.gate_repo_hook(&path, HookName::Stop, hook, &path, HookResume::Stop)
            .is_none()
    );
}

#[test]
fn global_hooks_are_trusted() {
    let repo = TempDir::new().unwrap();
    let mut app = app_for(repo.path(), MockWorktreeOps::new());
    let path = repo.path().to_path_buf();
    let hook = Some(HookConfig::Script("echo from-global-config".into()));
    let gate = app.gate_repo_hook(&path, HookName::Start, hook, &path, HookResume::Stop);
    assert!(gate.flatten().is_some());
    assert!(matches!(app.mode, AppMode::Normal));
}

#[test]
fn create_feature_asks_before_creating_the_worktree_then_replays() {
    let repo = TempDir::new().unwrap();
    write_hooks(
        repo.path(),
        serde_json::json!({ "on_worktree_created": "touch safety-hook-ran" }),
    );
    let wt_parent = TempDir::new().unwrap();
    let wt = wt_parent.path().join("wt");
    std::fs::create_dir_all(&wt).unwrap();

    let mut worktree = MockWorktreeOps::new();
    let created = wt.clone();
    worktree
        .expect_create()
        .times(1)
        .returning(move |_, _, _| Ok(created.clone()));
    let mut app = app_for(repo.path(), worktree);
    let mut state =
        CreateFeatureState::new("safety".into(), repo.path().to_path_buf(), vec![], false);
    state.branch = "feat-safety".into();
    state.use_worktree = true;
    state.session_name = "Claude 1".into();
    app.mode = AppMode::CreatingFeature(state);

    app.create_feature().unwrap();
    assert!(
        matches!(app.mode, AppMode::ApprovingRepoHook(_)),
        "no worktree may be created before approval"
    );

    handle_key(&mut app, KeyCode::Char('y')).unwrap();
    assert!(
        matches!(app.mode, AppMode::RunningHook(_)),
        "approved hook should run after the replayed create"
    );
    if let AppMode::RunningHook(state) = &mut app.mode
        && let Some(child) = state.child.as_mut()
    {
        let _ = child.wait();
    }
    assert!(wt.join("safety-hook-ran").exists());
}

// ── Item 8: `~` in project paths ─────────────────────────────────────────

#[test]
fn create_project_expands_tilde_paths() {
    let Some(home) = dirs::home_dir() else {
        return;
    };
    let mut app = App::new_for_test(
        ProjectStore {
            version: 4,
            projects: vec![],
            session_bookmarks: vec![],
            available_harnesses: vec![],
            prompt_templates: Vec::new(),
            extra: HashMap::new(),
        },
        Box::new(MockTmuxOps::new()),
        Box::new(MockWorktreeOps::new()),
    );
    let missing = "aiw-safety-test-does-not-exist-7f3a";
    let request = CreateProjectRequest {
        path: PathBuf::from(format!("~/{missing}")),
        project_name: "tilde".into(),
        preferred_agent: None,
        dry_run: true,
    };
    let err = app.create_project_from_request(&request).unwrap_err();
    assert_eq!(
        err.to_string(),
        format!("Path does not exist: {}", home.join(missing).display())
    );
}

// ── Items 3, 4, 9: worktree location, prep, and branch-in-use ────────────

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn init_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    git(dir.path(), &["config", "user.email", "t@example.com"]);
    git(dir.path(), &["config", "user.name", "t"]);
    std::fs::write(dir.path().join(".gitignore"), ".env*\n").unwrap();
    std::fs::write(dir.path().join("README.md"), "hi\n").unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-qm", "init"]);
    dir
}

#[test]
fn new_worktrees_are_created_outside_the_repo_with_env_copied() {
    let repo = init_repo();
    std::fs::write(repo.path().join(".env"), "SECRET=1\n").unwrap();
    let path = crate::worktree::WorktreeManager::create(repo.path(), "safety-feat", "safety-feat")
        .unwrap();
    assert!(!path.starts_with(repo.path()));
    assert!(path.starts_with(crate::fork::worktree_paths::default_base_dir()));
    assert!(!repo.path().join(".worktrees").exists());
    assert_eq!(
        std::fs::read_to_string(path.join(".env")).unwrap(),
        "SECRET=1\n"
    );

    crate::worktree::WorktreeManager::remove(repo.path(), &path).unwrap();
    assert!(!path.exists());
    // The now-empty project folder is pruned too.
    assert!(!path.parent().unwrap().exists());
}

#[test]
fn branch_already_checked_out_fails_early_and_leaves_no_directories() {
    let repo = init_repo();
    let err =
        crate::worktree::WorktreeManager::create(repo.path(), "safety-main", "main").unwrap_err();
    let canonical = repo.path().canonicalize().unwrap();
    let message = err.to_string();
    assert!(
        message.starts_with("Branch 'main' is already checked out in ")
            && message.ends_with("— choose a new branch name or turn off 'use worktree'"),
        "{message}"
    );
    assert!(
        message.contains(&canonical.display().to_string())
            || message.contains(&repo.path().display().to_string()),
        "{message}"
    );
    let planned = crate::fork::worktree_paths::new_worktree_path(repo.path(), "safety-main");
    assert!(!planned.exists());
    assert!(!planned.parent().unwrap().exists());
    assert!(!repo.path().join(".worktrees").exists());
}

#[test]
fn failed_git_worktree_add_prunes_created_folders() {
    let repo = init_repo();
    // `-b` with a branch name git rejects makes `git worktree add` fail
    // after AIW created the parent folder.
    let err = crate::worktree::WorktreeManager::create_from(
        repo.path(),
        "safety-bad",
        "bad..name",
        "main",
    )
    .unwrap_err();
    assert!(err.to_string().contains("git worktree add failed"), "{err}");
    let planned = crate::fork::worktree_paths::new_worktree_path(repo.path(), "safety-bad");
    assert!(!planned.exists());
    assert!(!planned.parent().unwrap().exists());
}

// ── Item 9: the new-feature dialog never pre-fills the current branch ────

#[test]
fn new_feature_dialog_starts_with_an_empty_branch() {
    let state = CreateFeatureState::new("p".into(), PathBuf::from("/tmp"), vec![], false);
    assert!(state.branch.is_empty());
}

// ── Item 5: port blocks follow the feature's lifecycle ───────────────────

#[test]
fn port_block_is_reserved_for_the_session_and_released_on_delete() {
    let mut store = store_with_repo(PathBuf::from("/tmp/aiw-ports-repo"));
    let mut feature = crate::project::Feature::new_for_project(
        "safety",
        "ports-feat".into(),
        "ports-feat".into(),
        PathBuf::from("/tmp/aiw-ports-repo"),
        false,
        crate::project::VibeMode::default(),
        false,
        false,
        AgentKind::default(),
        false,
        false,
    );
    feature.tmux_session = "aiw-aiw-ports-test-session".into();
    let (id, session) = (feature.id.clone(), feature.tmux_session.clone());
    store.projects[0].features.push(feature);

    crate::fork::ports::prepare_feature_session(&id, &session);
    let env = crate::fork::ports::env_for_tmux_session(&session);
    assert_eq!(env.len(), 2);
    assert_eq!(env[0].0, "AIW_PORT");
    assert!(env[0].1.parse::<u16>().unwrap() >= 4100);
    assert_eq!(env[1], ("AIW_PORT_COUNT".to_string(), "10".to_string()));

    store.remove_feature("safety", "ports-feat").unwrap();
    assert!(crate::fork::ports::env_for_tmux_session(&session).is_empty());
}

// ── Worktree checkout that vanished on disk ─────────────────────────────

/// Starting a worktree feature whose folder is gone must recreate the
/// checkout before anything else looks inside the folder (the screenshot-owner
/// check used to fail with ENOENT first, and a later start then launched the
/// agent into a folder holding only AIW's helper files).
#[test]
fn starting_a_feature_whose_worktree_vanished_recreates_it_first() {
    let repo = TempDir::new().unwrap();
    let parent = TempDir::new().unwrap();
    let gone = parent.path().join("vanished"); // never created
    let mut store = store_with_repo(repo.path().to_path_buf());
    let mut feature = crate::project::Feature::new_for_project(
        "safety",
        "wt-feat".into(),
        "wt-feat".into(),
        gone.clone(),
        true,
        crate::project::VibeMode::Vibe,
        false,
        false,
        AgentKind::Claude,
        false,
        false,
    );
    feature.add_session(crate::project::SessionKind::Claude);
    store.projects[0].features.push(feature);

    let mut worktree = MockWorktreeOps::new();
    worktree.expect_prune().times(1).returning(|_| Ok(()));
    worktree
        .expect_create()
        .times(1)
        .withf(|_, name, branch| name == "safety-wt-feat" && branch == "wt-feat")
        .returning(|_, _, _| Err(anyhow::anyhow!("boom: git worktree add")));
    let mut tmux = MockTmuxOps::new();
    tmux.expect_session_exists().returning(|_| false);
    let mut app = App::new_for_test(store, Box::new(tmux), Box::new(worktree));

    let err = app
        .ensure_feature_running(0, 0, crate::app::resource_gate::StartIntent::Approved)
        .unwrap_err();
    assert!(err.to_string().contains("boom"), "{err}");
}
