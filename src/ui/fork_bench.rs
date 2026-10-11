//! FORK: dashboard frame benchmark (DESIGN.md §12, G11 budget).
//!
//! Renders the dashboard into a ratatui `TestBackend` at 200×60 with a
//! realistic store (3 projects × 10 sessions, a mix of working / needs-you /
//! finished / stopped states, PR badges, token usage) and reports the mean and
//! p95 frame time. The budget (≤ 2 ms per frame) only applies to a release
//! build, so the timing test is `#[ignore]`d:
//!
//! ```bash
//! cargo +1.99.0 test --release --locked dashboard_frame_bench -- --ignored --nocapture
//! ```
//!
//! The always-on test below renders the same store through a `MockTmuxOps`
//! with no expectations, so any tmux call made while drawing fails it.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use chrono::{Duration as ChronoDuration, Utc};
use ratatui::{Terminal, backend::TestBackend};

use crate::app::App;
use crate::app::attention::{AttentionRecord, AttentionState};
use crate::project::{
    AgentKind, Feature, FeatureSession, Project, ProjectStatus, ProjectStore, SessionKind, VibeMode,
};
use crate::traits::{MockTmuxOps, MockWorktreeOps};

const WIDTH: u16 = 200;
const HEIGHT: u16 = 60;

fn session(
    index: usize,
    kind: SessionKind,
    label: &str,
    status_text: Option<&str>,
) -> FeatureSession {
    FeatureSession {
        id: format!("session-{index}"),
        kind,
        label: label.to_string(),
        tmux_window: format!("{label}-{index}"),
        claude_session_id: None,
        todo_reference: None,
        token_usage_source: None,
        token_usage_source_match: None,
        created_at: Utc::now(),
        command: None,
        on_stop: None,
        pre_check: None,
        stopped: index % 7 == 6,
        status_text: status_text.map(str::to_string),
        token_usage: None,
    }
}

/// 3 projects, each with 4 features holding 3 + 3 + 2 + 2 = 10 sessions.
pub(crate) fn realistic_store() -> ProjectStore {
    let now = Utc::now();
    let names = ["harbor-api", "billing-app", "workflow-engine"];
    let mut session_index = 0;
    let projects = names
        .iter()
        .enumerate()
        .map(|(pi, name)| {
            let features = [3usize, 3, 2, 2]
                .iter()
                .enumerate()
                .map(|(fi, count)| {
                    let sessions = (0..*count)
                        .map(|si| {
                            session_index += 1;
                            let (kind, label) = match si {
                                0 => (SessionKind::Claude, "claude"),
                                1 => (SessionKind::Codex, "codex"),
                                _ => (SessionKind::Terminal, "terminal"),
                            };
                            let status = (si == 0 && fi % 2 == 0)
                                .then_some("Editing src/auth/sso.rs — running cargo test");
                            session(session_index, kind, label, status)
                        })
                        .collect();
                    let feature_name = format!("{}-{}", name.split('-').next().unwrap(), fi + 12);
                    Feature {
                        id: format!("feat-{pi}-{fi}"),
                        name: feature_name.clone(),
                        branch: format!("feature/{feature_name}"),
                        workdir: PathBuf::from(format!(
                            "/home/dev/.local/share/ai_workstation/worktrees/{name}/{feature_name}"
                        )),
                        is_worktree: fi != 3,
                        tmux_session: format!("aiw-{feature_name}"),
                        sessions,
                        collapsed: false,
                        mode: [VibeMode::Vibeless, VibeMode::Vibe, VibeMode::SuperVibe][fi % 3]
                            .clone(),
                        review: fi == 1,
                        plan_mode: false,
                        agent: AgentKind::Claude,
                        enable_chrome: false,
                        pending_worktree_script: false,
                        ready: fi == 2,
                        status: [
                            ProjectStatus::Active,
                            ProjectStatus::Active,
                            ProjectStatus::Idle,
                            ProjectStatus::Stopped,
                        ][fi]
                            .clone(),
                        created_at: now - ChronoDuration::minutes((pi * 40 + fi * 7) as i64),
                        last_accessed: now,
                        summary: (fi == 0)
                            .then(|| "Wire SSO callback into the session store".into()),
                        summary_updated_at: None,
                        nickname: None,
                        selected_plan_path: None,
                        triage_source: None,
                        review_source: None,
                        issue_source: None,
                    }
                })
                .collect();
            Project {
                id: format!("proj-{pi}"),
                name: name.to_string(),
                repo: PathBuf::from(format!("/home/dev/code/{name}")),
                collapsed: false,
                features,
                created_at: now,
                preferred_agent: AgentKind::Claude,
                is_git: true,
            }
        })
        .collect();
    ProjectStore {
        version: 5,
        projects,
        session_bookmarks: vec![],
        available_harnesses: vec![],
        prompt_templates: Vec::new(),
        extra: HashMap::new(),
    }
}

/// An app over [`realistic_store`] with live-looking runtime state: two
/// working features, a question, a finished turn, an open PR.
pub(crate) fn realistic_app(tmux: MockTmuxOps) -> App {
    let mut app = App::new_for_test(
        realistic_store(),
        Box::new(tmux),
        Box::new(MockWorktreeOps::new()),
    );
    // The tree, explicitly: the chrome and bench tests read tree rows. The
    // workspace benches and tests set the layout themselves.
    app.selection = crate::app::Selection::Feature(1, 1);
    app.thinking_features.insert("aiw-harbor-12".to_string());
    app.thinking_features.insert("aiw-workflow-12".to_string());
    app.attention.insert(
        "aiw-billing-12".to_string(),
        AttentionRecord::new(AgentKind::Claude, AttentionState::Question, Utc::now()),
    );
    app.attention.insert(
        "aiw-harbor-13".to_string(),
        AttentionRecord::new(
            AgentKind::Claude,
            AttentionState::CompletedAwaitingReview,
            Utc::now(),
        ),
    );
    app.active_prs.insert(
        "feat-0-1".to_string(),
        crate::app::ActivePrStatus {
            branch: "feature/harbor-13".to_string(),
            head_sha: "abc123".to_string(),
            number: 482,
            unresolved_threads: Some(2),
        },
    );
    app
}

fn render(terminal: &mut Terminal<TestBackend>, app: &mut App) {
    terminal.draw(|frame| super::draw(frame, app)).unwrap();
}

#[test]
fn dashboard_frame_makes_no_tmux_calls() {
    // `MockTmuxOps` without expectations panics on any call, so a frame that
    // shells out to tmux fails here.
    let mut app = realistic_app(MockTmuxOps::new());
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).unwrap();
    for _ in 0..3 {
        render(&mut terminal, &mut app);
    }
}

#[test]
#[ignore = "timing benchmark; run in release with --ignored --nocapture"]
fn dashboard_frame_bench() {
    const WARMUP: usize = 50;
    const FRAMES: usize = 2_000;

    let mut app = realistic_app(MockTmuxOps::new());
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, HEIGHT)).unwrap();
    for _ in 0..WARMUP {
        render(&mut terminal, &mut app);
    }

    let mut samples: Vec<Duration> = Vec::with_capacity(FRAMES);
    for frame in 0..FRAMES {
        // Alternate the selection so every frame really repaints rows.
        app.selection = if frame % 2 == 0 {
            crate::app::Selection::Feature(1, 1)
        } else {
            crate::app::Selection::Session(0, 0, 1)
        };
        let started = Instant::now();
        render(&mut terminal, &mut app);
        samples.push(started.elapsed());
    }
    samples.sort_unstable();
    let total: Duration = samples.iter().sum();
    let mean = total / FRAMES as u32;
    let p50 = samples[FRAMES / 2];
    let p95 = samples[FRAMES * 95 / 100];
    let max = samples[FRAMES - 1];
    println!(
        "dashboard frame {WIDTH}x{HEIGHT} ({} build): mean {:.1}us p50 {:.1}us p95 {:.1}us max {:.1}us over {FRAMES} frames",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        mean.as_secs_f64() * 1e6,
        p50.as_secs_f64() * 1e6,
        p95.as_secs_f64() * 1e6,
        max.as_secs_f64() * 1e6,
    );
    if !cfg!(debug_assertions) {
        assert!(
            p95 <= Duration::from_millis(2),
            "dashboard frame p95 {p95:?} is over the 2 ms budget"
        );
    }
}

/// The same frame budget for the cockpit sidebar (DESIGN.md §18): a 36-column
/// pane with badges on every row and the info block, the selection
/// alternating between a row needing you and a working one.
#[test]
#[ignore = "timing benchmark; run in release with --ignored --nocapture"]
fn workspace_frame_bench() {
    const WARMUP: usize = 50;
    const FRAMES: usize = 2_000;
    const SIDEBAR: u16 = crate::fork::cockpit::DEFAULT_SIDEBAR_WIDTH;

    let mut app = realistic_app(MockTmuxOps::new());
    app.cockpit_attach(
        crate::fork::cockpit::Link::recording(),
        "/bin/aiw".into(),
        "/s/cockpit.sock".into(),
    );
    app.viewport_cols = SIDEBAR;
    app.viewport_total_rows = HEIGHT;
    app.viewport_rows = HEIGHT - 3;
    app.selection = crate::app::Selection::Session(1, 0, 0);
    app.workspace_commit_now();
    let mut terminal = Terminal::new(TestBackend::new(SIDEBAR, HEIGHT)).unwrap();
    for _ in 0..WARMUP {
        render(&mut terminal, &mut app);
    }

    let mut samples: Vec<Duration> = Vec::with_capacity(FRAMES);
    for frame in 0..FRAMES {
        app.selection = if frame % 2 == 0 {
            crate::app::Selection::Session(1, 0, 0)
        } else {
            crate::app::Selection::Session(0, 0, 1)
        };
        app.refresh_badge_ages();
        let started = Instant::now();
        render(&mut terminal, &mut app);
        samples.push(started.elapsed());
    }
    samples.sort_unstable();
    let total: Duration = samples.iter().sum();
    let mean = total / FRAMES as u32;
    let p50 = samples[FRAMES / 2];
    let p95 = samples[FRAMES * 95 / 100];
    let max = samples[FRAMES - 1];
    println!(
        "sidebar frame {SIDEBAR}x{HEIGHT} ({} build): mean {:.1}us p50 {:.1}us p95 {:.1}us max {:.1}us over {FRAMES} frames",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        mean.as_secs_f64() * 1e6,
        p50.as_secs_f64() * 1e6,
        p95.as_secs_f64() * 1e6,
        max.as_secs_f64() * 1e6,
    );
    if !cfg!(debug_assertions) {
        assert!(
            p95 <= Duration::from_millis(2),
            "workspace frame p95 {p95:?} is over the 2 ms budget"
        );
    }
}

/// Print the dashboard as text (for DESIGN.md sketches and eyeballing):
/// `cargo +1.99.0 test --locked dashboard_text_snapshot -- --ignored --nocapture`.
#[test]
#[ignore = "prints the rendered dashboard; nothing to assert"]
fn dashboard_text_snapshot() {
    let mut app = realistic_app(MockTmuxOps::new());
    // `AIW_SNAPSHOT_LAYOUT=workspace` prints a wide sidebar instead.
    if std::env::var("AIW_SNAPSHOT_LAYOUT").as_deref() == Ok("workspace") {
        app.selection = crate::app::Selection::Session(1, 0, 0);
        app.viewport_cols = 120;
        app.workspace_tick();
        std::thread::sleep(
            crate::app::fork_workspace::SELECTION_DEBOUNCE + Duration::from_millis(20),
        );
        app.workspace_tick();
    }
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    render(&mut terminal, &mut app);
    let buffer = terminal.backend().buffer();
    for y in 0..buffer.area.height {
        let row: String = (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect();
        println!("{}", row.trim_end());
    }
}
