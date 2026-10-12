use crate::app::*;
use crate::project::AgentKind;
use crate::traits::{MockTmuxOps, MockWorktreeOps};
use crossterm::event::KeyCode;
use std::collections::HashMap;
use tempfile::NamedTempFile;

// ── AppConfig defaults ───────────────────────────────────

#[test]
fn app_config_default_leader_timeout_is_five_seconds() {
    let config = AppConfig::default();
    assert_eq!(config.leader_timeout_seconds, 5);
}

#[test]
fn app_config_default_tmux_control_mode_is_disabled() {
    // Direct transport is the default; control mode is opt-in.
    let config = AppConfig::default();
    assert!(!config.tmux_control_mode);
}

#[test]
fn app_config_default_agent_restart_limit_is_one() {
    let config = AppConfig::default();
    assert_eq!(config.max_agent_autostart_sessions, 1);
}

#[test]
fn app_config_resource_guard_defaults() {
    let config = AppConfig::default();
    assert_eq!(config.max_concurrent_agents, 4);
    assert_eq!(config.low_memory_warn_mb, 1536);
    assert!(config.kill_editor_on_stop);
    assert_eq!(config.dormant_idle_minutes, 60);
    assert_eq!(config.dormant_last_accessed_hours, 4);
}

#[test]
fn app_config_missing_resource_guard_keys_use_defaults() {
    // Configs written before these keys existed must keep loading.
    let config: AppConfig = serde_json::from_str(r#"{"nerd_font":false}"#).unwrap();
    assert_eq!(config.agent_concurrency_limit(), Some(4));
    assert_eq!(config.low_memory_threshold_mb(), Some(1536));
    assert!(config.kill_editor_on_stop);
    assert_eq!(
        config.dormant_thresholds(),
        Some((
            std::time::Duration::from_secs(60 * 60),
            std::time::Duration::from_secs(4 * 3600)
        ))
    );
}

#[test]
fn app_config_resource_guards_can_be_configured() {
    let config: AppConfig = serde_json::from_str(
        r#"{"max_concurrent_agents":8,"low_memory_warn_mb":2048,"kill_editor_on_stop":false,
            "dormant_idle_minutes":15,"dormant_last_accessed_hours":24}"#,
    )
    .unwrap();
    assert_eq!(config.agent_concurrency_limit(), Some(8));
    assert_eq!(config.low_memory_threshold_mb(), Some(2048));
    assert!(!config.kill_editor_on_stop);
    assert_eq!(
        config.dormant_thresholds(),
        Some((
            std::time::Duration::from_secs(15 * 60),
            std::time::Duration::from_secs(24 * 3600)
        ))
    );
}

#[test]
fn app_config_zero_disables_each_resource_guard() {
    let config: AppConfig =
        serde_json::from_str(r#"{"max_concurrent_agents":0,"low_memory_warn_mb":0}"#).unwrap();
    assert_eq!(config.agent_concurrency_limit(), None);
    assert_eq!(config.low_memory_threshold_mb(), None);

    // Dormancy is an AND of both halves, so zeroing either one turns the
    // whole check off instead of marking every feature dormant.
    let idle_off: AppConfig = serde_json::from_str(r#"{"dormant_idle_minutes":0}"#).unwrap();
    assert_eq!(idle_off.dormant_thresholds(), None);
    let accessed_off: AppConfig =
        serde_json::from_str(r#"{"dormant_last_accessed_hours":0}"#).unwrap();
    assert_eq!(accessed_off.dormant_thresholds(), None);
}

#[test]
fn app_config_waiting_stale_minutes_defaults_to_thirty() {
    assert_eq!(AppConfig::default().waiting_stale_minutes, 30);
    assert_eq!(
        AppConfig::default().waiting_stale_threshold(),
        Some(std::time::Duration::from_secs(30 * 60))
    );
}

#[test]
fn app_config_missing_waiting_stale_minutes_uses_default() {
    // Configs written before the key existed must keep loading.
    let config: AppConfig = serde_json::from_str(r#"{"nerd_font":false}"#).unwrap();
    assert_eq!(
        config.waiting_stale_threshold(),
        Some(std::time::Duration::from_secs(30 * 60))
    );
}

#[test]
fn app_config_waiting_stale_minutes_is_configurable_and_zero_disables() {
    let config: AppConfig = serde_json::from_str(r#"{"waiting_stale_minutes":5}"#).unwrap();
    assert_eq!(
        config.waiting_stale_threshold(),
        Some(std::time::Duration::from_secs(5 * 60))
    );

    let disabled: AppConfig = serde_json::from_str(r#"{"waiting_stale_minutes":0}"#).unwrap();
    assert_eq!(disabled.waiting_stale_threshold(), None);
}

#[test]
fn app_config_context_defaults_match_the_hardcoded_fallbacks() {
    let config = AppConfig::default();
    assert_eq!(config.context_window_override, None);
    assert_eq!(
        config.context_warning_percent,
        crate::context_tracking::DEFAULT_CONTEXT_WARNING_PERCENT
    );
    assert_eq!(
        config.context_critical_percent,
        crate::context_tracking::DEFAULT_CONTEXT_CRITICAL_PERCENT
    );
}

#[test]
fn app_config_missing_context_keys_use_defaults() {
    // Configs written before these keys existed must keep loading.
    let config: AppConfig = serde_json::from_str(r#"{"nerd_font":false}"#).unwrap();
    assert_eq!(config.context_window_override, None);
    assert_eq!(config.context_warning_percent, 70);
    assert_eq!(config.context_critical_percent, 85);
}

#[test]
fn app_config_context_values_are_configurable() {
    let config: AppConfig = serde_json::from_str(
        r#"{"context_window_override":500000,"context_warning_percent":50,"context_critical_percent":75}"#,
    )
    .unwrap();
    assert_eq!(config.context_window_override, Some(500_000));
    assert_eq!(config.context_warning_percent, 50);
    assert_eq!(config.context_critical_percent, 75);
}

#[test]
fn app_config_default_diff_viewer_layout_is_unified() {
    let config = AppConfig::default();
    assert_eq!(config.diff_viewer_layout, DiffViewerLayout::Unified);
}

#[test]
fn app_config_missing_leader_timeout_uses_default() {
    let config: AppConfig = serde_json::from_str(r#"{"nerd_font":false}"#).unwrap();
    assert_eq!(config.leader_timeout_seconds, 5);
    assert!(!config.nerd_font);
}

#[test]
fn app_config_missing_tmux_control_mode_uses_default() {
    let config: AppConfig = serde_json::from_str(r#"{"nerd_font":false}"#).unwrap();
    assert!(!config.tmux_control_mode);
}

#[test]
fn app_config_tmux_control_mode_can_be_disabled() {
    let config: AppConfig = serde_json::from_str(r#"{"tmux_control_mode":false}"#).unwrap();
    assert!(!config.tmux_control_mode);
}

#[test]
fn app_config_tmux_control_mode_can_be_enabled() {
    let config: AppConfig = serde_json::from_str(r#"{"tmux_control_mode":true}"#).unwrap();
    assert!(config.tmux_control_mode);
}

#[test]
fn app_config_agent_restart_limit_can_be_configured() {
    let config: AppConfig = serde_json::from_str(r#"{"max_agent_autostart_sessions":0}"#).unwrap();
    assert_eq!(config.max_agent_autostart_sessions, 0);
}

#[test]
fn migrate_app_config_flips_pre_v1_defaults() {
    // A config written before versioning: control mode on, old popup hold.
    let mut config: AppConfig =
        serde_json::from_str(r#"{"tmux_control_mode":true,"diff_review_popup_hold_secs":3.0}"#)
            .unwrap();
    assert_eq!(config.config_version, 0);

    let changed = crate::app::setup::migrate_app_config(&mut config);

    assert!(changed);
    assert!(!config.tmux_control_mode);
    assert_eq!(config.diff_review_popup_hold_secs, 1.5);
    assert_eq!(config.config_version, crate::app::APP_CONFIG_VERSION);
}

#[test]
fn migrate_app_config_preserves_deliberate_pre_v1_values() {
    // Pre-v1 config that already chose non-default values: only the
    // version is stamped, the user's choices are kept.
    let mut config: AppConfig =
        serde_json::from_str(r#"{"tmux_control_mode":false,"diff_review_popup_hold_secs":5.0}"#)
            .unwrap();

    let changed = crate::app::setup::migrate_app_config(&mut config);

    assert!(changed);
    assert!(!config.tmux_control_mode);
    assert_eq!(config.diff_review_popup_hold_secs, 5.0);
    assert_eq!(config.config_version, crate::app::APP_CONFIG_VERSION);
}

#[test]
fn migrate_app_config_is_noop_when_current() {
    // An already-current config (e.g. someone who re-enabled control mode
    // after migrating) is left untouched and triggers no rewrite.
    let mut config = AppConfig {
        config_version: crate::app::APP_CONFIG_VERSION,
        tmux_control_mode: true,
        diff_review_popup_hold_secs: 3.0,
        ..AppConfig::default()
    };

    let changed = crate::app::setup::migrate_app_config(&mut config);

    assert!(!changed);
    assert!(config.tmux_control_mode);
    assert_eq!(config.diff_review_popup_hold_secs, 3.0);
}

#[test]
fn app_config_missing_diff_viewer_layout_uses_unified_default() {
    let config: AppConfig = serde_json::from_str(r#"{"nerd_font":false}"#).unwrap();
    assert_eq!(config.diff_viewer_layout, DiffViewerLayout::Unified);
}

#[test]
fn app_config_missing_projects_uses_default_preferred_agent_none() {
    let config: AppConfig = serde_json::from_str(r#"{"nerd_font":false}"#).unwrap();
    assert_eq!(config.projects.default_preferred_agent, None);
}

#[test]
fn app_config_projects_default_preferred_agent_deserializes() {
    let config: AppConfig =
        serde_json::from_str(r#"{"projects":{"default_preferred_agent":"codex"}}"#).unwrap();
    assert_eq!(
        config.projects.default_preferred_agent,
        Some(AgentKind::Codex)
    );
}

#[test]
fn default_project_preferred_agent_comes_from_config() {
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
    app.config.projects.default_preferred_agent = Some(AgentKind::Pi);

    assert_eq!(app.default_project_preferred_agent(), AgentKind::Pi);
}

/// FORK: a `config.json` from the OpenCode era still loads: the old
/// `opencode_theme` key is ignored, OpenCode preferences fall back to the
/// default agent, and `allowed_agents` drops OpenCode.
#[test]
fn legacy_opencode_config_json_still_loads() {
    let config: AppConfig = serde_json::from_str(
        r#"{
            "opencode_theme": "catppuccin-frappe",
            "projects": {"default_preferred_agent": "opencode"},
            "extension": {
                "allowed_agents": ["opencode", "codex"],
                "feature_presets": [{"name": "oc", "agent": "opencode"}]
            }
        }"#,
    )
    .expect("legacy config must load");
    assert_eq!(
        config.projects.default_preferred_agent,
        Some(AgentKind::default())
    );
    assert_eq!(
        config.extension.allowed_agents,
        Some(vec![AgentKind::Codex])
    );
    assert_eq!(
        config.extension.feature_presets[0].agent,
        AgentKind::default()
    );
}

// ── ContextSettings dialog ─────────────────────────────────

fn context_settings_test_app() -> App {
    App::new_for_test(
        ProjectStore {
            version: 5,
            projects: vec![],
            session_bookmarks: vec![],
            available_harnesses: vec![],
            prompt_templates: Vec::new(),
            extra: HashMap::new(),
        },
        Box::new(MockTmuxOps::new()),
        Box::new(MockWorktreeOps::new()),
    )
}

#[test]
fn context_settings_opens_prefilled_with_current_config() {
    let mut app = context_settings_test_app();
    app.config.context_window_override = Some(500_000);
    app.config.context_warning_percent = 60;
    app.config.context_critical_percent = 90;

    app.start_context_settings();

    let AppMode::ContextSettings(state) = &app.mode else {
        panic!("expected ContextSettings mode");
    };
    assert_eq!(state.window_limit_input, "500000");
    assert_eq!(state.warning_input, "60");
    assert_eq!(state.critical_input, "90");
    assert_eq!(state.field, crate::app::ContextSettingsField::WindowLimit);
}

#[test]
fn context_settings_confirm_persists_valid_values() {
    let mut app = context_settings_test_app();
    app.start_context_settings();

    let AppMode::ContextSettings(state) = &mut app.mode else {
        panic!("expected ContextSettings mode");
    };
    state.window_limit_input = "500000".to_string();
    state.warning_input = "50".to_string();
    state.critical_input = "80".to_string();

    assert!(app.context_settings_confirm());
    assert!(matches!(app.mode, AppMode::Normal));
    assert_eq!(app.config.context_window_override, Some(500_000));
    assert_eq!(app.config.context_warning_percent, 50);
    assert_eq!(app.config.context_critical_percent, 80);
}

#[test]
fn context_settings_confirm_blank_window_limit_clears_the_override() {
    let mut app = context_settings_test_app();
    app.config.context_window_override = Some(500_000);
    app.start_context_settings();

    let AppMode::ContextSettings(state) = &mut app.mode else {
        panic!("expected ContextSettings mode");
    };
    state.window_limit_input.clear();

    assert!(app.context_settings_confirm());
    assert_eq!(app.config.context_window_override, None);
}

#[test]
fn context_settings_confirm_rejects_critical_at_or_below_warning() {
    let mut app = context_settings_test_app();
    app.start_context_settings();

    let AppMode::ContextSettings(state) = &mut app.mode else {
        panic!("expected ContextSettings mode");
    };
    state.warning_input = "80".to_string();
    state.critical_input = "80".to_string();

    assert!(!app.context_settings_confirm());
    let AppMode::ContextSettings(state) = &app.mode else {
        panic!("dialog should stay open after a rejected confirm");
    };
    assert!(state.error.is_some());
    // Rejected values must not leak into the live config.
    assert_eq!(app.config.context_warning_percent, 70);
}

#[test]
fn context_settings_confirm_rejects_out_of_range_percentages() {
    let mut app = context_settings_test_app();
    app.start_context_settings();

    let AppMode::ContextSettings(state) = &mut app.mode else {
        panic!("expected ContextSettings mode");
    };
    state.critical_input = "150".to_string();

    assert!(!app.context_settings_confirm());
    assert!(matches!(&app.mode, AppMode::ContextSettings(s) if s.error.is_some()));
}

#[test]
fn context_settings_confirm_rejects_zero_window_limit() {
    let mut app = context_settings_test_app();
    app.start_context_settings();

    let AppMode::ContextSettings(state) = &mut app.mode else {
        panic!("expected ContextSettings mode");
    };
    state.window_limit_input = "0".to_string();

    assert!(!app.context_settings_confirm());
    assert!(matches!(&app.mode, AppMode::ContextSettings(s) if s.error.is_some()));
}

#[test]
fn context_settings_cancel_discards_changes() {
    let mut app = context_settings_test_app();
    app.start_context_settings();

    let AppMode::ContextSettings(state) = &mut app.mode else {
        panic!("expected ContextSettings mode");
    };
    state.warning_input = "10".to_string();

    app.cancel_context_settings();

    assert!(matches!(app.mode, AppMode::Normal));
    assert_eq!(app.config.context_warning_percent, 70);
}

#[test]
fn context_settings_focus_cycles_through_all_fields_and_wraps() {
    use crate::app::ContextSettingsField;

    let mut app = context_settings_test_app();
    app.start_context_settings();

    app.context_settings_focus_next();
    assert!(matches!(
        &app.mode,
        AppMode::ContextSettings(s) if s.field == ContextSettingsField::WarningPercent
    ));
    app.context_settings_focus_next();
    assert!(matches!(
        &app.mode,
        AppMode::ContextSettings(s) if s.field == ContextSettingsField::CriticalPercent
    ));
    app.context_settings_focus_next();
    assert!(matches!(
        &app.mode,
        AppMode::ContextSettings(s) if s.field == ContextSettingsField::WindowLimit
    ));
    app.context_settings_focus_prev();
    assert!(matches!(
        &app.mode,
        AppMode::ContextSettings(s) if s.field == ContextSettingsField::CriticalPercent
    ));
}

// ── ZaiPlanConfig::get_monthly_limit ─────────────────────

#[test]
fn zai_free_plan_monthly_limit() {
    let config = ZaiPlanConfig {
        plan: "free".to_string(),
        ..Default::default()
    };
    assert_eq!(config.get_monthly_limit(), Some(10_000_000));
}

#[test]
fn zai_coding_plan_monthly_limit() {
    let config = ZaiPlanConfig {
        plan: "coding-plan".to_string(),
        ..Default::default()
    };
    assert_eq!(config.get_monthly_limit(), Some(500_000_000));
}

#[test]
fn zai_unlimited_plan_monthly_limit_is_none() {
    let config = ZaiPlanConfig {
        plan: "unlimited".to_string(),
        ..Default::default()
    };
    assert_eq!(config.get_monthly_limit(), None);
}

#[test]
fn zai_custom_plan_monthly_limit_is_none() {
    let config = ZaiPlanConfig {
        plan: "enterprise".to_string(),
        ..Default::default()
    };
    assert_eq!(config.get_monthly_limit(), None);
}

#[test]
fn zai_explicit_token_limit_overrides_plan() {
    let config = ZaiPlanConfig {
        plan: "free".to_string(),
        monthly_token_limit: Some(999),
        ..Default::default()
    };
    assert_eq!(config.get_monthly_limit(), Some(999));
}

// ── Prompt library picker ─────────────────────────────────────

fn store_with_prompt_templates(names: &[&str]) -> ProjectStore {
    let templates = names
        .iter()
        .map(|name| {
            crate::prompt_library::PromptTemplate::new(name.to_string(), format!("body of {name}"))
        })
        .collect();
    ProjectStore {
        version: 5,
        projects: vec![],
        session_bookmarks: vec![],
        available_harnesses: vec![],
        prompt_templates: templates,
        extra: HashMap::new(),
    }
}

#[test]
fn prompt_library_opens_with_all_templates_visible() {
    let store = store_with_prompt_templates(&["alpha", "beta", "gamma"]);
    let mut app = App::new_for_test(
        store,
        Box::new(MockTmuxOps::new()),
        Box::new(MockWorktreeOps::new()),
    );

    app.open_prompt_library(None);

    match &app.mode {
        AppMode::PromptLibrary(state) => {
            assert_eq!(state.templates.len(), 3);
            assert_eq!(state.filtered.len(), 3);
            assert_eq!(state.selected, 0);
        }
        _ => panic!("expected PromptLibrary mode"),
    }
}

#[test]
fn prompt_library_nav_wraps_around() {
    let store = store_with_prompt_templates(&["a", "b", "c"]);
    let mut app = App::new_for_test(
        store,
        Box::new(MockTmuxOps::new()),
        Box::new(MockWorktreeOps::new()),
    );
    app.open_prompt_library(None);

    app.prompt_library_select_prev(); // wraps from 0 to last
    match &app.mode {
        AppMode::PromptLibrary(state) => assert_eq!(state.selected, 2),
        _ => panic!("expected PromptLibrary mode"),
    }
    app.prompt_library_select_next(); // wraps back to 0
    match &app.mode {
        AppMode::PromptLibrary(state) => assert_eq!(state.selected, 0),
        _ => panic!("expected PromptLibrary mode"),
    }
}

#[test]
fn prompt_library_filter_narrows_and_clamps_selection() {
    let store = store_with_prompt_templates(&["alpha", "beta", "gamma"]);
    let mut app = App::new_for_test(
        store,
        Box::new(MockTmuxOps::new()),
        Box::new(MockWorktreeOps::new()),
    );
    app.open_prompt_library(None);

    // Move selection to the last row, then filter to a single match so
    // the selection must clamp back into range.
    app.prompt_library_select_prev();
    if let AppMode::PromptLibrary(state) = &mut app.mode {
        assert_eq!(state.selected, 2);
        state.query = "alpha".to_string();
    }
    app.prompt_library_filter();

    match &app.mode {
        AppMode::PromptLibrary(state) => {
            assert_eq!(state.filtered.len(), 1);
            assert!(state.selected < state.filtered.len());
            let entry = state.selected_entry().unwrap();
            assert_eq!(entry.template.name, "alpha");
        }
        _ => panic!("expected PromptLibrary mode"),
    }
}

#[test]
fn prompt_library_inject_with_no_session_copies_and_exits() {
    let store = store_with_prompt_templates(&["only"]);
    let mut app = App::new_for_test(
        store,
        Box::new(MockTmuxOps::new()),
        Box::new(MockWorktreeOps::new()),
    );
    app.open_prompt_library(None);

    // No from_view, so injection falls back to a clipboard copy (which may
    // fail in headless CI) and returns to Normal either way.
    let _ = app.inject_selected_template();
    assert!(matches!(app.mode, AppMode::Normal));
}

#[test]
fn prompt_library_surfaces_global_config_templates_with_badge() {
    let store = store_with_prompt_templates(&["mine"]);
    let mut app = App::new_for_test(
        store,
        Box::new(MockTmuxOps::new()),
        Box::new(MockWorktreeOps::new()),
    );
    app.config.extension.prompt_templates = vec![crate::prompt_library::PromptTemplate::new(
        "shared".to_string(),
        "shared body".to_string(),
    )];

    app.open_prompt_library(None);

    match &app.mode {
        AppMode::PromptLibrary(state) => {
            assert_eq!(state.templates.len(), 2);
            let user = state
                .templates
                .iter()
                .find(|e| e.template.name == "mine")
                .unwrap();
            assert_eq!(user.source, crate::prompt_library::PromptSource::User);
            let global = state
                .templates
                .iter()
                .find(|e| e.template.name == "shared")
                .unwrap();
            assert_eq!(global.source, crate::prompt_library::PromptSource::Global);
            // Config entries can be edited but not deleted in place.
            assert!(global.source.is_editable());
            assert!(!global.source.is_deletable());
        }
        _ => panic!("expected PromptLibrary mode"),
    }
}

#[test]
fn prompt_library_export_to_global_updates_extension_config() {
    let store = store_with_prompt_templates(&["only"]);
    let mut app = App::new_for_test(
        store,
        Box::new(MockTmuxOps::new()),
        Box::new(MockWorktreeOps::new()),
    );
    app.open_prompt_library(None);

    app.export_selected_template(crate::app::PromptExportTarget::Global)
        .unwrap();

    // The in-memory global extension config gains the template (the file
    // write is a no-op in tests, which use an empty store_path).
    assert_eq!(app.config.extension.prompt_templates.len(), 1);
    assert_eq!(app.config.extension.prompt_templates[0].name, "only");

    // Exporting the same name again replaces rather than duplicates.
    app.open_prompt_library(None);
    app.export_selected_template(crate::app::PromptExportTarget::Global)
        .unwrap();
    assert_eq!(app.config.extension.prompt_templates.len(), 1);
}

#[test]
fn bare_uppercase_a_opens_harness_setup_from_dashboard() {
    // Regression: the `A` dispatch used to live in the leader-chord
    // handler, so a bare `A` on the dashboard did nothing. It must be
    // handled in the normal key handler like the other capital actions.
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
    app.mode = AppMode::Normal;
    let key =
        crossterm::event::KeyEvent::new(KeyCode::Char('A'), crossterm::event::KeyModifiers::NONE);
    crate::handlers::handle_normal_key(&mut app, key).unwrap();
    assert!(matches!(app.mode, AppMode::HarnessSetup(_)));
}

// Desktop debug history shares the DB reader without flushing pending writes.
fn debug_log_entry(second: i64, message: &str) -> crate::debug::LogEntry {
    crate::debug::LogEntry {
        timestamp: chrono::DateTime::from_timestamp(second, 0).unwrap(),
        level: crate::debug::LogLevel::Warn,
        context: "sync".into(),
        message: message.into(),
    }
}

#[test]
fn desktop_debug_log_merges_external_history_and_pending_without_writing() {
    let mut app = context_settings_test_app();
    let file = NamedTempFile::new().unwrap();
    app.db = Some(crate::db::AiwDb::open(file.path()).unwrap());
    let external = crate::db::AiwDb::open(file.path()).unwrap();
    external
        .append_log_entry(&debug_log_entry(20, "external"))
        .unwrap();
    app.pending_debug_log_entries
        .push_back(debug_log_entry(10, "pending"));
    app.debug_log
        .inject_entries(vec![debug_log_entry(5, "stale startup cache")]);
    let entries = app.recent_debug_log_entries().unwrap();
    assert_eq!(
        entries
            .iter()
            .map(|e| e.message.as_str())
            .collect::<Vec<_>>(),
        ["pending", "external"]
    );
    assert_eq!(app.pending_debug_log_entries.len(), 1);
    assert_eq!(external.load_recent_log(10).unwrap().len(), 1);
    external
        .append_log_entry(&debug_log_entry(30, "new external"))
        .unwrap();
    assert_eq!(
        app.recent_debug_log_entries()
            .unwrap()
            .last()
            .unwrap()
            .message,
        "new external"
    );
}

#[test]
fn desktop_debug_log_bounds_merged_history_and_preserves_pending_queue() {
    let mut app = context_settings_test_app();
    app.debug_log = crate::debug::DebugLog::new(2);
    let file = NamedTempFile::new().unwrap();
    let db = crate::db::AiwDb::open(file.path()).unwrap();
    for second in [10, 20, 30] {
        db.append_log_entry(&debug_log_entry(second, "persisted"))
            .unwrap();
    }
    app.db = Some(db);
    app.pending_debug_log_entries
        .push_back(debug_log_entry(25, "pending middle"));
    app.pending_debug_log_entries
        .push_back(debug_log_entry(40, "pending newest"));
    let entries = app.recent_debug_log_entries().unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].timestamp.timestamp(), 30);
    assert_eq!(entries[1].message, "pending newest");
    assert_eq!(app.pending_debug_log_entries.len(), 2);
}

#[test]
fn desktop_debug_log_without_database_returns_memory_and_serializes_levels() {
    let mut app = context_settings_test_app();
    app.db = None;
    app.debug_log
        .inject_entries(vec![debug_log_entry(10, "café\n<script>text</script>")]);
    let mut gui = crate::gui_contract::GuiHandle::from_app(app);
    let view = crate::gui_debug_log::load(&mut gui).unwrap();
    assert!(!view.shared_history);
    assert_eq!(view.limit, 1000);
    assert_eq!(view.entries.len(), 1);
    let json = serde_json::to_value(view).unwrap();
    assert_eq!(json["entries"][0]["level"], "WARN");
    assert_eq!(json["entries"][0]["message"], "café\n<script>text</script>");
}

#[test]
fn desktop_debug_log_database_failure_is_reported_instead_of_cached_history() {
    let mut app = context_settings_test_app();
    let file = NamedTempFile::new().unwrap();
    app.db = Some(crate::db::AiwDb::open(file.path()).unwrap());
    let external = rusqlite::Connection::open(file.path()).unwrap();
    external.execute("DROP TABLE debug_log", []).unwrap();
    app.debug_log
        .inject_entries(vec![debug_log_entry(10, "stale")]);
    let mut gui = crate::gui_contract::GuiHandle::from_app(app);
    let error = crate::gui_debug_log::load(&mut gui).unwrap_err();
    assert_eq!(error.kind, crate::gui_contract::GuiErrorKind::Internal);
    assert!(error.message.contains("debug_log"));
}
