use std::path::{Path, PathBuf};

use crate::project::{AgentKind, Feature, ProjectStore, SessionKind, VibeMode};

use super::AppConfig;

const NOTIFY_SH: &str = include_str!("../../scripts/notify.sh");
const CLEAR_NOTIFY_SH: &str = include_str!("../../scripts/clear-notify.sh");
const SAVE_PROMPT_SH: &str = include_str!("../../scripts/save-prompt.sh");
const THINKING_START_SH: &str = include_str!("../../scripts/thinking-start.sh");
const THINKING_STOP_SH: &str = include_str!("../../scripts/thinking-stop.sh");
const TOOL_START_SH: &str = include_str!("../../scripts/tool-start.sh");
const TOOL_STOP_SH: &str = include_str!("../../scripts/tool-stop.sh");
const ATTENTION_SH: &str = include_str!("../../scripts/attention.sh");
const CODEX_NOTIFY_SH: &str = include_str!("../../scripts/codex-notify.sh");
const CODEX_DIFF_REVIEW_SH: &str = include_str!("../../scripts/codex-diff-review.sh");
const SET_SESSION_STATUS_SH: &str = include_str!("../../scripts/set-session-status.sh");
// FORK: hook-driven productivity scripts (DESIGN.md §16).
const PERMISSION_REQUEST_SH: &str = include_str!("../../scripts/permission-request.sh");
const STATUS_LINE_SH: &str = include_str!("../../scripts/status-line.sh");
const STOP_HOOK_SH: &str = include_str!("../../scripts/stop-hook.sh");
const SUBAGENT_SH: &str = include_str!("../../scripts/subagent.sh");
const SESSION_CONTEXT_SH: &str = include_str!("../../scripts/session-context.sh");
const CUSTOM_DIFF_REVIEW_SH: &str =
    include_str!("../../plugins/diff-review/scripts/custom-diff-review.sh");

const AIW_SKILLS: &[(&str, &str)] = &[
    (
        "add-session",
        include_str!("../../skills/aiw-add-session/SKILL.md"),
    ),
    (
        "add-hook",
        include_str!("../../skills/aiw-add-hook/SKILL.md"),
    ),
    (
        "add-preset",
        include_str!("../../skills/aiw-add-preset/SKILL.md"),
    ),
    (
        "add-prompt",
        include_str!("../../skills/aiw-add-prompt/SKILL.md"),
    ),
    (
        "configure",
        include_str!("../../skills/aiw-configure/SKILL.md"),
    ),
    (
        "release-notes",
        include_str!("../../skills/aiw-release-notes/SKILL.md"),
    ),
];
const CLAUDE_SETTINGS_LOCAL_JSON: &str = "settings.local.json";
const CLAUDE_SETTINGS_JSON: &str = "settings.json";
const CLAUDE_STATE_JSON: &str = "aiw-hook-state.json";
const HOOK_REFRESH_STAMP: &str = concat!(env!("CARGO_PKG_VERSION"), ":jqless-hooks-v10");
/// The Claude hook events AIW writes entries for (and cleans up again).
const CLAUDE_MANAGED_EVENTS: &[&str] = &[
    "Stop",
    "Notification",
    "PreToolUse",
    "PostToolUse",
    "UserPromptSubmit",
    // FORK: DESIGN.md §16
    "PermissionRequest",
    "PreCompact",
    "SessionEnd",
    "SubagentStart",
    "SubagentStop",
    // FORK: DESIGN.md §19.1
    "StopFailure",
    "PostCompact",
    "PostModelSwitch",
    "TaskCreated",
    "TaskCompleted",
    "CwdChanged",
    "PermissionDenied",
    // FORK: DESIGN.md §19.3
    "SessionStart",
];
const CLAUDE_MANAGED_SCRIPT_NAMES: &[&str] = &[
    "notify.sh",
    "clear-notify.sh",
    "save-prompt.sh",
    "thinking-start.sh",
    "thinking-stop.sh",
    "tool-start.sh",
    "tool-stop.sh",
    "attention.sh",
    // FORK: DESIGN.md §16
    "permission-request.sh",
    "stop-hook.sh",
    "subagent.sh",
    "hook-event.sh", // FORK: DESIGN.md §19.1
];

#[derive(Default)]
struct ClaudeSettingsState {
    permissions_added: Vec<String>,
}

impl ClaudeSettingsState {
    fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|value| {
                value
                    .get("permissions_added")
                    .and_then(|v| v.as_array())
                    .map(|entries| Self {
                        permissions_added: entries
                            .iter()
                            .filter_map(|entry| entry.as_str().map(ToOwned::to_owned))
                            .collect(),
                    })
            })
            .unwrap_or_default()
    }

    fn save(&self, path: &Path) {
        if self.permissions_added.is_empty() {
            let _ = std::fs::remove_file(path);
            return;
        }

        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let _ = std::fs::write(
            path,
            serde_json::to_string_pretty(&serde_json::json!({
                "permissions_added": self.permissions_added,
            }))
            .unwrap_or_default()
                + "\n",
        );
    }
}

fn read_json_object(path: &Path) -> serde_json::Value {
    match std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
    {
        Some(value) if value.is_object() => value,
        _ => serde_json::json!({}),
    }
}

fn write_json_object(path: &Path, value: &serde_json::Value) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(
        path,
        serde_json::to_string_pretty(value).unwrap_or_default() + "\n",
    );
}

fn ensure_gitignore_entry(path: &Path, entry: &str) {
    let needs_entry = std::fs::read_to_string(path)
        .map(|s| !s.lines().any(|line| line == entry))
        .unwrap_or(true);
    if needs_entry
        && let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
    {
        use std::io::Write as _;
        let _ = f.write_all(format!("{entry}\n").as_bytes());
    }
}

fn claude_managed_commands() -> Vec<String> {
    [
        crate::project::aiw_claude_hooks_dir(),
        crate::project::aiw_config_dir(),
    ]
    .into_iter()
    .flat_map(|dir| {
        CLAUDE_MANAGED_SCRIPT_NAMES
            .iter()
            .map(move |name| dir.join(name).to_string_lossy().into_owned())
    })
    .collect()
}

#[cfg(test)]
use crate::fork::shell::quote as shell_quote;

fn shell_unquote_single(value: &str) -> String {
    value
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
        .map(|value| value.replace("'\\''", "'"))
        .unwrap_or_else(|| value.to_string())
}

#[cfg(test)]
pub(crate) fn claude_hook_command(path: &Path) -> String {
    shell_quote(&path.to_string_lossy())
}

pub(crate) fn claude_exec_hook(path: &Path) -> serde_json::Value {
    serde_json::json!({
        "type": "command",
        "command": path.to_string_lossy(),
        "args": []
    })
}

/// `claude_exec_hook` with arguments — used by `attention.sh`, which takes the
/// event kind (`question` / `completed`) as argv[1] so one script serves every
/// lifecycle event that means "this session stopped".
pub(crate) fn claude_exec_hook_with_args(path: &Path, args: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "type": "command",
        "command": path.to_string_lossy(),
        "args": args
    })
}

/// FORK: `claude_exec_hook_with_args` with a per-hook `timeout` (seconds) for
/// hooks that wait on the user (DESIGN.md §16).
pub(crate) fn claude_exec_hook_with_timeout(
    path: &Path,
    args: &[&str],
    timeout_secs: u64,
) -> serde_json::Value {
    let mut hook = claude_exec_hook_with_args(path, args);
    hook["timeout"] = serde_json::json!(timeout_secs);
    hook
}

pub(crate) fn is_aiw_claude_hook_command(command: &str, managed_commands: &[String]) -> bool {
    let normalized = shell_unquote_single(command);
    if managed_commands
        .iter()
        .any(|managed| managed == command || managed == &normalized)
    {
        return true;
    }

    // The config-dir resolver depends on HOME/XDG_CONFIG_HOME and can differ
    // across platforms or across runs (e.g. macOS' Library/Application
    // Support path, or a custom XDG_CONFIG_HOME pointed at an arbitrary
    // directory). `aiw_config_dir()` always resolves to
    // `<config root>/ai_workstation` regardless of what the config root itself
    // is named, so the only structural invariant we can rely on for a
    // *previous* run's config root is the immediate parent directory's name.
    // Treat only known helper names under such a directory as managed so
    // unrelated user hooks with the same basename are preserved.
    //
    // FORK: hooks written by upstream AMF (`<config root>/amf/…` and
    // `~/.amf/hooks/…`) are recognised too, so restarting a migrated feature
    // replaces AMF's entries instead of running both sets side by side.
    let Some((parent, name)) = normalized.rsplit_once('/') else {
        return false;
    };
    let parent = Path::new(parent);
    let dir_name = |path: &Path| {
        path.file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string)
    };
    let parent_name = dir_name(parent);
    let grandparent_name = parent.parent().and_then(dir_name);
    let is_config_dir = matches!(
        parent_name.as_deref(),
        Some(crate::project::AIW_DIR_NAME | crate::project::LEGACY_AMF_DIR_NAME)
    );
    let is_safe_hook_dir = parent_name.as_deref() == Some("hooks")
        && matches!(
            grandparent_name.as_deref(),
            Some(crate::project::AIW_DIR_NAME | ".amf")
        );
    CLAUDE_MANAGED_SCRIPT_NAMES.contains(&name) && (is_config_dir || is_safe_hook_dir)
}

fn is_unquoted_aiw_claude_hook(hook: &serde_json::Value, managed_commands: &[String]) -> bool {
    // Claude's exec form passes `command` directly when `args` is present, so
    // a bare path containing spaces is safe there. Only shell-form commands
    // without `args` need quoting and migration.
    !hook["args"].is_array()
        && hook["command"].as_str().is_some_and(|command| {
            !(command.starts_with('\'') && command.ends_with('\''))
                && is_aiw_claude_hook_command(command, managed_commands)
        })
}

fn has_unquoted_aiw_claude_hooks(
    settings: &serde_json::Value,
    managed_commands: &[String],
) -> bool {
    settings
        .get("hooks")
        .and_then(|value| value.as_object())
        .is_some_and(|hooks_obj| {
            hooks_obj.values().any(|entries| {
                entries.as_array().is_some_and(|entries| {
                    entries.iter().any(|entry| {
                        entry["hooks"].as_array().is_some_and(|hooks| {
                            hooks
                                .iter()
                                .any(|hook| is_unquoted_aiw_claude_hook(hook, managed_commands))
                        })
                    })
                })
            })
        })
}

fn is_aiw_claude_hook_entry(entry: &serde_json::Value, managed_commands: &[String]) -> bool {
    entry["hooks"].as_array().is_some_and(|hooks| {
        hooks.iter().any(|hook| {
            hook["command"]
                .as_str()
                .is_some_and(|command| is_aiw_claude_hook_command(command, managed_commands))
        })
    })
}

fn remove_aiw_claude_hooks(settings: &mut serde_json::Value, managed_commands: &[String]) -> bool {
    let Some(hooks_obj) = settings
        .get_mut("hooks")
        .and_then(|value| value.as_object_mut())
    else {
        return false;
    };

    let mut changed = false;
    for event in CLAUDE_MANAGED_EVENTS {
        let Some(entries) = hooks_obj
            .get_mut(*event)
            .and_then(|value| value.as_array_mut())
        else {
            continue;
        };
        let before = entries.len();
        entries.retain(|entry| !is_aiw_claude_hook_entry(entry, managed_commands));
        changed |= entries.len() != before;
    }

    hooks_obj.retain(|_, value| value.as_array().is_none_or(|entries| !entries.is_empty()));

    if hooks_obj.is_empty() {
        settings.as_object_mut().unwrap().remove("hooks");
        changed = true;
    }

    changed
}

/// Rewrite AIW-owned commands in an inherited Claude settings file without
/// replacing its hook layout. Worktrees can inherit `.claude` settings from
/// the root repository, so refreshing only the worktree-local file leaves a
/// stale root command active in Claude.
fn migrate_aiw_claude_hooks_to_runtime_dir(
    settings_path: &Path,
    managed_commands: &[String],
) -> bool {
    let Some(mut settings) = std::fs::read_to_string(settings_path)
        .ok()
        .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
        .filter(serde_json::Value::is_object)
    else {
        return false;
    };
    let Some(hooks) = settings
        .get_mut("hooks")
        .and_then(serde_json::Value::as_object_mut)
    else {
        return false;
    };

    let runtime_dir = crate::project::aiw_claude_hooks_dir();
    let mut changed = false;
    for entries in hooks
        .values_mut()
        .filter_map(serde_json::Value::as_array_mut)
    {
        for entry in entries {
            let Some(commands) = entry
                .get_mut("hooks")
                .and_then(serde_json::Value::as_array_mut)
            else {
                continue;
            };
            for hook in commands {
                let Some(command) = hook.get("command").and_then(serde_json::Value::as_str) else {
                    continue;
                };
                if !is_aiw_claude_hook_command(command, managed_commands) {
                    continue;
                }
                let normalized = shell_unquote_single(command);
                let Some(script_name) = Path::new(&normalized).file_name() else {
                    continue;
                };
                let replacement = runtime_dir.join(script_name).to_string_lossy().into_owned();
                let uses_current_command = command == replacement;
                let uses_exec_args = hook
                    .get("args")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(Vec::is_empty);
                if uses_current_command && uses_exec_args {
                    continue;
                }
                hook["command"] = serde_json::Value::String(replacement);
                hook["args"] = serde_json::json!([]);
                changed = true;
            }
        }
    }

    if changed {
        write_json_object(settings_path, &settings);
    }
    changed
}

fn repair_inherited_repo_claude_hooks(
    workdir: &Path,
    repo: &Path,
    is_worktree: bool,
    managed_commands: &[String],
) -> usize {
    if !is_worktree || workdir == repo {
        return 0;
    }

    let claude_dir = repo.join(".claude");
    [CLAUDE_SETTINGS_LOCAL_JSON, CLAUDE_SETTINGS_JSON]
        .into_iter()
        .filter(|name| {
            migrate_aiw_claude_hooks_to_runtime_dir(&claude_dir.join(name), managed_commands)
        })
        .count()
}

fn push_claude_hook_entry(settings: &mut serde_json::Value, event: &str, entry: serde_json::Value) {
    let root = settings.as_object_mut().unwrap();
    let hooks = root
        .entry("hooks")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .unwrap();
    let entries = hooks
        .entry(event.to_string())
        .or_insert_with(|| serde_json::json!([]))
        .as_array_mut()
        .unwrap();
    entries.push(entry);
}

fn update_claude_permissions(
    settings: &mut serde_json::Value,
    state_path: &Path,
    wants_diff_review: bool,
) {
    let mut state = ClaudeSettingsState::load(state_path);

    if wants_diff_review {
        {
            let perms = settings
                .as_object_mut()
                .unwrap()
                .entry("permissions")
                .or_insert_with(|| serde_json::json!({}))
                .as_object_mut()
                .unwrap();
            let allow = perms
                .entry("allow")
                .or_insert_with(|| serde_json::json!([]))
                .as_array_mut()
                .unwrap();

            for permission in ["Edit", "Write"] {
                if !allow.iter().any(|value| value.as_str() == Some(permission)) {
                    allow.push(serde_json::json!(permission));
                    if !state
                        .permissions_added
                        .iter()
                        .any(|value| value == permission)
                    {
                        state.permissions_added.push(permission.to_string());
                    }
                }
            }
        }
    } else if let Some(allow) = settings
        .pointer_mut("/permissions/allow")
        .and_then(|value| value.as_array_mut())
    {
        allow.retain(|value| {
            value.as_str().is_none_or(|permission| {
                !state
                    .permissions_added
                    .iter()
                    .any(|added| added == permission)
            })
        });
        state.permissions_added.clear();
    }

    if settings
        .pointer("/permissions/allow")
        .and_then(|value| value.as_array())
        .is_some_and(|allow| allow.is_empty())
        && let Some(permissions) = settings
            .get_mut("permissions")
            .and_then(|value| value.as_object_mut())
    {
        permissions.remove("allow");
    }

    if settings
        .get("permissions")
        .and_then(|value| value.as_object())
        .is_some_and(|permissions| permissions.is_empty())
    {
        settings.as_object_mut().unwrap().remove("permissions");
    }

    state.save(state_path);
}

fn cleanup_claude_settings_file(path: &Path, state_path: Option<&Path>) {
    if !path.exists() {
        return;
    }

    let managed_commands = claude_managed_commands();
    let mut settings = read_json_object(path);
    let had_aiw_hooks = remove_aiw_claude_hooks(&mut settings, &managed_commands);

    if let Some(state_path) = state_path {
        update_claude_permissions(&mut settings, state_path, false);
    } else if had_aiw_hooks
        && let Some(allow) = settings
            .pointer_mut("/permissions/allow")
            .and_then(|value| value.as_array_mut())
    {
        allow.retain(|value| value.as_str() != Some("Edit") && value.as_str() != Some("Write"));

        if settings
            .pointer("/permissions/allow")
            .and_then(|value| value.as_array())
            .is_some_and(|entries| entries.is_empty())
            && let Some(permissions) = settings
                .get_mut("permissions")
                .and_then(|value| value.as_object_mut())
        {
            permissions.remove("allow");
        }

        if settings
            .get("permissions")
            .and_then(|value| value.as_object())
            .is_some_and(|permissions| permissions.is_empty())
        {
            settings.as_object_mut().unwrap().remove("permissions");
        }
    }

    if settings.as_object().is_some_and(|root| root.is_empty()) {
        let _ = std::fs::remove_file(path);
    } else {
        write_json_object(path, &settings);
    }
}

/// Atomically write `content` to `path`: stage it in a temp file in the same
/// directory (so the final `rename` stays on one filesystem and is atomic),
/// optionally set its mode, then rename over the target.
///
/// This matters because these files are hook scripts that the agent harness
/// executes. A plain in-place `fs::write` truncates and rewrites the *same*
/// inode, so a shell that is mid-execution reading the script can observe a
/// half-rewritten file and fail with a spurious syntax error. Renaming a fully
/// written temp over the target means a running hook keeps reading the complete
/// old inode while new runs pick up the complete new one — never a mix.
fn write_atomic(path: &Path, content: &str, mode: Option<u32>) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("script");
    let tmp = dir.join(format!(".{file_name}.tmp.{}", std::process::id()));

    let write_result = (|| {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
        #[cfg(unix)]
        if let Some(mode) = mode {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(std::fs::Permissions::from_mode(mode))?;
        }
        #[cfg(not(unix))]
        let _ = mode;
        std::fs::rename(&tmp, path)
    })();

    if write_result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    write_result
}

/// Write `content` to `path` only when the on-disk content differs.
/// Avoids unnecessary write syscalls when the file is already up to date,
/// which matters because `ensure_notify_scripts` is called once per feature
/// on startup. The write itself is atomic (see `write_atomic`).
#[cfg(not(unix))]
fn write_if_changed(path: &Path, content: &str) {
    if std::fs::read_to_string(path).ok().as_deref() == Some(content) {
        return;
    }
    let _ = write_atomic(path, content, None);
}

/// Same as `write_if_changed` but also ensures the file is executable.
/// Permissions are only set when the file content was actually rewritten.
#[cfg(unix)]
fn write_executable_if_changed(path: &Path, content: &str) {
    if std::fs::read_to_string(path).ok().as_deref() == Some(content) {
        return;
    }
    let _ = write_atomic(path, content, Some(0o755));
}

#[cfg(not(unix))]
fn write_executable_if_changed(path: &Path, content: &str) {
    write_if_changed(path, content);
}

pub fn ensure_notify_scripts() {
    let config_dir = crate::project::aiw_config_dir();
    let claude_hooks_dir = crate::project::aiw_claude_hooks_dir();
    let _ = std::fs::create_dir_all(&config_dir);
    let _ = std::fs::create_dir_all(&claude_hooks_dir);
    write_executable_if_changed(&claude_hooks_dir.join("notify.sh"), NOTIFY_SH);
    write_executable_if_changed(&claude_hooks_dir.join("clear-notify.sh"), CLEAR_NOTIFY_SH);
    write_executable_if_changed(&claude_hooks_dir.join("save-prompt.sh"), SAVE_PROMPT_SH);
    write_executable_if_changed(
        &claude_hooks_dir.join("thinking-start.sh"),
        THINKING_START_SH,
    );
    write_executable_if_changed(&claude_hooks_dir.join("thinking-stop.sh"), THINKING_STOP_SH);
    write_executable_if_changed(&claude_hooks_dir.join("tool-start.sh"), TOOL_START_SH);
    write_executable_if_changed(&claude_hooks_dir.join("tool-stop.sh"), TOOL_STOP_SH);
    write_executable_if_changed(&claude_hooks_dir.join("attention.sh"), ATTENTION_SH);
    // FORK: DESIGN.md §16
    write_executable_if_changed(
        &claude_hooks_dir.join("permission-request.sh"),
        PERMISSION_REQUEST_SH,
    );
    write_executable_if_changed(&claude_hooks_dir.join("status-line.sh"), STATUS_LINE_SH);
    write_executable_if_changed(&claude_hooks_dir.join("stop-hook.sh"), STOP_HOOK_SH);
    write_executable_if_changed(&claude_hooks_dir.join("subagent.sh"), SUBAGENT_SH);
    write_executable_if_changed(
        &claude_hooks_dir.join(crate::fork::hook_events::SCRIPT_NAME),
        crate::fork::hook_events::SCRIPT,
    );
    write_executable_if_changed(
        &claude_hooks_dir.join("session-context.sh"),
        SESSION_CONTEXT_SH,
    );
    write_executable_if_changed(
        &config_dir.join("codex-diff-review.sh"),
        CODEX_DIFF_REVIEW_SH,
    );
    write_executable_if_changed(&config_dir.join("codex-notify.sh"), CODEX_NOTIFY_SH);
    write_executable_if_changed(
        &config_dir.join("set-session-status.sh"),
        SET_SESSION_STATUS_SH,
    );
    write_executable_if_changed(
        &claude_hooks_dir.join("custom-diff-review.sh"),
        CUSTOM_DIFF_REVIEW_SH,
    );
}

/// Path of the version-stamp file used to skip hook/plugin refresh when
/// the AIW binary version has not changed since the last startup.
fn hook_refresh_stamp_path() -> std::path::PathBuf {
    crate::project::aiw_config_dir().join("last_hook_refresh_version")
}

/// Returns `true` when the stamp matches the current binary version, meaning
/// every feature's hooks and plugins were already refreshed by this exact
/// version of AIW and nothing needs to be rewritten.
fn hooks_already_current() -> bool {
    std::fs::read_to_string(hook_refresh_stamp_path())
        .ok()
        .as_deref()
        == Some(HOOK_REFRESH_STAMP)
}

/// Record that hooks have been refreshed for the current binary version.
fn mark_hooks_current() {
    let _ = std::fs::write(hook_refresh_stamp_path(), HOOK_REFRESH_STAMP);
}

pub fn refresh_claude_hooks_for_store(store: &ProjectStore) -> usize {
    if hooks_already_current() {
        return 0;
    }
    let mut refreshed = 0usize;
    for project in &store.projects {
        for feature in &project.features {
            if !feature_uses_claude(feature) {
                continue;
            }
            ensure_notification_hooks(
                &feature.workdir,
                &project.repo,
                &feature.mode,
                &AgentKind::Claude,
                feature.is_worktree,
            );
            refreshed += 1;
        }
    }
    mark_hooks_current();
    refreshed
}

pub fn repair_unquoted_claude_hooks_for_store(store: &ProjectStore) -> usize {
    let managed_commands = claude_managed_commands();
    let mut repaired = 0usize;
    for project in &store.projects {
        for feature in &project.features {
            if !feature_uses_claude(feature) {
                continue;
            }
            let claude_dir = feature.workdir.join(".claude");
            let has_stale_hook = [CLAUDE_SETTINGS_LOCAL_JSON, CLAUDE_SETTINGS_JSON]
                .into_iter()
                .map(|name| read_json_object(&claude_dir.join(name)))
                .any(|settings| has_unquoted_aiw_claude_hooks(&settings, &managed_commands));
            if !has_stale_hook {
                continue;
            }
            ensure_notification_hooks(
                &feature.workdir,
                &project.repo,
                &feature.mode,
                &AgentKind::Claude,
                feature.is_worktree,
            );
            repaired += 1;
        }
    }
    repaired
}

fn feature_uses_claude(feature: &Feature) -> bool {
    matches!(feature.agent, AgentKind::Claude)
        || feature
            .sessions
            .iter()
            .any(|session| matches!(session.kind, SessionKind::Claude))
}

/// Apply one-time, version-gated migrations to a freshly loaded config.
/// Returns `true` if anything changed and the file should be rewritten.
///
/// Each step only runs when `config_version` is below its version, and
/// only rewrites a value that still equals the old default — a user who
/// deliberately set the value after migrating is never clobbered, because
/// the version bump stops the step from firing twice.
pub(crate) fn migrate_app_config(config: &mut AppConfig) -> bool {
    let mut changed = false;

    if config.config_version < 1 {
        // v1: the persistent control-mode (-CC PTY) transport corrupts the
        // agent pane on Unix, so move pre-flip configs to the new direct
        // default. Also shorten the diff-review popup hold from its old 3.0s
        // default to the new 1.5s.
        if config.tmux_control_mode {
            config.tmux_control_mode = false;
            changed = true;
        }
        if (config.diff_review_popup_hold_secs - 3.0).abs() < f64::EPSILON {
            config.diff_review_popup_hold_secs = 1.5;
            changed = true;
        }
    }

    if config.config_version < crate::app::APP_CONFIG_VERSION {
        config.config_version = crate::app::APP_CONFIG_VERSION;
        changed = true;
    }

    changed
}

/// The configuration exactly as it is on disk, touching nothing.
///
/// [`load_config`] is a write path in three ways: it creates `config.json` when
/// it is missing and rewrites it when a key migrates. `aiw doctor` needs the
/// same settings and must leave all of that alone, so it reads through here instead — migrations
/// are applied in memory only.
pub fn read_config() -> AppConfig {
    let config_path = crate::project::aiw_config_dir().join("config.json");
    let mut config: AppConfig = std::fs::read_to_string(&config_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    migrate_app_config(&mut config);
    config
}

pub fn load_config() -> AppConfig {
    let config_path = crate::project::aiw_config_dir().join("config.json");

    if config_path.exists() {
        let mut config: AppConfig = std::fs::read_to_string(&config_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        if migrate_app_config(&mut config) {
            let _ = std::fs::write(
                &config_path,
                serde_json::to_string_pretty(&config).unwrap_or_default(),
            );
        }
        config
    } else {
        let config = AppConfig::default();
        let dir = config_path.parent().unwrap();
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(
            &config_path,
            serde_json::to_string_pretty(&config).unwrap_or_default(),
        );
        config
    }
}

pub fn remove_old_diff_review_plugin(repo: &Path) {
    let settings_path = repo.join(".claude").join(CLAUDE_SETTINGS_LOCAL_JSON);
    if !settings_path.exists() {
        return;
    }

    let mut settings: serde_json::Value = match std::fs::read_to_string(&settings_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
    {
        Some(v) => v,
        None => return,
    };

    let changed = settings
        .get_mut("enabledPlugins")
        .and_then(|p| p.as_object_mut())
        .map(|obj| obj.remove("diff-review@claude_vibeless").is_some())
        .unwrap_or(false);

    if !changed {
        return;
    }

    if settings
        .get("enabledPlugins")
        .and_then(|p| p.as_object())
        .is_some_and(|obj| obj.is_empty())
    {
        settings.as_object_mut().unwrap().remove("enabledPlugins");
    }

    if settings.as_object().is_some_and(|obj| obj.is_empty()) {
        let _ = std::fs::remove_file(&settings_path);
    } else {
        let _ = std::fs::write(
            &settings_path,
            serde_json::to_string_pretty(&settings).unwrap_or_default() + "\n",
        );
    }
}

fn ensure_codex_notify_hook(workdir: &Path) {
    let codex_dir = workdir.join(".codex");
    let _ = std::fs::create_dir_all(&codex_dir);

    let hook_path = codex_dir.join("aiw-codex-notify.sh");
    let _ = std::fs::write(&hook_path, CODEX_NOTIFY_SH);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&hook_path, std::fs::Permissions::from_mode(0o755));
    }

    // FORK: no write to ~/.codex/config.toml; the hook is passed per launch
    // by `codex_config::launch_override_args`.

    let project_config_path = codex_dir.join("config.toml");
    if project_config_path.exists() {
        let mut config = std::fs::read_to_string(&project_config_path)
            .ok()
            .and_then(|s| toml::from_str::<toml::Value>(&s).ok())
            .unwrap_or_else(|| toml::Value::Table(toml::map::Map::new()));

        if let Some(table) = config.as_table_mut() {
            table.remove("notify");
            if table.is_empty() {
                let _ = std::fs::remove_file(&project_config_path);
            } else if let Ok(rendered) = toml::to_string_pretty(&config) {
                let _ = std::fs::write(&project_config_path, rendered + "\n");
            }
        }
    }
}

fn agent_skills_dir(workdir: &Path, agent: &AgentKind) -> Option<PathBuf> {
    match agent {
        AgentKind::Claude => Some(workdir.join(".claude").join("skills")),
        AgentKind::Codex => Some(workdir.join(".agents").join("skills")),
        AgentKind::Pi => None,
        // FORK: Gemini/Cursor read AGENTS.md/CLAUDE.md, not a skills dir.
        AgentKind::Gemini | AgentKind::Cursor => None,
    }
}

fn ensure_aiw_skills(workdir: &Path, agent: &AgentKind) {
    let Some(skills_dir) = agent_skills_dir(workdir, agent) else {
        return;
    };
    for (name, content) in AIW_SKILLS {
        let skill_dir = skills_dir.join(format!("aiw-{name}"));
        let skill_path = skill_dir.join("SKILL.md");
        if std::fs::read_to_string(&skill_path).ok().as_deref() == Some(content) {
            continue;
        }
        let _ = std::fs::create_dir_all(&skill_dir);
        let _ = std::fs::write(&skill_path, content);
    }
}

fn cleanup_aiw_skills(workdir: &Path, agent: &AgentKind) {
    let Some(skills_dir) = agent_skills_dir(workdir, agent) else {
        return;
    };
    for (name, _) in AIW_SKILLS {
        let _ = std::fs::remove_dir_all(skills_dir.join(format!("aiw-{name}")));
    }
}

fn cleanup_claude_notification_hooks(workdir: &Path) {
    let claude_dir = workdir.join(".claude");
    cleanup_claude_settings_file(
        &claude_dir.join(CLAUDE_SETTINGS_LOCAL_JSON),
        Some(&claude_dir.join(CLAUDE_STATE_JSON)),
    );
    cleanup_claude_settings_file(&claude_dir.join(CLAUDE_SETTINGS_JSON), None);

    let _ = std::fs::remove_file(claude_dir.join("latest-prompt.txt"));
    let _ = std::fs::remove_dir_all(claude_dir.join("notifications"));
    cleanup_aiw_skills(workdir, &AgentKind::Claude);
}

fn cleanup_codex_notify_hook(workdir: &Path) {
    // FORK: the hooks AIW merged into .codex/hooks.json go too.
    crate::fork::codex_hooks::cleanup_hooks_file(workdir, &crate::project::aiw_claude_hooks_dir());
    let codex_dir = workdir.join(".codex");
    let hook_path = codex_dir.join("aiw-codex-notify.sh");

    let _ = std::fs::remove_file(&hook_path);
    let project_config_path = codex_dir.join("config.toml");
    if project_config_path.exists() {
        let mut config = std::fs::read_to_string(&project_config_path)
            .ok()
            .and_then(|s| toml::from_str::<toml::Value>(&s).ok())
            .unwrap_or_else(|| toml::Value::Table(toml::map::Map::new()));

        if let Some(table) = config.as_table_mut() {
            table.remove("notify");
            if table.is_empty() {
                let _ = std::fs::remove_file(&project_config_path);
            } else if let Ok(rendered) = toml::to_string_pretty(&config) {
                let _ = std::fs::write(&project_config_path, rendered + "\n");
            }
        }
    }
    cleanup_aiw_skills(workdir, &AgentKind::Codex);
}

pub fn cleanup_agent_injected_files(workdir: &Path, agent: &AgentKind) {
    match agent {
        AgentKind::Claude => cleanup_claude_notification_hooks(workdir),
        AgentKind::Codex => cleanup_codex_notify_hook(workdir),
        AgentKind::Pi => {}
        // FORK: remove the Gemini/Cursor hooks AIW merged in.
        AgentKind::Gemini | AgentKind::Cursor => {
            crate::fork::extra_agents::cleanup_hooks(workdir, agent)
        }
    }
}

pub(crate) fn resolve_diff_review_command(workdir: &Path, repo: &Path) -> Option<String> {
    let script_name = "custom-diff-review.sh";
    let script_suffix = ["plugins", "diff-review", "scripts", script_name];
    let aiw_root = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent()?.parent()?.parent().map(PathBuf::from));

    let under_plugins = |base: PathBuf| script_suffix.iter().fold(base, |p, s| p.join(s));
    let mut candidates = vec![
        under_plugins(workdir.to_path_buf()),
        under_plugins(repo.to_path_buf()),
        crate::project::aiw_claude_hooks_dir().join(script_name),
    ];
    if let Some(aiw_root) = aiw_root {
        candidates.push(under_plugins(aiw_root));
    }

    candidates
        .into_iter()
        .find(|p| p.exists())
        .map(|p| p.to_string_lossy().into_owned())
}

pub fn ensure_notification_hooks(
    workdir: &Path,
    repo: &Path,
    mode: &VibeMode,
    agent: &AgentKind,
    is_worktree: bool,
) {
    // Feature creation / restart should not depend on startup having
    // already staged the helper scripts into the AIW runtime directory.
    ensure_notify_scripts();
    remove_old_diff_review_plugin(repo);
    // FORK: keep the helper files below out of commits (.git/info/exclude).
    crate::fork::git_exclude::exclude_aiw_helper_files(workdir);

    if matches!(agent, AgentKind::Codex) {
        ensure_codex_notify_hook(workdir);
        // FORK: Codex hooks (permission requests, …) in .codex/hooks.json.
        crate::fork::codex_hooks::ensure_hooks_file(
            workdir,
            &crate::project::aiw_claude_hooks_dir(),
        );
        ensure_aiw_skills(workdir, agent);
        return;
    }
    // FORK: Gemini/Cursor hooks merge into .gemini/settings.json and
    // .cursor/hooks.json (git-excluded); see src/fork/extra_agents.rs.
    if crate::fork::extra_agents::is_extra(agent) {
        crate::fork::extra_agents::ensure_hooks(workdir, agent);
        return;
    }

    let claude_dir = workdir.join(".claude");
    let settings_path = claude_dir.join(CLAUDE_SETTINGS_LOCAL_JSON);
    let state_path = claude_dir.join(CLAUDE_STATE_JSON);

    let hooks_dir = crate::project::aiw_claude_hooks_dir();
    let managed_commands = claude_managed_commands();
    repair_inherited_repo_claude_hooks(workdir, repo, is_worktree, &managed_commands);
    let wants_diff_review = matches!(mode, VibeMode::Vibeless);
    let diff_review_cmd = if wants_diff_review {
        resolve_diff_review_command(workdir, repo)
    } else {
        None
    };

    let mut settings = read_json_object(&settings_path);
    remove_aiw_claude_hooks(&mut settings, &managed_commands);

    // FORK: one builder for the hook entries (`fork::claude_settings`). With
    // `claude_hooks_via_settings_flag` they go into the per-session
    // `--settings` file at launch instead of this file, which then only
    // carries the user's own entries (DESIGN.md §16.6).
    if !crate::fork::claude_settings::via_settings_flag() {
        let entries =
            crate::fork::claude_settings::hook_entries(&crate::fork::claude_settings::HookInputs {
                hooks_dir: &hooks_dir,
                workdir,
                mode,
                diff_review_cmd: diff_review_cmd.as_deref(),
            });
        for (event, entry) in entries {
            push_claude_hook_entry(&mut settings, event, entry);
        }
    }

    update_claude_permissions(&mut settings, &state_path, wants_diff_review);

    let _ = std::fs::create_dir_all(&claude_dir);
    write_json_object(&settings_path, &settings);
    cleanup_claude_settings_file(&claude_dir.join(CLAUDE_SETTINGS_JSON), None);

    // Ensure notifications/ is gitignored within .claude/
    let claude_gitignore = claude_dir.join(".gitignore");
    ensure_gitignore_entry(&claude_gitignore, "notifications/");
    ensure_gitignore_entry(&claude_gitignore, "latest-prompt.txt");
    ensure_gitignore_entry(&claude_gitignore, "skills/aiw-*/");

    ensure_aiw_skills(workdir, agent);
}

#[cfg(test)]
mod atomic_write_tests {
    use super::*;
    use tempfile::TempDir;

    /// The staged temp file must not be left behind after a successful write —
    /// a lingering `.name.tmp.<pid>` would clutter the config dir.
    fn temp_files_in(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| name.contains(".tmp."))
            .collect()
    }

    #[test]
    fn write_atomic_writes_content_and_leaves_no_temp() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("hook.sh");

        write_atomic(&path, "echo hi\n", None).unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "echo hi\n");
        assert!(
            temp_files_in(dir.path()).is_empty(),
            "no staging temp file should remain"
        );
    }

    #[test]
    fn write_atomic_replaces_existing_content() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("hook.sh");
        std::fs::write(&path, "old\n").unwrap();

        write_atomic(&path, "new\n", None).unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new\n");
    }

    #[cfg(unix)]
    #[test]
    fn write_atomic_sets_executable_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("hook.sh");

        write_atomic(&path, "#!/bin/sh\n", Some(0o755)).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
    }

    #[test]
    fn write_executable_if_changed_skips_when_unchanged() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("hook.sh");
        write_executable_if_changed(&path, "same\n");
        let first = std::fs::metadata(&path).unwrap().modified().unwrap();

        // A second call with identical content must not rewrite the file.
        std::thread::sleep(std::time::Duration::from_millis(10));
        write_executable_if_changed(&path, "same\n");
        let second = std::fs::metadata(&path).unwrap().modified().unwrap();

        assert_eq!(first, second, "unchanged content should not be rewritten");
    }
}
