mod batch_creation;
mod browse;
mod config_wizard;
mod context_settings;
mod dialog;
mod diff;
pub(crate) mod diff_review;
mod dormant;
mod feature_creation;
mod fork;
mod fork_background; // FORK: background Claude sessions (DESIGN.md §19.7)
mod fork_claude_worktrees; // FORK: `W` worktrees Claude made (DESIGN.md §19.6)
pub(crate) mod fork_followups; // FORK: queued follow-ups (P7)
mod fork_identity; // FORK: `f` fork dialog and `E` external sessions (DESIGN.md §19.2)
mod fork_land; // FORK: `P` land (DESIGN.md §19.10)
mod fork_messaging; // FORK: `@` messages (DESIGN.md §19.5)
mod fork_organize; // FORK: `G` groups and `#` notes (DESIGN.md §19.4)
pub(crate) mod fork_permissions; // FORK: permission approvals (P6)
mod fork_schedules; // FORK: `t` schedules (DESIGN.md §19.9)
mod fork_watchdog; // FORK: `!` recovery actions (DESIGN.md §19.8)
pub(crate) mod fork_workspace; // FORK: workspace layout
mod harness;
mod hooks;
mod input;
mod issue_fixer;
mod mouse;
mod normal;
mod picker;
mod prompt_library;
mod search;
mod skill_picker;
mod todos;

use anyhow::Result;
use crossterm::event::{KeyCode, KeyEvent};

use crate::app::App;

pub use batch_creation::handle_create_batch_features_key;
pub use browse::handle_browse_path_key;
pub use config_wizard::handle_config_wizard_key;
pub use context_settings::handle_context_settings_key;
pub use dialog::{
    handle_create_project_key, handle_debug_log_key, handle_delete_feature_key,
    handle_delete_project_key, handle_help_key, handle_latest_prompt_key,
    handle_markdown_viewer_key, handle_rename_feature_key, handle_rename_session_key,
    handle_resource_confirm_key, handle_session_config_key, handle_stopped_session_dialog_key,
    handle_theme_picker_key,
};
pub use diff::{handle_diff_picker_key, handle_diff_viewer_key, handle_diff_viewer_wheel};
pub use diff_review::handle_diff_review_key;
pub use dormant::handle_dormant_key;
#[cfg(test)]
pub use feature_creation::handle_create_feature_key;
pub use feature_creation::handle_create_feature_key_event;
pub use fork::handle_fork_feature_key;
pub use harness::handle_harness_setup_key;
pub use hooks::{handle_deleting_feature_key, handle_hook_prompt_key, handle_running_hook_key};
pub use input::handle_paste;
pub use issue_fixer::{
    handle_issue_browser_key, handle_issue_duplicate_warning_key, handle_issue_setup_key,
};
pub use mouse::handle_mouse;
pub use normal::handle_normal_key;
pub(crate) use normal::{
    DASHBOARD_KEYBINDING_ACTIONS, EXTRA_KEYBINDING_ACTIONS, default_key_for_action,
};
pub use picker::{
    handle_bookmark_picker_key, handle_claude_session_confirm_key,
    handle_claude_session_picker_key, handle_codex_session_confirm_key,
    handle_codex_session_picker_key, handle_command_picker_key, handle_markdown_file_picker_key,
    handle_new_session_name_key, handle_notification_picker_key, handle_session_picker_key,
    handle_syntax_language_picker_key,
};
pub use prompt_library::{
    handle_placeholder_fill_key, handle_prompt_editor_key, handle_prompt_library_key,
};
pub use search::handle_search_key;
pub use skill_picker::handle_skill_picker_key;
pub use todos::{
    handle_todo_delete_disposition_key, handle_todo_implement_choice_key,
    handle_todo_quick_capture_key, handle_todo_reference_completion_key,
    handle_todo_spawn_target_key, handle_todos_host_reassign_key, handle_todos_key,
};

pub fn handle_key(app: &mut App, key: KeyEvent) -> Result<()> {
    use crate::app::AppMode;

    match &app.mode {
        AppMode::Normal => handle_normal_key(app, key),
        AppMode::Todos(_) => handle_todos_key(app, key),
        AppMode::TodoQuickCapture(_) => handle_todo_quick_capture_key(app, key),
        AppMode::TodosHostReassign(_) => handle_todos_host_reassign_key(app, key.code),
        AppMode::TodoImplementChoice(_) => handle_todo_implement_choice_key(app, key.code),
        AppMode::TodoSpawnTarget(_) => handle_todo_spawn_target_key(app, key.code),
        AppMode::TodoDeleteDisposition(_) => handle_todo_delete_disposition_key(app, key.code),
        AppMode::ConfirmTodoReferenceCompletion(_) => {
            handle_todo_reference_completion_key(app, key.code)
        }
        AppMode::CreatingProject(_) => handle_create_project_key(app, key),
        AppMode::BrowsingPath(_) => handle_browse_path_key(app, key),
        AppMode::CreatingFeature(_) => handle_create_feature_key_event(app, key), // FORK: §19.3
        AppMode::CreatingBatchFeatures(_) => handle_create_batch_features_key(app, key.code),
        AppMode::DeletingProject(_) => handle_delete_project_key(app, key.code),
        AppMode::DeletingFeature(_, _) => handle_delete_feature_key(app, key.code),
        AppMode::ConfirmResourceStart(_) => handle_resource_confirm_key(app, key.code),
        AppMode::Dormant(_) => handle_dormant_key(app, key),
        AppMode::Help(_) => handle_help_key(app, key),
        AppMode::NotificationPicker(_, _) => handle_notification_picker_key(app, key.code),
        AppMode::RenamingSession(_) => handle_rename_session_key(app, key.code),
        AppMode::RenamingFeature(_) => handle_rename_feature_key(app, key.code),
        AppMode::SessionConfig(_) => handle_session_config_key(app, key.code),
        AppMode::ProjectAgentConfig(_) => handle_session_config_key(app, key.code),
        AppMode::CommandPicker(_) => handle_command_picker_key(app, key.code),
        AppMode::MarkdownFilePicker(_) => handle_markdown_file_picker_key(app, key),
        AppMode::Searching(_) => handle_search_key(app, key.code),
        AppMode::NamingNewSession(_) => handle_new_session_name_key(app, key.code),
        AppMode::ClaudeSessionPicker(_) => handle_claude_session_picker_key(app, key.code),
        AppMode::ConfirmingClaudeSession { .. } => handle_claude_session_confirm_key(app, key.code),
        AppMode::CodexSessionPicker(_) => handle_codex_session_picker_key(app, key.code),
        AppMode::ConfirmingCodexSession { .. } => handle_codex_session_confirm_key(app, key.code),
        AppMode::StoppedSessionDialog(_) => handle_stopped_session_dialog_key(app, key),
        AppMode::SessionPicker(_) => handle_session_picker_key(app, key.code),
        AppMode::BookmarkPicker(_) => handle_bookmark_picker_key(app, key.code),
        AppMode::DiffPicker(_) => handle_diff_picker_key(app, key),
        AppMode::DiffViewerLoading(_) => handle_diff_viewer_key(app, key),
        AppMode::DiffViewer(_) => handle_diff_viewer_key(app, key),
        AppMode::IssueBrowser(_) => handle_issue_browser_key(app, key),
        AppMode::IssueSetup(_) => handle_issue_setup_key(app, key),
        AppMode::IssueDuplicateWarning(_) => handle_issue_duplicate_warning_key(app, key),
        AppMode::DiffReviewPrompt(_) => handle_diff_review_key(app, key),
        AppMode::RunningHook(_) => handle_running_hook_key(app, key.code),
        AppMode::DeletingFeatureInProgress(_) => handle_deleting_feature_key(app, key.code),
        AppMode::HookPrompt(_) => handle_hook_prompt_key(app, key.code),
        // FORK: repo hook approval dialog.
        AppMode::ApprovingRepoHook(_) => crate::fork::hook_approval::handle_key(app, key.code),
        // FORK: permission detail overlay (P6).
        AppMode::PermissionDetail(_) => fork_permissions::handle_detail_key(app, key),
        // FORK: the follow-up queue dialog (P7).
        AppMode::FollowupQueue(_) => fork_followups::handle_dialog_key(app, key),
        AppMode::LatestPrompt(_) => handle_latest_prompt_key(app, key.code),
        AppMode::PromptLibrary(_) => handle_prompt_library_key(app, key.code),
        AppMode::PromptEditor(_) => handle_prompt_editor_key(app, key),
        AppMode::PlaceholderFill(_) => handle_placeholder_fill_key(app, key),
        AppMode::SkillPicker(_) => handle_skill_picker_key(app, key),
        AppMode::ForkingFeature(_) => handle_fork_feature_key(app, key.code),
        AppMode::ForkSession(_) => fork_identity::handle_fork_session_key(app, key.code),
        AppMode::ExternalSessions(_) => fork_identity::handle_external_sessions_key(app, key.code),
        AppMode::ClaudeWorktrees(_) => {
            fork_claude_worktrees::handle_claude_worktrees_key(app, key.code)
        }
        // FORK: §19.7
        AppMode::BackgroundPrompt(_) => fork_background::handle_background_prompt_key(app, key),
        AppMode::BackgroundRemove(_) => fork_background::handle_background_remove_key(app, key),
        // FORK: §19.4
        AppMode::FeatureGroups(_) => fork_organize::handle_feature_groups_key(app, key),
        AppMode::NoteEditor(_) => fork_organize::handle_note_editor_key(app, key),
        AppMode::MessageSession(_) => fork_messaging::handle_message_dialog_key(app, key),
        AppMode::Watchdog(_) => fork_watchdog::handle_watchdog_key(app, key),
        AppMode::Schedules(_) => fork_schedules::handle_schedules_key(app, key),
        AppMode::ScheduleEditor(_) => fork_schedules::handle_schedule_editor_key(app, key),
        AppMode::Land(_) => fork_land::handle_land_key(app, key),
        AppMode::ThemePicker(_) => handle_theme_picker_key(app, key.code),
        AppMode::SyntaxLanguagePicker(_) => handle_syntax_language_picker_key(app, key.code),
        AppMode::DebugLog(_) => handle_debug_log_key(app, key.code),
        AppMode::MarkdownLoading(_) => {
            if matches!(key.code, KeyCode::Esc | KeyCode::Char('q')) {
                app.cancel_markdown_loading();
            }
            Ok(())
        }
        AppMode::MarkdownViewer(_) => handle_markdown_viewer_key(app, key),
        AppMode::HarnessSetup(_) => handle_harness_setup_key(app, key.code),
        AppMode::ConfigWizard(_) => handle_config_wizard_key(app, key),
        AppMode::ContextSettings(_) => handle_context_settings_key(app, key),
    }
}
