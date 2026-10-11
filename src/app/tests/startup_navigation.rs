use super::support::*;
use crate::app::util::{shorten_path, slugify, slugify_shortened};
use crate::app::*;
use crate::project::{AgentKind, Feature, Project};
use crate::traits::{MockTmuxOps, MockWorktreeOps};
use chrono::{Duration, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::collections::HashMap;

#[test]
fn startup_harness_setup_allows_quit_with_q() {
    let store = ProjectStore {
        version: 5,
        projects: vec![],
        session_bookmarks: vec![],
        available_harnesses: vec![],
        prompt_templates: Vec::new(),
        extra: HashMap::new(),
    };
    let mut app = App::new_for_test(
        store,
        Box::new(MockTmuxOps::new()),
        Box::new(MockWorktreeOps::new()),
    );

    app.open_harness_setup(true);
    crate::handlers::handle_harness_setup_key(&mut app, KeyCode::Char('q')).unwrap();

    assert!(app.should_quit);
}

// FORK: the harness screen probes every CLI on open.

fn harness_app(available: Vec<AgentKind>) -> App {
    let store = ProjectStore {
        version: 5,
        projects: vec![],
        session_bookmarks: vec![],
        available_harnesses: available,
        prompt_templates: Vec::new(),
        extra: HashMap::new(),
    };
    let mut app = App::new_for_test(
        store,
        Box::new(MockTmuxOps::new()),
        Box::new(MockWorktreeOps::new()),
    );
    // Claude and Codex are "on PATH"; the other three are not.
    app.harness_probe = |kind| match kind {
        AgentKind::Claude | AgentKind::Codex => Ok(()),
        other => anyhow::bail!("{} missing", other.display_name()),
    };
    app
}

fn harness_rows(app: &App) -> Vec<(AgentKind, HarnessCheckStatus, bool)> {
    match &app.mode {
        AppMode::HarnessSetup(state) => state
            .harnesses
            .iter()
            .map(|h| (h.kind.clone(), h.status.clone(), h.enabled))
            .collect(),
        _ => panic!("expected the harness screen"),
    }
}

fn settle_harness_checks(app: &mut App) {
    let deadline = Instant::now() + std::time::Duration::from_secs(5);
    loop {
        app.poll_harness_checks();
        let pending = harness_rows(app)
            .iter()
            .any(|(_, status, _)| *status == HarnessCheckStatus::Checking);
        if !pending {
            return;
        }
        assert!(Instant::now() < deadline, "harness checks never finished");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn select_harness(app: &mut App, kind: &AgentKind) {
    if let AppMode::HarnessSetup(state) = &mut app.mode {
        state.selected = AgentKind::index_in(&AgentKind::ALL, kind);
    }
}

fn harness_row(app: &App, kind: &AgentKind) -> (HarnessCheckStatus, bool) {
    harness_rows(app)
        .into_iter()
        .find(|(k, _, _)| k == kind)
        .map(|(_, status, enabled)| (status, enabled))
        .unwrap()
}

#[test]
fn first_run_probes_every_harness_and_ticks_the_installed_ones() {
    let mut app = harness_app(vec![]);
    app.open_harness_setup(true);
    assert!(
        harness_rows(&app)
            .iter()
            .all(|(_, status, _)| *status == HarnessCheckStatus::Checking),
        "every row is probed as soon as the screen opens"
    );
    settle_harness_checks(&mut app);
    for (kind, status, enabled) in harness_rows(&app) {
        match kind {
            AgentKind::Claude | AgentKind::Codex => {
                assert_eq!(status, HarnessCheckStatus::Installed, "{kind:?}");
                assert!(enabled, "{kind:?} is ticked without a keypress");
            }
            _ => {
                assert!(
                    matches!(status, HarnessCheckStatus::NotFound(_)),
                    "{kind:?}"
                );
                assert!(!enabled, "{kind:?}");
            }
        }
    }
}

#[test]
fn the_manage_screen_reports_what_is_installed_but_keeps_the_saved_choice() {
    let mut app = harness_app(vec![AgentKind::Claude]);
    app.open_harness_setup(false);
    settle_harness_checks(&mut app);
    let (status, enabled) = harness_row(&app, &AgentKind::Codex);
    assert_eq!(status, HarnessCheckStatus::Installed);
    assert!(
        !enabled,
        "Codex is shown as installed but left for the user to tick"
    );
    assert!(
        harness_row(&app, &AgentKind::Claude).1,
        "the saved choice stays ticked"
    );

    // Enter on the installed Codex row ticks it at once, and c saves both.
    select_harness(&mut app, &AgentKind::Codex);
    crate::handlers::handle_harness_setup_key(&mut app, KeyCode::Enter).unwrap();
    assert!(harness_row(&app, &AgentKind::Codex).1);
    crate::handlers::handle_harness_setup_key(&mut app, KeyCode::Char('c')).unwrap();
    assert_eq!(
        app.store.available_harnesses,
        vec![AgentKind::Claude, AgentKind::Codex]
    );
}

#[test]
fn enter_during_the_opening_sweep_still_ticks_the_row() {
    let mut app = harness_app(vec![]);
    app.open_harness_setup(false);
    // Codex is still "checking" when the user presses Enter on it.
    select_harness(&mut app, &AgentKind::Codex);
    crate::handlers::handle_harness_setup_key(&mut app, KeyCode::Enter).unwrap();
    settle_harness_checks(&mut app);
    let (status, enabled) = harness_row(&app, &AgentKind::Codex);
    assert_eq!(status, HarnessCheckStatus::Installed);
    assert!(enabled, "the Enter was not lost to the sweep");
}

// ── slugify ───────────────────────────────────────────────

#[test]
fn slugify_spaces_become_hyphens() {
    assert_eq!(slugify("hello world"), "hello-world");
}

#[test]
fn slugify_empty_input() {
    assert_eq!(slugify(""), "");
}

#[test]
fn slugify_all_specials() {
    assert_eq!(slugify("!@#$%"), "");
}

#[test]
fn slugify_shortened_cuts_on_a_word_boundary() {
    assert_eq!(
        slugify_shortened("add a way to shorten the feature branch title", 20),
        "add-a-way-to-shorten"
    );
}

#[test]
fn slugify_shortened_leaves_a_slug_that_already_fits() {
    assert_eq!(slugify_shortened("hello world", 20), "hello-world");
    assert_eq!(slugify_shortened("hello world", 11), "hello-world");
}

/// No word boundary to cut on: a hard cut beats a name that is still too long.
#[test]
fn slugify_shortened_hard_cuts_a_single_oversized_word() {
    assert_eq!(slugify_shortened("supercalifragilistic", 5), "super");
}

/// A cut must never leave a trailing or doubled separator behind.
#[test]
fn slugify_shortened_never_leaves_a_dangling_separator() {
    for max in 1..40 {
        let slug = slugify_shortened("alpha beta gamma delta epsilon zeta", max);
        assert!(slug.chars().count() <= max, "max={max} got {slug}");
        assert!(
            !slug.starts_with('-') && !slug.ends_with('-'),
            "max={max} got {slug}"
        );
        assert!(!slug.contains("--"), "max={max} got {slug}");
    }
}

/// Nothing sluggable in, nothing out — the callers that need a fallback name
/// check for the empty string.
#[test]
fn slugify_shortened_of_an_unsluggable_title_is_empty() {
    assert_eq!(slugify_shortened("!!! ???", 20), "");
}

#[test]
fn slugify_preserves_hyphens() {
    assert_eq!(slugify("my-feature"), "my-feature");
}

// ── shorten_path ──────────────────────────────────────────

#[test]
fn shorten_path_inside_home() {
    if let Some(home) = dirs::home_dir() {
        let path = home.join("projects").join("my-app");
        let result = shorten_path(&path);
        assert_eq!(result, "~/projects/my-app");
    }
}

#[test]
fn shorten_path_outside_home() {
    let path = std::path::Path::new("/tmp/some/path");
    let result = shorten_path(path);
    assert_eq!(result, "/tmp/some/path");
}

#[test]
fn visible_items_prioritizes_non_worktree_features() {
    let now = Utc::now();
    let project = Project {
        id: "proj-1".to_string(),
        name: "my-project".to_string(),
        repo: PathBuf::from("/tmp/test-repo"),
        collapsed: false,
        features: vec![
            Feature {
                id: "feat-worktree".to_string(),
                name: "worktree-newer".to_string(),
                branch: "worktree-newer".to_string(),
                workdir: PathBuf::from("/tmp/test-repo/.worktrees/worktree-newer"),
                is_worktree: true,
                tmux_session: "aiw-worktree-newer".to_string(),
                sessions: vec![],
                collapsed: false,
                mode: VibeMode::default(),
                review: false,
                agent: AgentKind::default(),
                enable_chrome: false,
                pending_worktree_script: false,
                ready: false,
                status: ProjectStatus::Stopped,
                created_at: now + Duration::minutes(1),
                last_accessed: now + Duration::minutes(1),
                summary: None,
                summary_updated_at: None,
                nickname: None,
                triage_source: None,
                review_source: None,
                issue_source: None,
            },
            Feature {
                id: "feat-repo".to_string(),
                name: "repo-older".to_string(),
                branch: "repo-older".to_string(),
                workdir: PathBuf::from("/tmp/test-repo"),
                is_worktree: false,
                tmux_session: "aiw-repo-older".to_string(),
                sessions: vec![],
                collapsed: false,
                mode: VibeMode::default(),
                review: false,
                agent: AgentKind::default(),
                enable_chrome: false,
                pending_worktree_script: false,
                ready: false,
                status: ProjectStatus::Stopped,
                created_at: now,
                last_accessed: now,
                summary: None,
                summary_updated_at: None,
                nickname: None,
                triage_source: None,
                review_source: None,
                issue_source: None,
            },
        ],
        created_at: now,
        preferred_agent: AgentKind::default(),
        is_git: true,
    };
    let store = ProjectStore {
        version: 2,
        projects: vec![project],
        session_bookmarks: vec![],
        available_harnesses: vec![],
        prompt_templates: Vec::new(),
        extra: HashMap::new(),
    };

    let app = App::new_for_test(
        store,
        Box::new(MockTmuxOps::new()),
        Box::new(MockWorktreeOps::new()),
    );
    let visible = app.visible_items();

    assert!(matches!(visible[0], VisibleItem::Project(0)));
    assert!(matches!(visible[1], VisibleItem::Feature(0, 1)));
    assert!(matches!(visible[2], VisibleItem::Feature(0, 0)));
    assert_eq!(
        visible.len(),
        3,
        "a persisted current plan must not become a dashboard tree item"
    );
}

// FORK(cockpit): the sidebar gives a session a second line (its reason)
// while it is selected or needs you — not for a status line.
#[test]
fn ensure_selection_visible_accounts_for_multi_line_sessions() {
    let mut store = store_with_feature(ProjectStatus::Stopped);
    store.projects[0].features[0].sessions = vec![
        make_session("claude-1", Some("running")),
        make_session("claude-2", Some("running")),
        make_session("claude-3", None),
    ];

    let mut app = App::new_for_test(
        store,
        Box::new(MockTmuxOps::new()),
        Box::new(MockWorktreeOps::new()),
    );
    app.selection = Selection::Session(0, 0, 1);
    assert_eq!(app.visible_item_height(&VisibleItem::Session(0, 0, 1)), 2);
    assert_eq!(app.visible_item_height(&VisibleItem::Session(0, 0, 0)), 1);

    // project, feature, claude-1, [claude-2 ×2]: five rows, four visible.
    app.ensure_selection_visible(4);

    assert_eq!(app.scroll_offset, 1);
}

#[test]
fn item_index_at_visible_row_maps_status_line_to_same_session() {
    let mut store = store_with_feature(ProjectStatus::Stopped);
    store.projects[0].features[0].sessions = vec![
        make_session("claude-1", Some("running")),
        make_session("claude-2", None),
    ];

    let mut app = App::new_for_test(
        store,
        Box::new(MockTmuxOps::new()),
        Box::new(MockWorktreeOps::new()),
    );
    // The selected session takes two rows (its reason line).
    app.selection = Selection::Session(0, 0, 0);

    assert!(matches!(app.item_index_at_visible_row(0, 4), Some(0)));
    assert!(matches!(app.item_index_at_visible_row(1, 4), Some(1)));
    assert!(matches!(app.item_index_at_visible_row(2, 4), Some(2)));
    assert!(matches!(app.item_index_at_visible_row(3, 4), Some(2)));
    assert_eq!(app.item_index_at_visible_row(4, 4), None);
}

#[test]
fn usability_search_types_navigation_letters_and_reveals_collapsed_result() {
    let mut store = store_with_feature(ProjectStatus::Stopped);
    store.projects[0].collapsed = true;
    store.projects[0].features[0].collapsed = true;
    store.projects[0].features[0]
        .sessions
        .push(make_session("jkl", None));
    let mut app = App::new_for_test(
        store,
        Box::new(MockTmuxOps::new()),
        Box::new(MockWorktreeOps::new()),
    );
    app.start_search();
    assert!(matches!(&app.mode, AppMode::Searching(s) if s.matches.len() == 3));
    for c in "jkl".chars() {
        crate::handlers::handle_search_key(&mut app, KeyCode::Char(c)).unwrap();
    }
    assert!(matches!(&app.mode, AppMode::Searching(s) if s.query == "jkl" && s.matches.len() == 1));
    crate::handlers::handle_search_key(&mut app, KeyCode::Enter).unwrap();
    assert!(matches!(app.selection, Selection::Session(0, 0, 0)));
    assert!(
        app.visible_items()
            .iter()
            .any(|item| matches!(item, VisibleItem::Session(0, 0, 0)))
    );
}

#[test]
fn usability_search_tab_navigation_and_empty_results_are_safe() {
    let mut app = App::new_for_test(
        store_with_feature(ProjectStatus::Stopped),
        Box::new(MockTmuxOps::new()),
        Box::new(MockWorktreeOps::new()),
    );
    app.start_search();
    crate::handlers::handle_search_key(&mut app, KeyCode::BackTab).unwrap();
    assert!(matches!(&app.mode, AppMode::Searching(s) if s.selected_match == 1));
    crate::handlers::handle_search_key(&mut app, KeyCode::Tab).unwrap();
    assert!(matches!(&app.mode, AppMode::Searching(s) if s.selected_match == 0));
    crate::handlers::handle_search_key(&mut app, KeyCode::Char('!')).unwrap();
    crate::handlers::handle_search_key(&mut app, KeyCode::Enter).unwrap();
    assert!(matches!(&app.mode, AppMode::Searching(s) if s.matches.is_empty()));
    crate::handlers::handle_search_key(&mut app, KeyCode::Backspace).unwrap();
    assert!(matches!(&app.mode, AppMode::Searching(s) if s.matches.len() == 2));
}

#[test]
fn usability_help_end_then_up_moves_immediately() {
    let mut app = App::new_for_test(
        ProjectStore::empty(),
        Box::new(MockTmuxOps::new()),
        Box::new(MockWorktreeOps::new()),
    );
    app.mode = AppMode::Help(HelpState {
        from_view: None,
        scroll_offset: 0,
    });
    crate::handlers::handle_help_key(&mut app, KeyEvent::new(KeyCode::End, KeyModifiers::NONE))
        .unwrap();
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
    terminal
        .draw(|frame| crate::ui::draw(frame, &mut app))
        .unwrap();
    let bottom = match &app.mode {
        AppMode::Help(s) => s.scroll_offset,
        _ => unreachable!(),
    };
    assert!(bottom > 0 && bottom < 1000);
    crate::handlers::handle_help_key(&mut app, KeyEvent::new(KeyCode::Up, KeyModifiers::NONE))
        .unwrap();
    terminal
        .draw(|frame| crate::ui::draw(frame, &mut app))
        .unwrap();
    assert!(matches!(&app.mode, AppMode::Help(s) if s.scroll_offset == bottom - 1));
}

#[test]
fn slugify_special_chars_become_hyphens() {
    assert_eq!(slugify("foo/bar.baz"), "foo-bar-baz");
}

#[test]
fn usability_search_finds_the_feature_name_shown_on_the_dashboard() {
    let mut store = store_with_feature(ProjectStatus::Stopped);
    store.projects[0].features[0].nickname = Some("Release polish".into());
    let mut app = App::new_for_test(
        store,
        Box::new(MockTmuxOps::new()),
        Box::new(MockWorktreeOps::new()),
    );
    app.start_search();
    crate::handlers::handle_paste(&mut app, "release").unwrap();
    assert!(
        matches!(&app.mode, AppMode::Searching(s) if s.matches.len() == 1 && s.matches[0].label == "Release polish")
    );
}
