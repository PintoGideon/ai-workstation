mod batch_creation;
mod browse;
mod config_wizard;
mod context_settings;
mod debug;
mod diff;
mod dormant;
mod editor_view;
mod feature;
pub(crate) mod fork_followups; // FORK: queued follow-ups (P7)
mod harness;
mod help;
mod hooks;
mod issue_fixer;
mod markdown;
mod project;
mod prompt_library;
mod resource_gate;
mod search;
mod session;
mod theme;
mod todos;

pub use batch_creation::draw_create_batch_features_dialog;
pub use browse::draw_browse_path_dialog;
pub use config_wizard::draw_config_wizard_dialog;
pub use context_settings::draw_context_settings_dialog;
pub use debug::draw_debug_log;
pub use diff::{draw_diff_picker, draw_diff_viewer, draw_diff_viewer_loading};
pub use dormant::draw_dormant_view;
pub use feature::{
    draw_confirm_supervibe_dialog, draw_create_feature_dialog, draw_delete_feature_confirm,
    draw_deleting_feature_dialog, draw_fork_feature_dialog,
};
pub use harness::draw_harness_setup_dialog;
pub use help::draw_help;
pub use hooks::{
    draw_diff_review_dialog, draw_hook_prompt_dialog, draw_latest_prompt_dialog,
    draw_running_hook_dialog,
};
pub use issue_fixer::{draw_issue_browser, draw_issue_duplicate_warning, draw_issue_setup};
pub use markdown::{draw_markdown_loading, draw_markdown_viewer};
pub use project::{draw_create_project_dialog, draw_delete_project_confirm};
pub use prompt_library::{
    draw_placeholder_fill, draw_prompt_editor, draw_prompt_library, draw_skill_picker,
};
pub use resource_gate::draw_resource_confirm_dialog;
pub use search::draw_search_dialog;
pub use session::{
    draw_new_session_name_dialog, draw_project_agent_config_dialog, draw_rename_feature_dialog,
    draw_rename_session_dialog, draw_session_config_dialog, draw_stopped_session_dialog,
};
pub use theme::draw_theme_picker;
pub use todos::{
    draw_todo_delete_disposition_dialog, draw_todo_implement_choice_dialog,
    draw_todo_quick_capture_dialog, draw_todo_reference_completion_dialog,
    draw_todo_spawn_target_dialog, draw_todos_host_reassign_dialog,
    draw_todos_view_with_visibility,
};
