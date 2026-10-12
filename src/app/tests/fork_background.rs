//! FORK: background Claude sessions at the App level (DESIGN.md §19.7):
//! the `s` picker and New Feature dialog routes, the `claude --bg` launch,
//! state from the roster, attach / detach, and stop / restart / rm.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::KeyCode;
use tempfile::TempDir;

use super::fork_fan_out::{Fixture, fixture, key, step, type_text};
use crate::app::*;
use crate::fork::background::BgState;
use crate::fork::badges::{BadgeKind, session_badge};
use crate::project::{AgentKind, Feature, ProjectStatus, SessionKind, VibeMode};

const FAKE: &str = r#"#!/bin/sh
dir=$(dirname "$0")
printf '%s\n' "$*" >> "$dir/claude.log"
case "$1" in
  --bg) echo "backgrounded abc12345"; echo "  claude attach abc12345   open in this terminal" ;;
  --resume) echo "backgrounded def67890" ;;
  stop) echo "stopped $2" ;;
  respawn) echo "respawned $2" ;;
  logs) printf 'line one\nline two\n\033[1mthe last line\033[0m\n' ;;
  rm)
    if [ -n "$3" ]; then echo "removed $2"; else
      echo "Not removing $2: the worktree has uncommitted changes. To discard them, run 'claude rm $2 --force-remove-worktree wt-1'" >&2
      exit 1
    fi ;;
  agents) cat "$dir/roster.json" 2>/dev/null || echo '[]' ;;
esac
"#;

struct Bg {
    f: Fixture,
    dir: TempDir,
}

impl Bg {
    fn log(&self) -> Vec<String> {
        std::fs::read_to_string(self.dir.path().join("claude.log"))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn roster(&self, json: &str) {
        std::fs::write(self.dir.path().join("roster.json"), json).unwrap();
    }

    /// Tick until `done` holds (commands run on threads).
    fn wait(&mut self, what: &str, done: impl Fn(&App) -> bool) {
        let started = Instant::now();
        while !done(&self.f.app) {
            assert!(
                started.elapsed() < Duration::from_secs(10),
                "timed out waiting for {what}; log: {:?}",
                self.log()
            );
            self.f.app.background_tick();
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// The fan-out fixture plus a fake `claude` and one feature in the repo dir.
fn bg() -> Bg {
    let mut f = fixture();
    let dir = TempDir::new().unwrap();
    let bin = dir.path().join("claude");
    std::fs::write(&bin, FAKE).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    f.app.background.claude_bin = Some(bin.to_string_lossy().into_owned());
    let repo = f.app.store.projects[0].repo.clone();
    let mut feature = Feature::new_for_project(
        "automation-project",
        "feat".into(),
        "feat".into(),
        repo,
        false,
        VibeMode::Vibe,
        AgentKind::Claude,
        false,
    );
    feature.status = ProjectStatus::Stopped;
    f.app.store.projects[0].features.push(feature);
    Bg { f, dir }
}

fn feature(app: &App, name: &str) -> (usize, usize) {
    let fi = app.store.projects[0]
        .features
        .iter()
        .position(|f| f.name == name)
        .unwrap_or_else(|| panic!("no feature {name}"));
    (0, fi)
}

fn add_bg(t: &mut Bg, prompt: &str) -> String {
    let (pi, fi) = feature(&t.f.app, "feat");
    t.f.app
        .create_background_session(pi, fi, "Claude bg 1", prompt)
        .unwrap()
}

fn launched(t: &mut Bg, id: &str) {
    let session = id.to_string();
    t.wait("the launch", |app| app.background_id(&session).is_some());
}

fn indices(app: &App, id: &str) -> (usize, usize, usize) {
    app.session_indices_by_id(id).unwrap()
}

#[test]
fn the_picker_names_prompts_and_launches_a_background_session() {
    let mut t = bg();
    let app = &mut t.f.app;
    app.selection = Selection::Feature(0, 0);
    app.open_session_picker().unwrap();
    let AppMode::SessionPicker(state) = &mut app.mode else {
        panic!("no picker");
    };
    let at = state
        .builtin_sessions
        .iter()
        .position(|o| o.background)
        .expect("the background entry is offered");
    assert_eq!(state.builtin_sessions[at].label, "Claude (background)");
    state.selected = at;
    crate::handlers::handle_session_picker_key(app, KeyCode::Enter).unwrap();
    let AppMode::NamingNewSession(naming) = &app.mode else {
        panic!("not naming");
    };
    assert_eq!(naming.input, "Claude bg 1");
    crate::handlers::handle_new_session_name_key(app, KeyCode::Enter).unwrap();
    assert!(matches!(app.mode, AppMode::BackgroundPrompt(_)));
    for c in "fix the flaky test".chars() {
        crate::handlers::handle_key(app, KeyCode::Char(c).into()).unwrap();
    }
    crate::handlers::handle_key(app, KeyCode::Enter.into()).unwrap();
    assert!(matches!(app.mode, AppMode::Normal));

    let session = app.store.projects[0].features[0]
        .sessions
        .iter()
        .find(|s| s.label == "Claude bg 1")
        .expect("added")
        .clone();
    assert!(session.is_background());
    assert_eq!(session.kind, SessionKind::Claude);
    assert_eq!(session.tmux_window, "aiw-bg-claude");
    assert_eq!(app.background_state(&session.id), BgState::Starting);

    launched(&mut t, &session.id);
    let app = &t.f.app;
    assert_eq!(app.background_id(&session.id), Some("abc12345"));
    let line = t.log().into_iter().find(|l| l.starts_with("--bg")).unwrap();
    assert!(line.contains("--settings "), "{line}");
    assert!(line.contains("--name feat-claude-bg-1"), "{line}");
    assert!(
        line.contains("--permission-mode acceptEdits"),
        "Vibe mode: {line}"
    );
    assert!(line.ends_with("fix the flaky test"), "{line}");
    // The hook environment rides on the settings file; Claude's supervisor
    // starts the process, not AIW's shell.
    let settings = crate::fork::launch::claude_settings_path(&session.id);
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(settings).unwrap()).unwrap();
    assert_eq!(v["env"]["AIW_FEATURE_SESSION_ID"], session.id.as_str());
    assert_eq!(v["env"]["AIW_ACTIVE"], "1");
    assert_eq!(v["worktree"]["bgIsolation"], "none");
    assert!(
        v["statusLine"].is_object(),
        "AIW's usual settings are still there"
    );
    // No pane, no tmux session for it.
    let calls = t.f.calls.lock().unwrap();
    assert!(calls.launches.is_empty(), "{:?}", calls.launches);
    assert!(calls.sessions.is_empty());
    drop(calls);
    // Persisted.
    let stored =
        t.f.app
            .db
            .as_ref()
            .unwrap()
            .all_background_sessions()
            .unwrap();
    assert_eq!(stored[&session.id].bg_id.as_deref(), Some("abc12345"));
}

#[test]
fn the_new_feature_dialog_runs_claude_in_the_background() {
    let mut t = bg();
    super::fork_fan_out::open_dialog(&mut t.f.app, "bg-feat", true);
    if let AppMode::CreatingFeature(state) = &t.f.app.mode {
        assert!(state.background_offered);
        assert!(state.background_available());
        assert_eq!(state.mode_focus, 0, "on the harness list");
    }
    key(&mut t.f.app, KeyCode::Char('b'));
    if let AppMode::CreatingFeature(state) = &t.f.app.mode {
        assert!(state.background_selected());
        assert_eq!(
            state.max_mode_focus(),
            2,
            "no extra row: the Mode step is unchanged"
        );
        assert!(
            state
                .focused_mode_description()
                .unwrap()
                .contains("claude --bg")
        );
    }
    // Ticking a second harness (a fan-out) turns it off.
    key(&mut t.f.app, KeyCode::Char(' '));
    key(&mut t.f.app, KeyCode::Char('j'));
    key(&mut t.f.app, KeyCode::Char(' '));
    if let AppMode::CreatingFeature(state) = &t.f.app.mode {
        assert!(!state.background_selected(), "not for a fan-out");
    }
    key(&mut t.f.app, KeyCode::Char(' ')); // untick Codex
    key(&mut t.f.app, KeyCode::Char('k'));
    key(&mut t.f.app, KeyCode::Char(' ')); // untick Claude
    if let AppMode::CreatingFeature(state) = &t.f.app.mode {
        assert!(state.background_selected(), "back to a single Claude");
    }
    while step(&t.f.app) != CreateFeatureStep::Prompt {
        key(&mut t.f.app, KeyCode::Enter);
    }
    type_text(&mut t.f.app, "port the parser");
    key(&mut t.f.app, KeyCode::Enter);
    let (pi, fi) = feature(&t.f.app, "bg-feat");
    let feature = t.f.app.store.projects[pi].features[fi].clone();
    let claude = feature
        .sessions
        .iter()
        .find(|s| s.kind == SessionKind::Claude)
        .unwrap()
        .clone();
    assert!(claude.is_background(), "{:?}", claude.tmux_window);
    // Nothing of it runs in tmux: no session, no launch, no pasted prompt.
    {
        let calls = t.f.calls.lock().unwrap();
        assert!(!calls.sessions.contains(&feature.tmux_session));
        assert!(calls.launches.is_empty(), "{:?}", calls.launches);
        assert!(calls.pastes.is_empty(), "{:?}", calls.pastes);
    }
    launched(&mut t, &claude.id);
    let line = t.log().into_iter().find(|l| l.starts_with("--bg")).unwrap();
    assert!(line.ends_with("port the parser"), "the task prompt: {line}");
    assert!(t.f.app.pending_prompt_for(&claude.id).is_none());
}

#[test]
fn roster_states_and_hooks_drive_the_badge() {
    let mut t = bg();
    let id = add_bg(&mut t, "");
    launched(&mut t, &id);
    let (pi, fi, si) = indices(&t.f.app, &id);
    let now = chrono::Utc::now();
    let entry = |state: &str, extra: &str| {
        format!(
            r#"[{{"cwd":"/w","kind":"background","id":"abc12345","state":"{state}","sessionId":"conv-1"{extra}}}]"#
        )
    };
    let apply = |t: &mut Bg, json: &str| {
        let roster = crate::fork::claude_roster::parse(json);
        t.f.app.apply_claude_roster(roster);
    };
    apply(&mut t, &entry("working", r#","status":"busy""#));
    let badge = session_badge(&t.f.app, pi, fi, si, now);
    assert_eq!(badge.kind, BadgeKind::Working);
    assert!(
        badge.label_compact().starts_with("bg · working"),
        "{}",
        badge.label_compact()
    );
    assert_eq!(
        t.f.app.background.records[&id].conversation_id.as_deref(),
        Some("conv-1"),
        "the conversation id is learned from the roster"
    );
    apply(
        &mut t,
        &entry(
            "blocked",
            r#","status":"waiting","waitingFor":"permission prompt""#,
        ),
    );
    let badge = session_badge(&t.f.app, pi, fi, si, now);
    assert_eq!(badge.kind, BadgeKind::NeedsYou);
    assert_eq!(badge.label_compact(), "bg · permission prompt");
    // The feature is stopped (no tmux session) yet shows what its
    // background agent does.
    assert_eq!(
        crate::fork::badges::feature_badge(&t.f.app, pi, fi, now).kind,
        BadgeKind::NeedsYou
    );
    apply(&mut t, &entry("stopped", ""));
    let badge = session_badge(&t.f.app, pi, fi, si, now);
    assert_eq!(badge.kind, BadgeKind::Stopped);
    assert_eq!(badge.label_compact(), "bg · stopped · Enter resumes");
    apply(&mut t, "[]");
    assert_eq!(
        session_badge(&t.f.app, pi, fi, si, now).label_compact(),
        "bg · not running · c resumes"
    );
    // A roster read through the tick (the fake `claude agents --json`).
    t.roster(&entry("done", ""));
    t.wait("a roster read", |app| {
        app.background_state(&id) == BgState::Done
    });
}

#[test]
fn attach_opens_a_window_that_closes_on_detach() {
    let mut t = bg();
    let id = add_bg(&mut t, "");
    launched(&mut t, &id);
    t.f.app
        .apply_claude_roster(crate::fork::claude_roster::parse(
            r#"[{"cwd":"/w","kind":"background","id":"abc12345","state":"working"}]"#,
        ));
    let (pi, fi, si) = indices(&t.f.app, &id);
    t.f.app.selection = Selection::Session(pi, fi, si);
    assert!(
        !t.f.app.desired_live_target_exists(),
        "nothing to show before attach"
    );
    crate::handlers::handle_key(&mut t.f.app, KeyCode::Enter.into()).unwrap();
    let tmux_session = t.f.app.store.projects[pi].features[fi].tmux_session.clone();
    {
        let calls = t.f.calls.lock().unwrap();
        assert!(
            calls.sessions.contains(&tmux_session),
            "the feature's tmux session"
        );
        let (s, w, cmd) = calls.shell.last().expect("attach command");
        assert_eq!(
            (s.as_str(), w.as_str()),
            (tmux_session.as_str(), "aiw-bg-claude")
        );
        assert!(cmd.contains("attach 'abc12345'"), "{cmd}");
        assert!(cmd.contains("kill-window -t '="), "closes itself: {cmd}");
    }
    assert!(t.f.app.background_attached(&id));
    assert!(
        t.f.app.desired_live_target_exists(),
        "the pane shows the attach window"
    );
    // Detach: the window is gone.
    t.f.calls.lock().unwrap().sessions.clear();
    t.f.app
        .recent_launches
        .insert(id.clone(), Instant::now() - Duration::from_secs(10));
    t.f.app.background_tick();
    assert!(!t.f.app.background_attached(&id));
    assert!(!t.f.app.desired_live_target_exists());
    // The exited sweep never touched it.
    assert!(!t.f.app.store.projects[pi].features[fi].sessions[si].stopped);
}

#[test]
fn stop_restart_and_a_refused_rm_then_a_confirmed_one() {
    let mut t = bg();
    let id = add_bg(&mut t, "");
    launched(&mut t, &id);
    t.f.app
        .apply_claude_roster(crate::fork::claude_roster::parse(
            r#"[{"cwd":"/w","kind":"background","id":"abc12345","state":"working"}]"#,
        ));
    let (pi, fi, si) = indices(&t.f.app, &id);
    t.f.app.selection = Selection::Session(pi, fi, si);
    crate::handlers::handle_key(&mut t.f.app, KeyCode::Char('x').into()).unwrap();
    t.wait("stop", |app| {
        !app.background.is_running(&id, fork_background::Op::Stop)
    });
    crate::handlers::handle_key(&mut t.f.app, KeyCode::Char('c').into()).unwrap();
    t.wait("respawn", |app| {
        !app.background.is_running(&id, fork_background::Op::Respawn)
    });
    let log = t.log();
    assert!(log.contains(&"stop abc12345".to_string()), "{log:?}");
    assert!(log.contains(&"respawn abc12345".to_string()), "{log:?}");

    crate::handlers::handle_key(&mut t.f.app, KeyCode::Char('d').into()).unwrap();
    assert!(matches!(t.f.app.mode, AppMode::BackgroundRemove(_)));
    crate::handlers::handle_key(&mut t.f.app, KeyCode::Char('y').into()).unwrap();
    t.wait("the refusal", |app| {
        matches!(&app.mode, AppMode::BackgroundRemove(s) if s.refusal.is_some() && !s.running)
    });
    if let AppMode::BackgroundRemove(state) = &t.f.app.mode {
        let refusal = state.refusal.as_ref().unwrap();
        assert_eq!(refusal.flags, ["--force-remove-worktree", "wt-1"]);
        assert!(refusal.text.contains("uncommitted changes"));
    }
    assert!(
        t.f.app.session_indices_by_id(&id).is_some(),
        "a refusal removes nothing"
    );
    crate::handlers::handle_key(&mut t.f.app, KeyCode::Char('y').into()).unwrap();
    t.wait("the removal", |app| {
        app.session_indices_by_id(&id).is_none()
    });
    assert!(matches!(t.f.app.mode, AppMode::Normal));
    let log = t.log();
    assert!(
        log.contains(&"rm abc12345 --force-remove-worktree wt-1".to_string()),
        "{log:?}"
    );
    assert!(
        !t.f.app
            .db
            .as_ref()
            .unwrap()
            .all_background_sessions()
            .unwrap()
            .contains_key(&id)
    );
    // Removed by `rm`: no `stop` follows.
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        t.log().iter().filter(|l| l.starts_with("stop")).count(),
        1,
        "{:?}",
        t.log()
    );
}

#[test]
fn a_session_claude_no_longer_lists_resumes_its_conversation() {
    let mut t = bg();
    let id = add_bg(&mut t, "");
    launched(&mut t, &id);
    t.f.app
        .background
        .records
        .get_mut(&id)
        .unwrap()
        .conversation_id = Some("conv-9".into());
    t.f.app
        .apply_claude_roster(crate::fork::claude_roster::parse("[]"));
    assert_eq!(t.f.app.background_state(&id), BgState::Gone);
    let (pi, fi, si) = indices(&t.f.app, &id);
    t.f.app.selection = Selection::Session(pi, fi, si);
    crate::handlers::handle_key(&mut t.f.app, KeyCode::Char('c').into()).unwrap();
    t.wait("the resume", |app| {
        app.background_id(&id) == Some("def67890")
    });
    assert!(
        t.log().contains(&"--resume conv-9 --bg".to_string()),
        "{:?}",
        t.log()
    );
}

#[test]
fn the_selected_session_shows_its_latest_output() {
    let mut t = bg();
    let id = add_bg(&mut t, "");
    launched(&mut t, &id);
    let (pi, fi, si) = indices(&t.f.app, &id);
    t.f.app.selection = Selection::Session(pi, fi, si);
    t.wait("the logs", |app| {
        app.background_info_line(&id).ends_with("the last line")
    });
    let info =
        t.f.app
            .session_info_for(pi, fi, si, chrono::Utc::now())
            .unwrap();
    assert_eq!(info.harness, "Claude Code (background)");
    assert!(
        info.background.unwrap().starts_with("bg abc12345 · "),
        "id and state lead the line"
    );
    assert!(t.log().contains(&"logs abc12345".to_string()));
}

#[test]
fn a_feature_start_skips_background_sessions() {
    let mut t = bg();
    let (pi, fi) = feature(&t.f.app, "feat");
    t.f.app.store.projects[pi].features[fi].add_session(SessionKind::Claude);
    let id = add_bg(&mut t, "");
    launched(&mut t, &id);
    t.f.app
        .ensure_feature_running(pi, fi, crate::app::resource_gate::StartIntent::Approved)
        .unwrap();
    let calls = t.f.calls.lock().unwrap();
    assert_eq!(
        calls
            .launches
            .iter()
            .map(|(h, _, w)| (h.as_str(), w.as_str()))
            .collect::<Vec<_>>(),
        [("claude", "claude")],
        "only the interactive Claude"
    );
}

#[test]
fn a_background_only_feature_start_opens_no_tmux_session() {
    let mut t = bg();
    let id = add_bg(&mut t, "");
    launched(&mut t, &id);
    let (pi, fi) = feature(&t.f.app, "feat");
    t.f.app
        .apply_claude_roster(crate::fork::claude_roster::parse("[]"));
    t.f.app
        .ensure_feature_running(pi, fi, crate::app::resource_gate::StartIntent::Approved)
        .unwrap();
    assert!(t.f.calls.lock().unwrap().sessions.is_empty());
    // `c` on the feature (re)starts the background session Claude dropped.
    assert!(
        t.f.app.background.pending.contains_key(&id)
            || t.log()
                .iter()
                .any(|l| l.starts_with("--resume") || l.starts_with("respawn")),
        "{:?}",
        t.log()
    );
}

#[test]
fn removing_the_row_stops_the_session_in_claude() {
    let mut t = bg();
    let id = add_bg(&mut t, "");
    launched(&mut t, &id);
    let (pi, fi, si) = indices(&t.f.app, &id);
    t.f.app.selection = Selection::Session(pi, fi, si);
    t.f.app.remove_session().unwrap();
    let dir: PathBuf = t.dir.path().to_path_buf();
    let started = Instant::now();
    loop {
        let log = std::fs::read_to_string(dir.join("claude.log")).unwrap_or_default();
        if log.lines().any(|l| l == "stop abc12345") {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(10), "{log}");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!t.f.app.background.records.contains_key(&id));
}

/// §19.11 (real-harness smoke run): an edit review a background session
/// raised shows on its own row, while the roster only says "working".
#[test]
fn an_edit_review_the_background_session_raised_shows_on_its_row() {
    let mut t = bg();
    let id = add_bg(&mut t, "");
    launched(&mut t, &id);
    let (pi, fi, si) = indices(&t.f.app, &id);
    let roster = crate::fork::claude_roster::parse(
        r#"[{"cwd":"/w","kind":"background","id":"abc12345","state":"working","status":"busy","sessionId":"conv-1"}]"#,
    );
    t.f.app.apply_claude_roster(roster);
    let feature = t.f.app.store.projects[pi].features[fi].name.clone();
    t.f.app.pending_inputs.push(crate::app::PendingInput {
        feature_name: Some(feature),
        project_name: Some(t.f.app.store.projects[pi].name.clone()),
        notification_type: "diff-review".into(),
        message: "Review: bg-iso.txt".into(),
        aiw_feature_session_id: Some(id.clone()),
        ..Default::default()
    });
    let badge = session_badge(&t.f.app, pi, fi, si, chrono::Utc::now());
    assert_eq!(badge.kind, BadgeKind::NeedsYou);
    assert!(
        badge
            .label_compact()
            .contains("diff review: Review: bg-iso.txt"),
        "{}",
        badge.label_compact()
    );
}
