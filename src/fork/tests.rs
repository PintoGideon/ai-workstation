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
    let repo = init_repo();
    let parent = TempDir::new().unwrap();
    let gone = parent.path().join("vanished");
    git(
        repo.path(),
        &[
            "worktree",
            "add",
            "-q",
            gone.to_str().unwrap(),
            "-b",
            "wt-feat",
        ],
    );
    std::fs::remove_dir_all(&gone).unwrap(); // registered with git, folder gone
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

// ── a broken aiw.json is announced once per project load ─────────────────────

#[test]
fn a_broken_aiw_json_is_announced_once_per_project_load_and_again_when_it_changes() {
    use crate::app::Selection;

    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(
        repo.join("aiw.json"),
        "{ \"lifecycle_hooks\": { \"on_start\": ",
    )
    .unwrap();
    let mut app = App::new_for_test(
        store_with_repo(repo.clone()),
        Box::new(MockTmuxOps::new()),
        Box::new(MockWorktreeOps::new()),
    );
    app.selection = Selection::Project(0);

    app.reload_extension_config();
    assert_eq!(app.toasts.len(), 1);
    let first = app.toasts[0].message.clone();
    assert!(
        first.starts_with("aiw.json in safety is invalid: "),
        "{first}"
    );
    assert!(first.contains("line 1"), "{first}");
    assert!(first.ends_with("hooks and custom sessions are disabled until fixed"));
    assert!(app.active_extension.lifecycle_hooks.on_start.is_none());

    // Reloading on every selection change must not nag.
    app.reload_extension_config();
    app.reload_extension_config();
    assert_eq!(app.toasts.len(), 1);

    // A different error is news again.
    std::fs::write(repo.join("aiw.json"), "{\n\n  [").unwrap();
    app.reload_extension_config();
    assert_eq!(app.toasts.len(), 2);
    assert!(
        app.toasts[1].message.contains("line 3"),
        "{}",
        app.toasts[1].message
    );

    // Fixed: forgotten, so breaking it later is reported afresh.
    std::fs::write(
        repo.join("aiw.json"),
        r#"{"lifecycle_hooks":{"on_start":"true"}}"#,
    )
    .unwrap();
    app.reload_extension_config();
    assert!(app.reported_config_errors.is_empty());
    assert!(app.active_extension.lifecycle_hooks.on_start.is_some());
    assert_eq!(app.toasts.len(), 2);
}

#[test]
fn a_broken_aiw_json_read_by_any_action_is_announced_on_the_next_tick() {
    let tmp = TempDir::new().unwrap();
    let repo = tmp.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    std::fs::write(repo.join("aiw.json"), "{ \"custom_sessions\": [").unwrap();
    let mut app = App::new_for_test(
        store_with_repo(repo.clone()),
        Box::new(MockTmuxOps::new()),
        Box::new(MockWorktreeOps::new()),
    );

    // Reads through `&self` (automation, plan interview, triage…) queue it.
    let _ = app.extension_for_repo(&repo);
    let _ = app.merge_extension(&app.config.extension.clone(), &repo);
    assert!(app.toasts.is_empty());
    assert!(app.report_config_errors());
    assert_eq!(app.toasts.len(), 1);
    assert!(app.toasts[0].message.contains("is invalid"));

    // Same error again: no second toast.
    let _ = app.extension_for_repo(&repo);
    assert!(!app.report_config_errors());
    assert_eq!(app.toasts.len(), 1);
}

// ── malformed notification files are quarantined ─────────────────────────────

#[test]
fn a_malformed_notification_file_is_moved_to_the_quarantine_by_the_scan() {
    let notify_dir = crate::project::aiw_config_dir().join("notifications");
    std::fs::create_dir_all(&notify_dir).unwrap();
    let name = format!("garbage-{}.json", std::process::id());
    let junk = notify_dir.join(&name);
    std::fs::write(&junk, "{\"session_id\": ").unwrap();
    let file = std::fs::File::options().write(true).open(&junk).unwrap();
    file.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(30))
        .unwrap();

    let app = App::new_for_test(
        ProjectStore::empty(),
        Box::new(MockTmuxOps::new()),
        Box::new(MockWorktreeOps::new()),
    );
    let inputs = app.read_notification_files();

    assert!(!inputs.iter().any(|i| i.file_path == junk));
    assert!(
        !junk.exists(),
        "the malformed file is gone from the scan dir"
    );
    let quarantined = crate::fork::bad_notifications::bad_dir().join(&name);
    assert!(quarantined.exists(), "{}", quarantined.display());
    let _ = std::fs::remove_file(quarantined);
}

// ── hook failures are visible ────────────────────────────────────────────────

fn empty_app() -> App {
    App::new_for_test(
        ProjectStore::empty(),
        Box::new(MockTmuxOps::new()),
        Box::new(MockWorktreeOps::new()),
    )
}

#[test]
fn a_failing_lifecycle_hook_raises_a_toast_naming_the_hook_and_feature() {
    let tmp = TempDir::new().unwrap();
    let mut app = empty_app();
    let tag = format!("hooktoast-{}", std::process::id());

    app.run_lifecycle_hook("on_start", &tag, "exit 7", tmp.path(), None);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let expected = format!("Hook on_start failed (exit 7) in {tag}");
    while !app.toasts.iter().any(|t| t.message == expected) {
        assert!(
            std::time::Instant::now() < deadline,
            "no toast: {:?}",
            app.toasts.iter().map(|t| &t.message).collect::<Vec<_>>()
        );
        app.drain_fork_notices();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    // A hook that succeeds says nothing.
    let quiet = format!("quiet-{}", std::process::id());
    app.run_lifecycle_hook("on_stop", &quiet, "exit 0", tmp.path(), None);
    std::thread::sleep(std::time::Duration::from_millis(300));
    app.drain_fork_notices();
    assert!(!app.toasts.iter().any(|t| t.message.contains(&quiet)));
}

fn running_hook_state(child: Option<std::process::Child>) -> crate::app::RunningHookState {
    crate::app::RunningHookState {
        script: "sleep 600".to_string(),
        workdir: std::env::temp_dir(),
        project_name: "safety".to_string(),
        feature_name: None,
        todo_origin: None,
        issue_source: None,
        branch: "slow".to_string(),
        mode: crate::project::VibeMode::default(),
        review: false,
        plan_mode: false,
        quick_plan: false,
        agent: AgentKind::Claude,
        create_terminal: false,
        session_name: "Claude 1".to_string(),
        enable_chrome: false,
        startup_prompt: None,
        child,
        output: String::new(),
        success: None,
        output_rx: None,
        started_at: std::time::Instant::now(),
        timeout_noted: false,
        skipped: false,
    }
}

#[test]
fn a_hanging_worktree_hook_offers_a_way_out_after_the_timeout() {
    use crate::app::RunningHookState;

    let child = Command::new("sleep")
        .arg("600")
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let pid = child.id();
    let mut app = empty_app();
    app.mode = AppMode::RunningHook(running_hook_state(Some(child)));

    // Before the timeout: nothing to skip, Esc is ignored while running.
    app.poll_running_hook().unwrap();
    assert!(!app.skip_running_hook());
    crate::handlers::handle_running_hook_key(&mut app, KeyCode::Esc).unwrap();
    assert!(matches!(app.mode, AppMode::RunningHook(_)));

    // Past the timeout the dialog text says what to do.
    if let AppMode::RunningHook(state) = &mut app.mode {
        state.started_at = std::time::Instant::now() - RunningHookState::TIMEOUT;
    }
    app.poll_running_hook().unwrap();
    let AppMode::RunningHook(state) = &app.mode else {
        panic!("still running")
    };
    assert!(state.timeout_noted);
    assert!(
        state.output.contains("still running after 60 s"),
        "{}",
        state.output
    );
    assert!(state.output.contains("Esc to skip"), "{}", state.output);
    assert!(state.child.is_some());

    // Skipping kills the hook and records it.
    assert!(app.skip_running_hook());
    let AppMode::RunningHook(state) = &app.mode else {
        panic!("still in the dialog until completion")
    };
    assert!(state.skipped);
    assert!(state.child.is_none());
    assert!(state.output.contains("hook skipped"));
    let alive = Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    assert!(!alive, "the hook process was killed");
}

#[test]
fn skipping_a_hanging_hook_also_kills_what_it_started() {
    use crate::app::RunningHookState;
    use std::os::unix::process::CommandExt;

    // Like a hook running `npm install`: `sh` with a long-running child.
    let child = Command::new("sh")
        .args(["-c", "sleep 600 & echo $!; wait"])
        .stdout(std::process::Stdio::piped())
        .process_group(0)
        .spawn()
        .unwrap();
    let mut child = child;
    let mut line = String::new();
    {
        use std::io::BufRead;
        let stdout = child.stdout.take().unwrap();
        std::io::BufReader::new(stdout)
            .read_line(&mut line)
            .unwrap();
    }
    let grandchild: i64 = line.trim().parse().unwrap();
    assert!(crate::resources::procs::pid_alive(grandchild));

    let mut app = empty_app();
    let mut state = running_hook_state(Some(child));
    state.started_at = std::time::Instant::now() - RunningHookState::TIMEOUT;
    app.mode = AppMode::RunningHook(state);
    assert!(app.skip_running_hook());

    // The grandchild is reparented and reaped by init, which can take a moment.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while crate::resources::procs::pid_alive(grandchild) && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        !crate::resources::procs::pid_alive(grandchild),
        "the hook's child was killed too"
    );
}
