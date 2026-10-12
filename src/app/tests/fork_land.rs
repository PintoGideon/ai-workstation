//! FORK: `P` lands a feature, at the App level (DESIGN.md §19.10): the dialog
//! and its preflight, a local merge end to end (the land terminal, the base,
//! the cleanup that removes the feature and its branch), failures on the row,
//! the working-agents confirm, aborting a conflicted rebase, one land per
//! repository, hooks and automation.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::json;

use super::fork_fan_out::{Calls, Shared, lenient_tmux};
use super::support::*;
use crate::app::*;
use crate::fork::badges::{BadgeKind, feature_badge};
use crate::fork::land::{Finish, InProgress, LandConfig};
use crate::project::SessionKind;
use crate::traits::MockWorktreeOps;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "init.defaultBranch=main",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn commit(dir: &Path, file: &str, text: &str) {
    std::fs::write(dir.join(file), text).unwrap();
    git(dir, &["add", file]);
    git(dir, &["commit", "-qm", file]);
}

struct Fixture {
    _dir: tempfile::TempDir,
    app: App,
    calls: Shared,
    repo: PathBuf,
    wt: PathBuf,
    root: PathBuf,
}

/// A real repo on `main` with a feature worktree (`my-feat`, one commit,
/// running `Claude 1`), the feature's tmux session known to the mock.
fn fixture() -> Fixture {
    let dir = tempfile::TempDir::new().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    commit(&repo, "README.md", "hi\n");
    let wt = root.join("feat");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "my-feat",
            &wt.to_string_lossy(),
        ],
    );
    commit(&wt, "feature.txt", "work\n");

    let mut store = store_with_feature(ProjectStatus::Active);
    store.projects[0].repo = repo.clone();
    store.projects[0].is_git = true;
    let feature = &mut store.projects[0].features[0];
    feature.workdir = wt.clone();
    feature.is_worktree = true;
    feature.add_session_named(SessionKind::Claude, "Claude 1".into());
    let calls: Shared = Default::default();
    calls.lock().unwrap().sessions.insert("aiw-my-feat".into());
    let mut app = App::new_for_test(
        store,
        Box::new(lenient_tmux(calls.clone())),
        Box::new(MockWorktreeOps::new()),
    );
    app.db = Some(crate::db::AiwDb::open(&root.join("aiw.db")).unwrap());
    app.land.state_dir = Some(root.join("state"));
    app.selection = Selection::Feature(0, 0);
    Fixture {
        _dir: dir,
        app,
        calls,
        repo,
        wt,
        root,
    }
}

fn press(app: &mut App, code: KeyCode) {
    crate::handlers::handle_key(app, KeyEvent::new(code, KeyModifiers::NONE)).unwrap();
}

/// The repository's `aiw.json` `land` block (what users write).
fn set_land(app: &mut App, config: LandConfig) {
    let repo = app.store.projects[0].repo.clone();
    std::fs::write(
        repo.join("aiw.json"),
        serde_json::to_string(&json!({ "land": config })).unwrap(),
    )
    .unwrap();
}

/// Tick until `done` holds, or panic after a few seconds.
fn tick_until(app: &mut App, what: &str, done: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < deadline {
        app.land_tick();
        if done(app) {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("timed out waiting for {what}");
}

fn dialog(app: &App) -> &crate::app::fork_land::LandDialogState {
    match &app.mode {
        AppMode::Land(state) => state,
        _ => panic!("the land dialog is not open"),
    }
}

fn open_with_preflight(app: &mut App) {
    press(app, KeyCode::Char('P'));
    tick_until(
        app,
        "the preflight",
        |a| matches!(&a.mode, AppMode::Land(s) if s.preflight.is_some()),
    );
}

fn dialog_text(app: &App) -> String {
    crate::app::fork_land::land_lines(app, dialog(app), 20)
        .into_iter()
        .map(|(t, _)| t)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn p_opens_the_dialog_and_the_preflight_fills_it() {
    let mut fx = fixture();
    commit(&fx.repo, "base.txt", "moved\n");
    set_land(
        &mut fx.app,
        LandConfig {
            check_command: Some("true".into()),
            ..Default::default()
        },
    );
    press(&mut fx.app, KeyCode::Char('P'));
    assert!(dialog(&fx.app).preflight.is_none());
    assert!(dialog_text(&fx.app).contains("Checking"));
    tick_until(
        &mut fx.app,
        "the preflight",
        |a| matches!(&a.mode, AppMode::Land(s) if s.preflight.is_some()),
    );
    let pre = dialog(&fx.app).preflight.clone().unwrap();
    assert_eq!(pre.base, "main");
    assert_eq!((pre.ahead, pre.behind), (1, 1));
    assert!(pre.landable(), "{:?}", pre.blockers);
    let text = dialog_text(&fx.app);
    assert!(text.contains("my-feat → main"), "{text}");
    assert!(text.contains("git rebase main"), "{text}");
    assert!(text.contains("check: true"), "{text}");
    press(&mut fx.app, KeyCode::Esc);
    assert!(matches!(fx.app.mode, AppMode::Normal));
}

#[test]
fn a_project_row_asks_for_a_feature() {
    let mut fx = fixture();
    fx.app.selection = Selection::Project(0);
    press(&mut fx.app, KeyCode::Char('P'));
    assert!(matches!(fx.app.mode, AppMode::Normal));
    assert_eq!(fx.app.message.as_deref(), Some("Select a feature to land"));
}

#[test]
fn a_local_merge_lands_then_removes_the_feature_and_its_branch() {
    let mut fx = fixture();
    commit(&fx.repo, "base.txt", "moved\n");
    set_land(
        &mut fx.app,
        LandConfig {
            check_command: Some("test -f feature.txt && test -f base.txt".into()),
            ..Default::default()
        },
    );
    open_with_preflight(&mut fx.app);
    press(&mut fx.app, KeyCode::Enter);
    // The land terminal opened in the feature and tails the log.
    {
        let calls = fx.calls.lock().unwrap();
        let cmd = calls
            .shell
            .iter()
            .map(|(_, _, cmd)| cmd.clone())
            .find(|cmd| cmd.contains(".tail.sh"))
            .unwrap_or_else(|| panic!("{:?}", calls.shell));
        let script = cmd.trim_start_matches("clear; sh ").trim_matches('\'');
        let body = std::fs::read_to_string(script).unwrap();
        assert!(body.contains("tail -n +1 -F"), "{body}");
    }
    assert!(
        fx.app.store.projects[0].features[0]
            .sessions
            .iter()
            .any(|s| s.label == "land" && s.kind == SessionKind::Terminal)
    );
    tick_until(&mut fx.app, "the land to end", |a| {
        a.land_stage("feat-1").is_none() && a.land_last("feat-1").is_some()
    });
    let last = fx.app.land_last("feat-1").unwrap().outcome.clone();
    assert!(last.ok, "{:?}", last.failure);
    let head = git(&fx.wt, &["rev-parse", "HEAD"]);
    assert_eq!(git(&fx.repo, &["rev-parse", "main"]), head);
    assert!(dialog_text(&fx.app).contains("✓ landed"));
    // The cleanup waits for the dialog to close …
    fx.app.land_tick();
    assert!(fx.app.feature_indices_by_id("feat-1").is_some());
    press(&mut fx.app, KeyCode::Esc);
    fx.app.land_tick();
    // … then removes the feature through the normal delete flow.
    let deadline = Instant::now() + Duration::from_secs(20);
    while fx.app.feature_indices_by_id("feat-1").is_some() && Instant::now() < deadline {
        fx.app.poll_deleting_feature_and_finish().unwrap();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        fx.app.feature_indices_by_id("feat-1").is_none(),
        "{:?}",
        fx.app.message
    );
    assert!(!fx.wt.exists());
    fx.app.land_tick();
    let branches = git(&fx.repo, &["branch", "--list", "my-feat"]);
    assert_eq!(branches, "", "the landed branch is deleted");
    // Automation still answers for the removed feature by name.
    let status = fx
        .app
        .automation_land(&json!({"op": "status", "feature": "my-feat"}));
    assert_eq!(status["state"], "succeeded", "{status}");
}

#[test]
fn a_failing_check_marks_the_row_until_dismissed() {
    let mut fx = fixture();
    let main_before = git(&fx.repo, &["rev-parse", "main"]);
    set_land(
        &mut fx.app,
        LandConfig {
            check_command: Some("echo boom; exit 2".into()),
            ..Default::default()
        },
    );
    open_with_preflight(&mut fx.app);
    press(&mut fx.app, KeyCode::Enter);
    tick_until(&mut fx.app, "the land to end", |a| {
        a.land_failure("feat-1").is_some()
    });
    assert_eq!(git(&fx.repo, &["rev-parse", "main"]), main_before);
    let badge = feature_badge(&fx.app, 0, 0, chrono::Utc::now());
    assert_eq!(badge.kind, BadgeKind::NeedsYou);
    assert_eq!(
        badge.detail.as_deref(),
        Some("land failed: check failed (exit 2)")
    );
    assert_eq!(
        fx.app.land_line("feat-1").as_deref(),
        Some("⛔ land failed: check failed (exit 2) (P)")
    );
    let text = dialog_text(&fx.app);
    assert!(
        text.contains("last land failed: check failed (exit 2)"),
        "{text}"
    );
    assert!(text.contains("boom"), "{text}");
    // The row's height accounts for the land line.
    let item = crate::app::VisibleItem::Feature(0, 0);
    let with_line = fx.app.workspace_item_height(&item);
    press(&mut fx.app, KeyCode::Char('x'));
    assert!(fx.app.land_failure("feat-1").is_none());
    assert_eq!(fx.app.workspace_item_height(&item), with_line - 1);
    // A failed land does no cleanup.
    press(&mut fx.app, KeyCode::Esc);
    fx.app.land_tick();
    assert!(fx.app.feature_indices_by_id("feat-1").is_some());
}

#[test]
fn landing_while_an_agent_works_needs_a_second_confirm() {
    let mut fx = fixture();
    set_land(
        &mut fx.app,
        LandConfig {
            finish: Some(Finish::None),
            ..Default::default()
        },
    );
    let claude = fx.app.store.projects[0].features[0].sessions[0].id.clone();
    fx.app.ipc_thinking_feature_sessions.insert(claude);
    open_with_preflight(&mut fx.app);
    assert_eq!(dialog(&fx.app).working, vec!["Claude 1".to_string()]);
    press(&mut fx.app, KeyCode::Enter);
    assert!(dialog(&fx.app).confirm_working);
    assert!(
        fx.app.land_stage("feat-1").is_none(),
        "Enter alone does not land"
    );
    press(&mut fx.app, KeyCode::Char('y'));
    tick_until(&mut fx.app, "the land to end", |a| {
        a.land_last("feat-1").is_some()
    });
    assert!(fx.app.land_last("feat-1").unwrap().outcome.ok);
    // finish none keeps the feature and marks it ready.
    press(&mut fx.app, KeyCode::Esc);
    fx.app.land_tick();
    assert!(fx.app.store.projects[0].features[0].ready);
}

#[test]
fn a_conflicted_rebase_stays_in_progress_and_a_aborts_it() {
    let mut fx = fixture();
    commit(&fx.repo, "feature.txt", "base version\n");
    open_with_preflight(&mut fx.app);
    press(&mut fx.app, KeyCode::Enter);
    tick_until(&mut fx.app, "the land to end", |a| {
        a.land_failure("feat-1").is_some()
    });
    let failure = fx.app.land_failure("feat-1").unwrap().clone();
    assert_eq!(failure.in_progress, Some(InProgress::Rebase));
    assert_eq!(
        crate::fork::land::in_progress(&fx.wt),
        Some(InProgress::Rebase)
    );
    assert!(dialog_text(&fx.app).contains("a aborts the rebase"));
    press(&mut fx.app, KeyCode::Char('a'));
    assert_eq!(crate::fork::land::in_progress(&fx.wt), None);
    assert_eq!(dialog(&fx.app).note.as_deref(), Some("aborted the rebase"));
    tick_until(
        &mut fx.app,
        "the fresh preflight",
        |a| matches!(&a.mode, AppMode::Land(s) if s.preflight.is_some()),
    );
}

#[test]
fn a_dirty_worktree_blocks_the_land() {
    let mut fx = fixture();
    std::fs::write(fx.wt.join("feature.txt"), "edited\n").unwrap();
    open_with_preflight(&mut fx.app);
    let pre = dialog(&fx.app).preflight.clone().unwrap();
    assert!(!pre.landable());
    press(&mut fx.app, KeyCode::Enter);
    assert!(fx.app.land_stage("feat-1").is_none());
    assert_eq!(
        dialog(&fx.app).note.as_deref(),
        Some("blocked — see the list above")
    );
    assert!(dialog_text(&fx.app).contains("uncommitted work (1 changed, 0 untracked)"));
}

#[test]
fn one_land_per_repository() {
    let mut fx = fixture();
    // A second feature in the same repo.
    let wt2 = fx.root.join("feat2");
    git(
        &fx.repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "other",
            &wt2.to_string_lossy(),
        ],
    );
    commit(&wt2, "other.txt", "x\n");
    let mut second = fx.app.store.projects[0].features[0].clone();
    second.id = "feat-2".into();
    second.name = "other".into();
    second.branch = "other".into();
    second.workdir = wt2.clone();
    second.tmux_session = "aiw-other".into();
    second.sessions.clear();
    fx.app.store.projects[0].features.push(second);
    set_land(
        &mut fx.app,
        LandConfig {
            check_command: Some("sleep 2".into()),
            finish: Some(Finish::None),
            ..Default::default()
        },
    );
    open_with_preflight(&mut fx.app);
    press(&mut fx.app, KeyCode::Enter);
    assert!(fx.app.land_stage("feat-1").is_some());
    fx.app.mode = AppMode::Normal;
    fx.app.selection = Selection::Feature(0, 1);
    open_with_preflight(&mut fx.app);
    let pre = dialog(&fx.app).preflight.clone().unwrap();
    assert!(
        pre.blockers
            .iter()
            .any(|b| b.contains("another land is running in this repository (my-feat)")),
        "{:?}",
        pre.blockers
    );
    tick_until(&mut fx.app, "the first land", |a| {
        a.land_last("feat-1").is_some()
    });
}

#[test]
fn automation_dry_run_start_and_status() {
    let mut fx = fixture();
    set_land(
        &mut fx.app,
        LandConfig {
            remove_worktree: Some(false),
            delete_branch: Some(false),
            ..Default::default()
        },
    );
    let main_before = git(&fx.repo, &["rev-parse", "main"]);
    let dry = fx
        .app
        .automation_land(&json!({"op": "dry_run", "feature": "my-feat"}));
    assert_eq!(dry["ok"], true);
    assert_eq!(dry["landable"], true);
    assert!(
        dry["preflight"]["plan"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c.as_str().unwrap().contains("merge --ff-only my-feat"))
    );
    assert_eq!(
        git(&fx.repo, &["rev-parse", "main"]),
        main_before,
        "dry run changes nothing"
    );
    assert_eq!(
        fx.app
            .automation_land(&json!({"op": "status", "feature": "my-feat"}))["state"],
        "idle"
    );
    let unknown = fx.app.automation_land(&json!({"feature": "nope"}));
    assert_eq!(unknown["ok"], false);

    let started = fx
        .app
        .automation_land(&json!({"op": "start", "feature": "my-feat"}));
    assert_eq!(started["ok"], true, "{started}");
    assert_eq!(started["status"], "started");
    tick_until(&mut fx.app, "the land", |a| a.land_last("feat-1").is_some());
    let status = fx
        .app
        .automation_land(&json!({"op": "status", "feature": "my-feat"}));
    assert_eq!(status["state"], "succeeded", "{status}");
    assert_eq!(
        git(&fx.repo, &["rev-parse", "main"]),
        git(&fx.wt, &["rev-parse", "HEAD"])
    );
    // Kept: remove_worktree false; marked ready once no dialog is open.
    fx.app.land_tick();
    assert!(fx.app.store.projects[0].features[0].ready);
    assert!(fx.wt.exists());
}

#[test]
fn automation_refuses_working_agents_without_yes() {
    let mut fx = fixture();
    let claude = fx.app.store.projects[0].features[0].sessions[0].id.clone();
    fx.app.ipc_thinking_feature_sessions.insert(claude);
    let refused = fx
        .app
        .automation_land(&json!({"op": "start", "feature": "my-feat"}));
    assert_eq!(refused["ok"], false);
    assert!(
        refused["error"].as_str().unwrap().contains("--yes"),
        "{refused}"
    );
    set_land(
        &mut fx.app,
        LandConfig {
            finish: Some(Finish::None),
            ..Default::default()
        },
    );
    let started = fx
        .app
        .automation_land(&json!({"op": "start", "feature": "my-feat", "yes": true}));
    assert_eq!(started["ok"], true, "{started}");
    tick_until(&mut fx.app, "the land", |a| a.land_last("feat-1").is_some());
}

#[test]
fn an_unapproved_repository_hook_is_gated_in_the_dialog_and_refused_in_automation() {
    let mut fx = fixture();
    let marker = fx.root.join("pre.txt");
    std::fs::write(
        fx.repo.join("aiw.json"),
        serde_json::to_string(&json!({
            "lifecycle_hooks": {"pre_land": format!("echo ran > {}", marker.display())},
            "land": {"finish": "none"}
        }))
        .unwrap(),
    )
    .unwrap();
    let refused = fx
        .app
        .automation_land(&json!({"op": "start", "feature": "my-feat"}));
    assert_eq!(refused["ok"], false);
    assert!(
        refused["error"]
            .as_str()
            .unwrap()
            .contains("pre_land hook is not approved"),
        "{refused}"
    );
    open_with_preflight(&mut fx.app);
    assert!(dialog_text(&fx.app).contains("pre_land hook: echo ran"));
    press(&mut fx.app, KeyCode::Enter);
    assert!(matches!(fx.app.mode, AppMode::ApprovingRepoHook(_)));
    press(&mut fx.app, KeyCode::Char('y'));
    // Approved: the dialog is back and the land runs with the hook.
    assert!(matches!(fx.app.mode, AppMode::Land(_)));
    tick_until(&mut fx.app, "the land", |a| a.land_last("feat-1").is_some());
    assert!(fx.app.land_last("feat-1").unwrap().outcome.ok);
    assert_eq!(std::fs::read_to_string(&marker).unwrap().trim(), "ran");
}

#[test]
fn the_land_line_reads_landing_while_it_runs() {
    let mut fx = fixture();
    set_land(
        &mut fx.app,
        LandConfig {
            check_command: Some("sleep 1".into()),
            finish: Some(Finish::None),
            ..Default::default()
        },
    );
    open_with_preflight(&mut fx.app);
    press(&mut fx.app, KeyCode::Enter);
    tick_until(&mut fx.app, "the check stage", |a| {
        a.land_line("feat-1").as_deref() == Some("⇡ landing · checking")
    });
    let badge = feature_badge(&fx.app, 0, 0, chrono::Utc::now());
    assert_eq!(badge.kind, BadgeKind::Working);
    tick_until(&mut fx.app, "the land", |a| a.land_last("feat-1").is_some());
    assert_eq!(fx.app.land_line("feat-1"), None);
    let _ = Calls::default();
}

#[test]
fn a_failed_land_is_a_needs_you_row_that_opens_the_dialog_and_x_dismisses() {
    let mut fx = fixture();
    set_land(
        &mut fx.app,
        LandConfig {
            check_command: Some("exit 3".into()),
            ..Default::default()
        },
    );
    open_with_preflight(&mut fx.app);
    press(&mut fx.app, KeyCode::Enter);
    tick_until(&mut fx.app, "the land to fail", |a| {
        a.land_failure("feat-1").is_some()
    });
    press(&mut fx.app, KeyCode::Esc);
    fx.app.mode = AppMode::Normal;

    let rows = fx.app.attention_rows();
    let index = rows
        .iter()
        .position(|r| {
            matches!(
                r,
                crate::app::attention::AttentionRow::LandFailed { feature_id, .. }
                    if feature_id == "feat-1"
            )
        })
        .expect("a failed land is listed in i");
    assert_eq!(
        rows.iter()
            .filter(|r| matches!(r, crate::app::attention::AttentionRow::LandFailed { .. }))
            .count(),
        1
    );

    // The row names the land's own failure, even when something else on the
    // feature is just as urgent (the feature badge would show that instead).
    let tmux_session = fx.app.store.projects[0].features[0].tmux_session.clone();
    fx.app.store.projects[0].features[0].status = crate::project::ProjectStatus::Active;
    fx.app.attention.insert(
        tmux_session,
        crate::app::attention::AttentionRecord::new(
            crate::project::AgentKind::Claude,
            crate::app::attention::AttentionState::Question,
            chrono::Utc::now(),
        ),
    );
    let rows = fx.app.attention_rows();
    let index = rows
        .iter()
        .position(|r| matches!(r, crate::app::attention::AttentionRow::LandFailed { .. }))
        .unwrap();
    let badges = crate::ui::picker::row_badges(&fx.app, &rows);
    let label = badges[index].as_ref().unwrap().detail.clone();
    assert_eq!(label.as_deref(), Some("land failed: check failed (exit 3)"));

    // Enter opens that feature's land dialog.
    fx.app.mode = AppMode::NotificationPicker(index, None);
    fx.app.handle_notification_select().unwrap();
    assert_eq!(dialog(&fx.app).feature_id, "feat-1");
    press(&mut fx.app, KeyCode::Esc);

    // x in the list forgets the failure; the row goes.
    fx.app.mode = AppMode::NotificationPicker(index, None);
    crate::handlers::handle_key(&mut fx.app, KeyEvent::from(KeyCode::Char('x'))).unwrap();
    assert!(fx.app.land_failure("feat-1").is_none());
    assert!(
        !fx.app
            .attention_rows()
            .iter()
            .any(|r| matches!(r, crate::app::attention::AttentionRow::LandFailed { .. }))
    );
}

