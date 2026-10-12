//! FORK: feature groups, notes and layout presets at the App level
//! (DESIGN.md §19.4).

use chrono::{Duration as ChronoDuration, Utc};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::fork_fan_out::{Fixture, fixture, key, open_dialog, session_ids, step};
use crate::app::fork_fan_out::GroupMember;
use crate::app::*;
use crate::automation::CreateFeatureRequest;
use crate::fork::fan_out::FanOutLayout;
use crate::fork::organize::{FeatureSlot, NoteTarget};
use crate::project::{AgentKind, Feature, ProjectStatus, SessionKind, VibeMode};

/// Push `names` as worktree features (oldest first), each with a Claude
/// session.
fn add_features(app: &mut App, names: &[&str]) {
    let project_name = app.store.projects[0].name.clone();
    let base = Utc::now() - ChronoDuration::hours(1);
    for (i, name) in names.iter().enumerate() {
        let mut feature = Feature::new_for_project(
            &project_name,
            name.to_string(),
            name.to_string(),
            std::path::PathBuf::from(format!("/tmp/wt/{name}")),
            true,
            VibeMode::Vibeless,
            AgentKind::Claude,
            false,
        );
        feature.created_at = base + ChronoDuration::seconds(i as i64);
        feature.status = ProjectStatus::Active;
        feature.collapsed = false;
        feature.add_session(SessionKind::Claude);
        app.store.projects[0].features.push(feature);
    }
}

fn fi(app: &App, name: &str) -> usize {
    app.store.projects[0]
        .features
        .iter()
        .position(|f| f.name == name)
        .unwrap_or_else(|| panic!("no feature {name}"))
}

/// The visible rows as names: features by name, sessions as `  label`.
fn rows(app: &App) -> Vec<String> {
    app.visible_items()
        .into_iter()
        .filter_map(|item| match item {
            VisibleItem::Project(_) => None,
            VisibleItem::Feature(pi, fi) => Some(app.store.projects[pi].features[fi].name.clone()),
            VisibleItem::Session(pi, fi, si) => Some(format!(
                "{}/{}",
                app.store.projects[pi].features[fi].name,
                app.store.projects[pi].features[fi].sessions[si].label
            )),
        })
        .collect()
}

/// The selected feature row, when a feature row is selected.
fn selected_feature(app: &App) -> Option<usize> {
    match app.selection {
        Selection::Feature(0, fi) => Some(fi),
        _ => None,
    }
}

fn normal(app: &mut App, code: KeyCode) {
    crate::handlers::handle_normal_key(app, KeyEvent::new(code, KeyModifiers::NONE)).unwrap();
}

fn dialog(app: &mut App, code: KeyCode) {
    crate::handlers::handle_key(app, KeyEvent::new(code, KeyModifiers::NONE)).unwrap();
}

fn dialog_ctrl(app: &mut App, c: char) {
    crate::handlers::handle_key(app, KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
        .unwrap();
}

fn type_in(app: &mut App, text: &str) {
    for c in text.chars() {
        dialog(app, KeyCode::Char(c));
    }
}

fn groups_state(app: &App) -> &crate::app::fork_organize::FeatureGroupsState {
    match &app.mode {
        AppMode::FeatureGroups(state) => state,
        _ => panic!("the groups dialog is not open"),
    }
}

fn last_toast(app: &App) -> String {
    app.toasts
        .last()
        .map(|t| t.message.clone())
        .unwrap_or_default()
}

#[test]
fn groups_order_the_sidebar_and_a_collapsed_group_is_one_row() {
    let mut f = fixture();
    let app = &mut f.app;
    add_features(app, &["a", "b", "c", "d"]);
    let backend = app.create_feature_group(0, "Backend").unwrap();
    let frontend = app.create_feature_group(0, "Frontend").unwrap();
    app.set_feature_group(0, fi(app, "b"), Some(&backend));
    app.set_feature_group(0, fi(app, "d"), Some(&backend));
    app.set_feature_group(0, fi(app, "c"), Some(&frontend));

    assert_eq!(
        rows(app),
        [
            "a",
            "a/Claude 1",
            "b",
            "b/Claude 1",
            "d",
            "d/Claude 1",
            "c",
            "c/Claude 1"
        ]
    );
    assert_eq!(app.feature_slot(0, fi(app, "b")), FeatureSlot::GroupHead);
    assert_eq!(app.feature_slot(0, fi(app, "d")), FeatureSlot::GroupMember);

    // Collapsing from a session of a hidden member lands on the group's row.
    app.selection = Selection::Session(0, fi(app, "d"), 0);
    normal(app, KeyCode::Char('C'));
    assert_eq!(rows(app), ["a", "a/Claude 1", "b", "c", "c/Claude 1"]);
    assert_eq!(selected_feature(app), Some(fi(app, "b")));
    assert_eq!(
        app.feature_slot(0, fi(app, "b")),
        FeatureSlot::CollapsedGroup
    );
    assert_eq!(app.feature_slot(0, fi(app, "d")), FeatureSlot::Hidden);

    // j/k and the Ctrl+j feature jump walk past the collapsed group.
    app.selection = Selection::Feature(0, fi(app, "a"));
    app.select_next_feature();
    assert_eq!(selected_feature(app), Some(fi(app, "b")));
    app.select_next_feature();
    assert_eq!(selected_feature(app), Some(fi(app, "c")));
    app.select_prev();
    assert_eq!(selected_feature(app), Some(fi(app, "b")));

    // Enter on the collapsed row opens the group; h on a folded member
    // folds it again.
    normal(app, KeyCode::Enter);
    assert_eq!(rows(app).len(), 8);
    let d = fi(app, "d");
    app.store.projects[0].features[d].collapsed = true;
    app.selection = Selection::Feature(0, fi(app, "d"));
    normal(app, KeyCode::Char('h'));
    assert!(app.feature_group(0, fi(app, "d")).unwrap().collapsed);
    normal(app, KeyCode::Char('l'));
    assert!(!app.feature_group(0, fi(app, "d")).unwrap().collapsed);

    // C on an ungrouped feature says so.
    app.selection = Selection::Feature(0, fi(app, "a"));
    normal(app, KeyCode::Char('C'));
    assert!(
        app.message
            .as_deref()
            .unwrap_or("")
            .contains("not in a group")
    );

    // Everything survives a restart.
    app.toggle_feature_group(&frontend);
    let db_path = app.db.as_ref().unwrap().path.clone();
    let mut reloaded = fixture();
    reloaded.app.store = app.store.clone();
    reloaded.app.db = Some(crate::db::AiwDb::open(&db_path).unwrap());
    reloaded.app.load_organize_state();
    assert_eq!(rows(&reloaded.app), rows(app));
    assert_eq!(
        reloaded
            .app
            .project_groups(0)
            .iter()
            .map(|g| (g.name.clone(), g.collapsed))
            .collect::<Vec<_>>(),
        vec![
            ("Backend".to_string(), false),
            ("Frontend".to_string(), true)
        ]
    );
}

#[test]
fn the_groups_dialog_moves_creates_renames_recolours_orders_and_deletes() {
    let mut f = fixture();
    let app = &mut f.app;
    add_features(app, &["a", "b"]);
    app.selection = Selection::Feature(0, fi(app, "a"));

    // No groups yet: (no group) and "+ New group…".
    normal(app, KeyCode::Char('G'));
    assert_eq!(app.group_dialog_rows(groups_state(app)).len(), 2);
    dialog(app, KeyCode::Char('j'));
    dialog(app, KeyCode::Enter); // New group…
    type_in(app, "Backend");
    dialog(app, KeyCode::Enter);
    assert!(matches!(app.mode, AppMode::Normal));
    assert_eq!(app.feature_group(0, fi(app, "a")).unwrap().name, "Backend");
    assert_eq!(
        app.message.as_deref(),
        Some("Moved 'a' to new group Backend")
    );

    // A duplicate name is refused in place; then move b into the group.
    app.selection = Selection::Session(0, fi(app, "b"), 0);
    normal(app, KeyCode::Char('G'));
    assert_eq!(groups_state(app).selected, 0, "b is not in a group yet");
    dialog(app, KeyCode::Char('n'));
    type_in(app, "backend");
    dialog(app, KeyCode::Enter);
    assert!(
        groups_state(app)
            .error
            .as_deref()
            .unwrap()
            .contains("already exists")
    );
    dialog(app, KeyCode::Esc); // leave the name field
    dialog(app, KeyCode::Char('j'));
    dialog(app, KeyCode::Enter);
    assert_eq!(app.feature_group(0, fi(app, "b")).unwrap().name, "Backend");
    assert_eq!(selected_feature(app), Some(fi(app, "b")));

    // Rename, recolour, a second group, reorder.
    normal(app, KeyCode::Char('G'));
    assert_eq!(groups_state(app).selected, 1, "opens on b's group");
    dialog(app, KeyCode::Char('r'));
    dialog_ctrl(app, 'u');
    type_in(app, "API");
    dialog(app, KeyCode::Enter);
    let group = app.project_groups(0)[0].clone();
    assert_eq!(group.name, "API");
    assert_eq!(group.color, "cyan");
    dialog(app, KeyCode::Char('c'));
    assert_eq!(app.project_groups(0)[0].color, "green");
    dialog(app, KeyCode::Char('n'));
    type_in(app, "UI");
    dialog(app, KeyCode::Enter);
    // With a feature selected, a new group takes it at once.
    assert!(matches!(app.mode, AppMode::Normal));
    assert_eq!(app.feature_group(0, fi(app, "b")).unwrap().name, "UI");
    assert_eq!(
        app.project_groups(0)
            .iter()
            .map(|g| g.name.as_str())
            .collect::<Vec<_>>(),
        ["API", "UI"]
    );
    assert_eq!(app.project_groups(0)[1].color, "cyan", "an unused colour");
    normal(app, KeyCode::Char('G'));
    assert_eq!(groups_state(app).selected, 2);
    dialog(app, KeyCode::Char('K')); // UI up
    assert_eq!(
        app.project_groups(0)
            .iter()
            .map(|g| g.name.as_str())
            .collect::<Vec<_>>(),
        ["UI", "API"]
    );
    assert_eq!(
        groups_state(app).selected,
        1,
        "the moved group stays selected"
    );
    assert_eq!(rows(app)[0], "b", "UI's feature now comes first");

    // Delete asks first; its features become ungrouped.
    dialog(app, KeyCode::Char('d'));
    assert!(groups_state(app).confirm_delete.is_some());
    dialog(app, KeyCode::Char('n'));
    assert_eq!(app.project_groups(0).len(), 2, "n keeps it");
    dialog(app, KeyCode::Char('d'));
    dialog(app, KeyCode::Char('y'));
    assert_eq!(app.project_groups(0).len(), 1);
    assert!(app.feature_group(0, fi(app, "b")).is_none());
    // Back to (no group) for a.
    app.mode = AppMode::Normal;
    app.selection = Selection::Feature(0, fi(app, "a"));
    normal(app, KeyCode::Char('G'));
    assert_eq!(groups_state(app).selected, 1, "opens on a's group");
    dialog(app, KeyCode::Char('k'));
    assert_eq!(groups_state(app).selected, 0);
    dialog(app, KeyCode::Enter);
    assert!(app.feature_group(0, fi(app, "a")).is_none());

    // From a project row, Enter cannot move anything.
    app.selection = Selection::Project(0);
    normal(app, KeyCode::Char('G'));
    assert_eq!(groups_state(app).selected, 1);
    dialog(app, KeyCode::Enter);
    assert!(groups_state(app).error.is_some());
    dialog(app, KeyCode::Esc);
    assert!(matches!(app.mode, AppMode::Normal));
}

#[test]
fn a_fan_out_pair_can_sit_inside_a_user_group_and_keeps_its_header() {
    let mut f = fixture();
    let app = &mut f.app;
    add_features(app, &["solo", "fix-claude", "fix-codex"]);
    for (i, name) in ["fix-claude", "fix-codex"].iter().enumerate() {
        let id = app.store.projects[0].features[fi(app, name)].id.clone();
        app.fan_out.members.insert(
            id,
            GroupMember {
                group_id: "fan-1".into(),
                base_branch: "fix".into(),
                agent: if i == 0 {
                    AgentKind::Claude
                } else {
                    AgentKind::Codex
                },
                position: i as i64,
            },
        );
    }
    app.viewport_cols = 40;
    let experiments = app.create_feature_group(0, "Experiments").unwrap();
    app.set_feature_group(0, fi(app, "fix-claude"), Some(&experiments));
    app.set_feature_group(0, fi(app, "fix-codex"), Some(&experiments));
    let head = fi(app, "fix-claude");
    assert!(app.fan_out_heads_group(0, head));
    assert_eq!(app.feature_slot(0, head), FeatureSlot::GroupHead);
    // Group header + fan-out header + the row itself.
    assert_eq!(app.workspace_item_height(&VisibleItem::Feature(0, head)), 3);

    // One sibling outside the group: the header follows the one drawn first.
    app.set_feature_group(0, fi(app, "fix-claude"), None);
    assert!(app.fan_out_heads_group(0, fi(app, "fix-claude")));
    assert!(!app.fan_out_heads_group(0, fi(app, "fix-codex")));
    app.set_feature_group(0, fi(app, "fix-claude"), Some(&experiments));
    app.set_feature_group(0, fi(app, "fix-codex"), None);
    assert!(app.fan_out_heads_group(0, fi(app, "fix-codex")));
    // A collapsed group's row stands for the group, not for the sibling.
    app.toggle_feature_group(&experiments);
    assert_eq!(
        app.feature_slot(0, fi(app, "fix-claude")),
        FeatureSlot::CollapsedGroup
    );
    assert_eq!(
        app.workspace_item_height(&VisibleItem::Feature(0, fi(app, "fix-claude"))),
        1
    );
    assert!(app.fan_out_heads_group(0, fi(app, "fix-codex")));

    // The sidebar draws both kinds of header without the old tree layout.
    app.toggle_feature_group(&experiments);
    app.set_feature_group(0, fi(app, "fix-codex"), Some(&experiments));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(40, 30)).unwrap();
    terminal.draw(|frame| crate::ui::draw(frame, app)).unwrap();
    let screen: String = {
        let buffer = terminal.backend().buffer();
        (0..30)
            .map(|y| {
                (0..40)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert!(screen.contains("■ Experiments"), "{screen}");
    assert!(screen.contains("⇶ fix"), "{screen}");
    let group_line = screen
        .lines()
        .position(|l| l.contains("■ Experiments"))
        .unwrap();
    let fan_line = screen.lines().position(|l| l.contains("⇶ fix")).unwrap();
    assert!(
        group_line < fan_line,
        "the group header comes first\n{screen}"
    );
}

#[test]
fn notes_persist_show_in_the_info_block_and_match_search() {
    let mut f = fixture();
    let app = &mut f.app;
    add_features(app, &["checkout", "billing"]);
    let billing = fi(app, "billing");

    // A session note through `#`.
    app.selection = Selection::Session(0, billing, 0);
    normal(app, KeyCode::Char('#'));
    type_in(app, "Waiting on Stripe");
    crate::handlers::handle_key(app, KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT)).unwrap();
    type_in(app, "webhook secret rotated");
    dialog(app, KeyCode::Enter);
    let session_id = app.store.projects[0].features[billing].sessions[0]
        .id
        .clone();
    assert_eq!(
        app.session_note(&session_id),
        Some("Waiting on Stripe\nwebhook secret rotated")
    );
    let info = app.session_info_for(0, billing, 0, Utc::now()).unwrap();
    assert_eq!(info.note.as_deref(), Some("Waiting on Stripe"));
    assert!(app.session_info_has_note((0, billing, 0)));

    // A feature note; the info block falls back to it.
    let checkout = fi(app, "checkout");
    app.selection = Selection::Feature(0, checkout);
    normal(app, KeyCode::Char('#'));
    type_in(app, "Blocked by the design review");
    dialog(app, KeyCode::Enter);
    let info = app.session_info_for(0, checkout, 0, Utc::now()).unwrap();
    assert_eq!(info.note.as_deref(), Some("Blocked by the design review"));

    // `/` finds both by note text, and shows the matching line.
    app.start_search();
    if let AppMode::Searching(state) = &mut app.mode {
        state.query = "webhook".into();
    }
    app.perform_search();
    let AppMode::Searching(state) = &app.mode else {
        panic!("search closed");
    };
    assert_eq!(state.matches.len(), 1);
    assert!(matches!(state.matches[0].item, VisibleItem::Session(0, f, 0) if f == billing));
    assert!(
        state.matches[0]
            .context
            .contains("✎ webhook secret rotated")
    );
    if let AppMode::Searching(state) = &mut app.mode {
        state.query = "design".into();
    }
    app.perform_search();
    let AppMode::Searching(state) = &app.mode else {
        panic!("search closed");
    };
    assert!(matches!(state.matches[0].item, VisibleItem::Feature(0, f) if f == checkout));

    // A jump into a collapsed group opens it.
    let group = app.create_feature_group(0, "Payments").unwrap();
    app.set_feature_group(0, checkout, Some(&group));
    app.toggle_feature_group(&group);
    assert!(app.feature_group(0, checkout).unwrap().collapsed);
    app.jump_to_search_match();
    assert!(!app.feature_group(0, checkout).unwrap().collapsed);
    assert_eq!(selected_feature(app), Some(checkout));
    // The group name is searchable too.
    app.start_search();
    if let AppMode::Searching(state) = &mut app.mode {
        state.query = "payments".into();
    }
    app.perform_search();
    let AppMode::Searching(state) = &app.mode else {
        panic!("search closed");
    };
    assert!(
        state
            .matches
            .iter()
            .any(|m| m.context.contains("group Payments"))
    );
    app.cancel_search();

    // Survives a restart; an empty save deletes; deleting the feature drops it.
    let db_path = app.db.as_ref().unwrap().path.clone();
    let reopened = crate::db::AiwDb::open(&db_path).unwrap();
    assert_eq!(reopened.all_notes().unwrap().len(), 2);
    app.selection = Selection::Feature(0, checkout);
    normal(app, KeyCode::Char('#'));
    dialog_ctrl(app, 'u');
    dialog(app, KeyCode::Enter);
    assert_eq!(
        app.feature_note(&app.store.projects[0].features[checkout].id.clone()),
        None
    );
    assert!(
        app.message
            .as_deref()
            .unwrap()
            .starts_with("Deleted the note")
    );
    let feature = app.store.projects[0].features[billing].clone();
    app.forget_organize_feature(&feature);
    assert_eq!(app.session_note(&session_id), None);
    assert!(reopened.all_notes().unwrap().is_empty());

    // Esc leaves a note untouched; a project row has no note.
    app.selection = Selection::Project(0);
    normal(app, KeyCode::Char('#'));
    assert!(matches!(app.mode, AppMode::Normal));
    assert!(
        app.message
            .as_deref()
            .unwrap()
            .contains("feature or a session")
    );
}

/// Write `aiw.json` into the fixture's repo.
fn write_project_config(f: &Fixture, json: serde_json::Value) {
    let repo = f.app.store.projects[0].repo.clone();
    std::fs::write(
        repo.join("aiw.json"),
        serde_json::to_string_pretty(&json).unwrap(),
    )
    .unwrap();
}

fn layout_config(custom: &str) -> serde_json::Value {
    serde_json::json!({
        "custom_sessions": [ { "name": "server", "command": "npm run dev" } ],
        "layout_presets": [ {
            "name": "full",
            "sessions": [
                { "kind": "terminal", "label": "watch", "command": "npm test -- --watch" },
                { "kind": "custom", "name": custom },
                { "kind": "agent", "agent": "codex" }
            ]
        } ],
        "feature_presets": [
            { "name": "with-layout", "agent": "claude", "layout": "full" },
            { "name": "bad-layout", "agent": "claude", "layout": "nope" }
        ]
    })
}

/// Through the Mode step to the Layout row, pick `full`, then create.
fn create_with_layout(app: &mut App, branch: &str) {
    open_dialog(app, branch, true);
    let AppMode::CreatingFeature(state) = &app.mode else {
        panic!("dialog closed");
    };
    assert_eq!(state.layout_presets, ["full"]);
    assert_eq!(state.layout_focus(), Some(3));
    for _ in 0..3 {
        key(app, KeyCode::Enter); // harness → mode → chrome → layout
    }
    key(app, KeyCode::Char('j'));
    let AppMode::CreatingFeature(state) = &app.mode else {
        panic!("dialog closed");
    };
    assert_eq!(state.selected_layout().as_deref(), Some("full"));
    key(app, KeyCode::Enter); // → session name
    assert_eq!(step(app), CreateFeatureStep::SessionName);
    key(app, KeyCode::Enter); // → prompt
    key(app, KeyCode::Enter); // create
}

#[test]
fn a_layout_preset_opens_its_sessions_once_the_feature_runs() {
    let mut f = fixture();
    write_project_config(&f, layout_config("server"));
    create_with_layout(&mut f.app, "dash");
    assert!(matches!(f.app.mode, AppMode::Normal), "created");
    let sessions = session_ids(&f.app, "dash");
    let labels: Vec<(String, SessionKind)> = sessions
        .iter()
        .map(|(l, k, _)| (l.clone(), k.clone()))
        .collect();
    assert_eq!(
        labels,
        vec![
            ("Claude 1".to_string(), SessionKind::Claude),
            ("watch".to_string(), SessionKind::Terminal),
            ("server".to_string(), SessionKind::Custom),
            ("Codex 1".to_string(), SessionKind::Codex),
        ]
    );
    let calls = f.calls.lock().unwrap();
    assert!(
        calls
            .shell
            .iter()
            .any(|(_, _, cmd)| cmd == "npm test -- --watch"),
        "the terminal runs its command: {:?}",
        calls.shell
    );
    assert!(calls.launches.iter().any(|(h, _, _)| h == "codex"));
    drop(calls);
    // The selection stays on the feature's own agent, not on the last
    // window the layout opened.
    match f.app.selection {
        Selection::Session(_, fi, si) => {
            assert_eq!(
                f.app.store.projects[0].features[fi].sessions[si].label,
                "Claude 1"
            )
        }
        Selection::Feature(0, fi) => assert_eq!(f.app.store.projects[0].features[fi].name, "dash"),
        ref other => panic!("selection moved: {other:?}"),
    }
    assert!(last_toast(&f.app).contains("Layout 'full': opened 3 sessions"));
    assert!(f.app.organize.pending_layout.is_none());
}

#[test]
fn a_bad_layout_stops_the_dialog_before_anything_is_created() {
    let mut f = fixture();
    write_project_config(&f, layout_config("missing"));
    create_with_layout(&mut f.app, "dash");
    assert!(
        matches!(f.app.mode, AppMode::CreatingFeature(_)),
        "still open"
    );
    assert!(f.app.store.projects[0].features.is_empty());
    assert_eq!(
        last_toast(&f.app),
        "layout 'full', session 2: custom session 'missing' is not in custom_sessions (known: server)"
    );
    assert!(f.app.organize.pending_layout.is_none());
}

#[test]
fn a_feature_preset_chooses_its_layout_and_a_missing_one_is_reported() {
    let mut f = fixture();
    write_project_config(&f, layout_config("server"));
    f.app.selection = Selection::Project(0);
    f.app.start_create_feature();
    let pick = |app: &mut App, index: usize| {
        if let AppMode::CreatingFeature(state) = &mut app.mode {
            state.step = CreateFeatureStep::SelectPreset;
            state.preset_index = index;
            state.layout_index = 0;
        }
        key(app, KeyCode::Enter);
    };
    pick(&mut f.app, 0);
    let AppMode::CreatingFeature(state) = &f.app.mode else {
        panic!("dialog closed");
    };
    assert_eq!(state.selected_layout().as_deref(), Some("full"));
    pick(&mut f.app, 1);
    let AppMode::CreatingFeature(state) = &f.app.mode else {
        panic!("dialog closed");
    };
    assert_eq!(state.selected_layout(), None);
    assert!(last_toast(&f.app).contains("names layout 'nope'"));
}

#[test]
fn automation_opens_the_layout_in_every_sibling_and_refuses_an_unknown_one() {
    let mut f = fixture();
    write_project_config(
        &f,
        serde_json::json!({
            "layout_presets": [ { "name": "term", "sessions": [ { "kind": "terminal" } ] } ]
        }),
    );
    let request = CreateFeatureRequest {
        project_name: f.app.store.projects[0].name.clone(),
        branch: "auto".into(),
        agents: vec![AgentKind::Claude, AgentKind::Codex],
        fan_out: Some(FanOutLayout::WorktreeEach),
        use_worktree: Some(true),
        mode: VibeMode::Vibe,
        layout: Some("term".into()),
        ..Default::default()
    };
    let response = f
        .app
        .create_feature_from_request_organized(&request)
        .unwrap();
    assert!(
        response
            .message
            .contains("layout 'term' opened 2 session(s)"),
        "{}",
        response.message
    );
    for name in ["auto-claude", "auto-codex"] {
        let kinds: Vec<SessionKind> = session_ids(&f.app, name)
            .into_iter()
            .map(|(_, k, _)| k)
            .collect();
        assert!(kinds.contains(&SessionKind::Terminal), "{name}: {kinds:?}");
    }
    let before = f.app.store.projects[0].features.len();
    let bad = CreateFeatureRequest {
        branch: "auto2".into(),
        layout: Some("nope".into()),
        ..request
    };
    let err = f
        .app
        .create_feature_from_request_organized(&bad)
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("layout 'nope' is not defined (known: term)")
    );
    assert_eq!(
        f.app.store.projects[0].features.len(),
        before,
        "nothing created"
    );
}

#[test]
fn deleting_a_project_drops_its_groups_and_notes() {
    let mut f = fixture();
    let app = &mut f.app;
    add_features(app, &["a"]);
    let group = app.create_feature_group(0, "G").unwrap();
    app.set_feature_group(0, 0, Some(&group));
    let feature_id = app.store.projects[0].features[0].id.clone();
    app.set_note(NoteTarget::Feature(feature_id.clone()), "hello");
    let project = app.store.projects[0].clone();
    app.forget_organize_project(&project);
    assert!(app.organize.groups.is_empty());
    assert!(app.organize.members.is_empty());
    assert_eq!(app.feature_note(&feature_id), None);
    let db = app.db.as_ref().unwrap();
    assert!(db.all_feature_groups().unwrap().is_empty());
    assert!(db.all_notes().unwrap().is_empty());
}
