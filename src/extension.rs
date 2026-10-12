use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::project::{AgentKind, StoredVibeMode, VibeMode};
use crate::prompt_library::PromptTemplate;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CustomSessionConfig {
    pub name: String,
    pub description: Option<String>,
    pub command: Option<String>,
    pub window_name: Option<String>,
    pub working_dir: Option<PathBuf>,
    pub icon: Option<String>,
    pub icon_nerd: Option<String>,
    pub on_stop: Option<String>,
    pub autolaunch: Option<bool>,
    pub pre_check: Option<String>,
}

impl CustomSessionConfig {
    /// Run the `pre_check` command (if any) and return
    /// `Ok(())` on success or `Err(message)` with the
    /// command output when it fails / is not found.
    pub fn run_pre_check(&self, workdir: &std::path::Path) -> std::result::Result<(), String> {
        let cmd = match &self.pre_check {
            Some(c) if !c.is_empty() => c,
            _ => return Ok(()),
        };
        match std::process::Command::new("bash")
            .arg("-c")
            .arg(cmd)
            .current_dir(workdir)
            .output()
        {
            Ok(output) if output.status.success() => Ok(()),
            Ok(output) => {
                let msg = String::from_utf8_lossy(&output.stdout);
                let err = String::from_utf8_lossy(&output.stderr);
                let combined = if !msg.is_empty() && !err.is_empty() {
                    format!("{}\n{}", msg.trim(), err.trim())
                } else if !msg.is_empty() {
                    msg.trim().to_string()
                } else if !err.is_empty() {
                    err.trim().to_string()
                } else {
                    format!("pre_check failed (exit {})", output.status)
                };
                Err(combined)
            }
            Err(e) => Err(format!("pre_check failed to run: {}", e)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookPrompt {
    pub title: String,
    pub options: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum HookConfig {
    Script(String),
    WithPrompt { script: String, prompt: HookPrompt },
}

impl HookConfig {
    pub fn script(&self) -> &str {
        match self {
            HookConfig::Script(s) => s,
            HookConfig::WithPrompt { script, .. } => script,
        }
    }

    pub fn prompt(&self) -> Option<&HookPrompt> {
        match self {
            HookConfig::Script(_) => None,
            HookConfig::WithPrompt { prompt, .. } => Some(prompt),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct LifecycleHooks {
    pub on_start: Option<HookConfig>,
    pub on_stop: Option<HookConfig>,
    pub on_worktree_created: Option<HookConfig>,
    /// FORK: §19.10 — runs in the feature's worktree before a land; a
    /// non-zero exit stops the land.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre_land: Option<HookConfig>,
    /// FORK: §19.10 — runs in the repository when a land ends (either way),
    /// with `AIW_LAND_RESULT`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_land: Option<HookConfig>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(default)]
pub struct FeaturePreset {
    pub name: String,
    pub branch_prefix: Option<String>,
    pub mode: VibeMode,
    pub agent: AgentKind,
    pub enable_chrome: bool,
    /// FORK: §19.3 — start several harnesses (first = primary). Empty means
    /// `agent` alone.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub agents: Vec<AgentKind>,
    /// FORK: §19.3 — how several agents are laid out.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fan_out: Option<crate::fork::fan_out::FanOutLayout>,
    /// FORK: §19.4 — the layout preset (by name) opened with the feature.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default)]
struct FeaturePresetDe {
    name: String,
    branch_prefix: Option<String>,
    mode: StoredVibeMode,
    agent: AgentKind,
    enable_chrome: bool,
    agents: Vec<AgentKind>,
    fan_out: Option<crate::fork::fan_out::FanOutLayout>,
    layout: Option<String>,
}

impl<'de> Deserialize<'de> for FeaturePreset {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let preset = FeaturePresetDe::deserialize(deserializer)?;
        let (mode, _legacy_review) = preset.mode.into_mode_and_review();
        Ok(Self {
            name: preset.name,
            branch_prefix: preset.branch_prefix,
            mode,
            agent: preset.agent,
            enable_chrome: preset.enable_chrome,
            agents: preset.agents,
            fan_out: preset.fan_out,
            layout: preset.layout,
        })
    }
}

impl FeaturePreset {
    pub fn normalize_legacy_review_mode(&mut self) -> bool {
        false
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ExtensionConfig {
    pub custom_sessions: Vec<CustomSessionConfig>,
    pub lifecycle_hooks: LifecycleHooks,
    pub keybindings: HashMap<String, char>,
    pub feature_presets: Vec<FeaturePreset>,
    // FORK: a legacy "opencode" entry is dropped rather than read as Claude.
    #[serde(deserialize_with = "crate::project::deserialize_allowed_agents")]
    pub allowed_agents: Option<Vec<AgentKind>>,
    /// Declarative prompt-library templates (read-only at load time).
    /// The library view can export user templates here. Phase 3 surfaces
    /// these in the picker with a `Global` / `Project` source badge.
    pub prompt_templates: Vec<PromptTemplate>,
    /// FORK: §19.4 — named sets of extra sessions opened with a feature.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub layout_presets: Vec<crate::fork::organize::LayoutPreset>,
    /// FORK: §19.9 — recurring prompts and commands for matching features.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub schedules: Vec<crate::fork::schedules::ScheduleConfig>,
    /// FORK: §19.10 — how `P` lands a feature (project over global, field
    /// by field).
    #[serde(skip_serializing_if = "crate::fork::land::LandConfig::is_empty")]
    pub land: crate::fork::land::LandConfig,
}

impl ExtensionConfig {
    pub fn allowed_agents(&self) -> Vec<AgentKind> {
        AgentKind::allowed_list(self.allowed_agents.as_deref())
    }

    pub fn allows_agent(&self, agent: &AgentKind) -> bool {
        self.allowed_agents().contains(agent)
    }

    /// FORK: §19.4 — a layout preset by name.
    pub fn layout_preset(&self, name: &str) -> Option<&crate::fork::organize::LayoutPreset> {
        self.layout_presets.iter().find(|p| p.name == name)
    }

    pub fn allowed_feature_presets(&self) -> Vec<FeaturePreset> {
        self.feature_presets
            .iter()
            .filter(|preset| {
                self.allows_agent(&preset.agent)
                    && preset.agents.iter().all(|agent| self.allows_agent(agent))
            })
            .cloned()
            .collect()
    }

    fn normalize_legacy_review_modes(&mut self) {
        for preset in &mut self.feature_presets {
            preset.normalize_legacy_review_mode();
        }
    }
}

/// Thin wrapper used only for deserializing the
/// `extension` field out of the global config file.
#[derive(Debug, Deserialize, Default)]
#[serde(default)]
struct GlobalConfigPartial {
    extension: ExtensionConfig,
}

#[cfg(test)]
thread_local! {
    // Lets tests stand in for the developer's real `~/.config/ai_workstation/config.json`
    // so assertions don't depend on what happens to be on the machine running
    // them. Thread-local (not a global static) because the test harness reuses
    // OS threads across tests, so each test must set this explicitly before
    // relying on it rather than assuming a clean slate.
    static TEST_GLOBAL_EXTENSION_CONFIG: std::cell::RefCell<Option<ExtensionConfig>> =
        const { std::cell::RefCell::new(None) };
}

/// Test-only seam for [`load_global_extension_config`]: forces the given
/// config (or clears the override with `None`) instead of reading
/// `~/.config/ai_workstation/config.json` for the rest of the calling thread's tests.
#[cfg(test)]
pub fn set_test_global_extension_config(config: Option<ExtensionConfig>) {
    TEST_GLOBAL_EXTENSION_CONFIG.with(|cell| *cell.borrow_mut() = config);
}

/// Load the `extension` block from
/// `~/.config/ai_workstation/config.json`.
/// Returns a default (empty) config on any failure.
pub fn load_global_extension_config() -> ExtensionConfig {
    #[cfg(test)]
    if let Some(config) = TEST_GLOBAL_EXTENSION_CONFIG.with(|cell| cell.borrow().clone()) {
        return config;
    }

    let config_path = crate::project::aiw_config_dir().join("config.json");

    if !config_path.exists() {
        return ExtensionConfig::default();
    }

    let mut config = std::fs::read_to_string(&config_path)
        .ok()
        .and_then(|s| serde_json::from_str::<GlobalConfigPartial>(&s).ok())
        .map(|c| c.extension)
        .unwrap_or_default();
    config.normalize_legacy_review_modes();
    config
}

/// Explains the directory to anyone who opens it wondering what it is —
/// and where their own settings belong instead.
const GENERATED_AIW_README: &str = "\
# `.aiw/` — generated by AIW

Everything in this directory is **generated at runtime** by AIW
(AI Workstation): session status files, plan-interview reference copies
and similar scratch data. None of it is hand-authored, none of it belongs
in a commit, and deleting the whole directory is safe — AIW recreates
what it needs.

It is gitignored dir-wide for that reason. Do not put settings here: a
config file dropped in this directory is silently untracked.

## Where project config goes

Hand-authored AIW config for this project lives at the repo root:

    aiw.json

That file is tracked and committed like any other project config. It
holds custom sessions, feature presets, lifecycle hooks, keybindings,
plan questions, and prompt templates.

This file is regenerated if deleted.
";

/// Create `{workdir}/.aiw/<name>`, dropping a README in `.aiw/` that says
/// the directory is generated and points at `aiw.json` for real config.
///
/// Returns the subdirectory path whether or not creation succeeded — the
/// callers treat a missing status directory as a soft failure, and a
/// README is never worth failing a session launch over.
pub fn generated_aiw_subdir(workdir: &Path, name: &str) -> PathBuf {
    let aiw_dir = workdir.join(".aiw");
    let subdir = aiw_dir.join(name);
    if std::fs::create_dir_all(&subdir).is_ok() {
        let readme = aiw_dir.join("README.md");
        if !readme.exists() {
            let _ = std::fs::write(&readme, GENERATED_AIW_README);
        }
    }
    subdir
}

/// The tracked, hand-authored project config: `{dir}/aiw.json`.
///
/// Config lives at the repo root, **not** inside `.aiw/`: everything in
/// that directory is generated, so it is gitignored dir-wide with no
/// exceptions. All writes target this path.
pub fn project_config_path(dir: &Path) -> PathBuf {
    dir.join("aiw.json")
}

/// The config file to read for `dir`: `aiw.json` when it exists. `None`
/// when there is none, so callers can distinguish "no config" from "empty
/// config".
pub fn resolve_project_config_path(dir: &Path) -> Option<PathBuf> {
    let current = project_config_path(dir);
    current.exists().then_some(current)
}

/// Write `json` to `{dir}/aiw.json`.
///
/// The new config is written to a sibling temp file and renamed into
/// place, so `aiw.json` either does not exist or holds a complete config
/// — never a truncated one.
pub fn write_project_config(dir: &Path, json: &str) -> anyhow::Result<PathBuf> {
    let path = project_config_path(dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Same directory as the target, so the rename is within one filesystem.
    let tmp = path.with_extension(format!("json.{}.tmp", uuid::Uuid::new_v4()));
    if let Err(err) = std::fs::write(&tmp, json) {
        let _ = std::fs::remove_file(&tmp);
        return Err(anyhow::Error::new(err).context(format!("failed to write {}", tmp.display())));
    }
    if let Err(err) = std::fs::rename(&tmp, &path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(
            anyhow::Error::new(err).context(format!("failed to replace {}", path.display()))
        );
    }
    Ok(path)
}

/// Load the project config (see `resolve_project_config_path`) and merge it onto
/// `base` according to the plan merge rules:
/// - custom_sessions: project appends; name collision →
///   project wins
/// - feature_presets: same rules
/// - lifecycle_hooks: project fields override global
/// - keybindings: project overrides global per-action
pub fn merge_project_extension_config(base: &ExtensionConfig, repo: &Path) -> ExtensionConfig {
    merge_project_extension_config_checked(base, repo).0
}

/// FORK: why a project config could not be used. Until the file is fixed the
/// project runs on the global config alone: no hooks, no custom sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectConfigError {
    pub path: PathBuf,
    /// serde's message, which ends in `at line N column M` for syntax
    /// errors, or the I/O error text.
    pub detail: String,
}

impl ProjectConfigError {
    /// The one-line warning shown for `project` (its display name).
    pub fn user_message(&self, project: &str) -> String {
        let file = self
            .path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_else(|| "aiw.json".to_string());
        format!(
            "{file} in {project} is invalid: {}; hooks and custom sessions are disabled until fixed",
            self.detail
        )
    }
}

/// FORK: read `repo`'s project config: `Ok(None)` when there is none,
/// `Err` when there is one that cannot be read or parsed.
pub fn read_project_extension_config(
    repo: &Path,
) -> Result<Option<ExtensionConfig>, ProjectConfigError> {
    let Some(path) = resolve_project_config_path(repo) else {
        return Ok(None);
    };
    let raw = std::fs::read_to_string(&path).map_err(|err| ProjectConfigError {
        path: path.clone(),
        detail: err.to_string(),
    })?;
    serde_json::from_str::<ExtensionConfig>(&raw)
        .map(Some)
        .map_err(|err| ProjectConfigError {
            path,
            detail: err.to_string(),
        })
}

/// [`merge_project_extension_config`], also returning why the project's own
/// config was ignored, when it was. FORK: callers that can talk to the user
/// surface the error instead of silently running without hooks.
pub fn merge_project_extension_config_checked(
    base: &ExtensionConfig,
    repo: &Path,
) -> (ExtensionConfig, Option<ProjectConfigError>) {
    let (project, error): (Option<ExtensionConfig>, Option<ProjectConfigError>) =
        match read_project_extension_config(repo) {
            Ok(project) => (project, None),
            Err(err) => (Some(ExtensionConfig::default()), Some(err)),
        };
    let Some(project) = project else {
        return (base.clone(), None);
    };

    // Merge custom_sessions: start with project, then
    // append global entries whose name doesn't collide.
    let mut custom_sessions = project.custom_sessions.clone();
    for entry in &base.custom_sessions {
        if !custom_sessions.iter().any(|e| e.name == entry.name) {
            custom_sessions.push(entry.clone());
        }
    }

    // Merge feature_presets: same strategy.
    let mut feature_presets = project.feature_presets.clone();
    for entry in &base.feature_presets {
        if !feature_presets.iter().any(|e| e.name == entry.name) {
            feature_presets.push(entry.clone());
        }
    }

    // Merge prompt_templates by name (project wins).
    let mut prompt_templates = project.prompt_templates.clone();
    for entry in &base.prompt_templates {
        if !prompt_templates.iter().any(|e| e.name == entry.name) {
            prompt_templates.push(entry.clone());
        }
    }

    // FORK: §19.4 — layout presets by name (project wins).
    let mut layout_presets = project.layout_presets.clone();
    for entry in &base.layout_presets {
        if !layout_presets.iter().any(|e| e.name == entry.name) {
            layout_presets.push(entry.clone());
        }
    }

    // FORK: §19.9 — schedules by name (project wins).
    let mut schedules = project.schedules.clone();
    for entry in &base.schedules {
        if !schedules.iter().any(|e| e.name == entry.name) {
            schedules.push(entry.clone());
        }
    }

    // Merge lifecycle_hooks: project fields take priority.
    let on_start = project
        .lifecycle_hooks
        .on_start
        .clone()
        .or_else(|| base.lifecycle_hooks.on_start.clone());
    let on_stop = project
        .lifecycle_hooks
        .on_stop
        .clone()
        .or_else(|| base.lifecycle_hooks.on_stop.clone());
    let on_worktree_created = project
        .lifecycle_hooks
        .on_worktree_created
        .clone()
        .or_else(|| base.lifecycle_hooks.on_worktree_created.clone());
    // FORK: §19.10
    let pre_land = project
        .lifecycle_hooks
        .pre_land
        .clone()
        .or_else(|| base.lifecycle_hooks.pre_land.clone());
    let post_land = project
        .lifecycle_hooks
        .post_land
        .clone()
        .or_else(|| base.lifecycle_hooks.post_land.clone());

    let mut keybindings = base.keybindings.clone();
    for (action, key) in &project.keybindings {
        keybindings.insert(action.clone(), *key);
    }

    let mut merged = ExtensionConfig {
        custom_sessions,
        lifecycle_hooks: LifecycleHooks {
            on_start,
            on_stop,
            on_worktree_created,
            pre_land,
            post_land,
        },
        keybindings,
        feature_presets,
        allowed_agents: project
            .allowed_agents
            .clone()
            .or_else(|| base.allowed_agents.clone()),
        prompt_templates,
        layout_presets,
        schedules,
        land: project.land.merged_over(&base.land),
    };
    merged.normalize_legacy_review_modes();
    (merged, error)
}

/// Write an `ExtensionConfig` to `{repo}/aiw.json`.
pub fn save_project_extension_config(repo: &Path, config: &ExtensionConfig) -> anyhow::Result<()> {
    let json = serde_json::to_string_pretty(config)?;
    write_project_config(repo, &json)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tempfile::TempDir;

    // FORK: §19.4 — layout presets merge by name (project wins) and a
    // feature preset keeps its layout name.
    #[test]
    fn layout_presets_merge_by_name_and_presets_keep_their_layout() {
        use crate::fork::organize::{LayoutPreset, LayoutSessionSpec};
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join("aiw.json"),
            r#"{
                "layout_presets": [
                    { "name": "dev", "sessions": [ { "kind": "terminal", "command": "make dev" } ] }
                ],
                "feature_presets": [ { "name": "p", "agent": "claude", "layout": "dev" } ]
            }"#,
        )
        .unwrap();
        let spec = |kind: &str| LayoutSessionSpec {
            kind: kind.into(),
            ..Default::default()
        };
        let global = ExtensionConfig {
            layout_presets: vec![
                LayoutPreset {
                    name: "dev".into(),
                    description: None,
                    sessions: vec![spec("nvim")],
                },
                LayoutPreset {
                    name: "ops".into(),
                    description: None,
                    sessions: vec![spec("terminal")],
                },
            ],
            ..Default::default()
        };
        let merged = merge_project_extension_config(&global, dir.path());
        let names: Vec<&str> = merged
            .layout_presets
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert_eq!(names, ["dev", "ops"]);
        assert_eq!(
            merged.layout_preset("dev").unwrap().sessions[0]
                .command
                .as_deref(),
            Some("make dev"),
            "the project's dev wins"
        );
        assert_eq!(merged.feature_presets[0].layout.as_deref(), Some("dev"));
        // Saving keeps them; an empty list is not written.
        let json = serde_json::to_value(&merged).unwrap();
        assert_eq!(json["layout_presets"][1]["name"], "ops");
        let empty = serde_json::to_value(ExtensionConfig::default()).unwrap();
        assert!(empty.get("layout_presets").is_none());
    }

    #[test]
    fn a_broken_aiw_json_is_reported_with_its_position_and_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("aiw.json"),
            "{\n  \"lifecycle_hooks\": { \"on_start\": \n",
        )
        .unwrap();
        let mut global = ExtensionConfig::default();
        global.custom_sessions.push(CustomSessionConfig {
            name: "global-logs".to_string(),
            ..Default::default()
        });

        let (merged, error) = merge_project_extension_config_checked(&global, tmp.path());

        let error = error.expect("the parse error is returned");
        assert_eq!(error.path, tmp.path().join("aiw.json"));
        assert!(error.detail.contains("line 3"), "{}", error.detail);
        assert!(error.detail.contains("column"), "{}", error.detail);
        let message = error.user_message("gamma");
        assert!(
            message.starts_with("aiw.json in gamma is invalid: "),
            "{message}"
        );
        assert!(message.ends_with("hooks and custom sessions are disabled until fixed"));
        // The project half is empty, the global half still applies.
        assert!(merged.lifecycle_hooks.on_start.is_none());
        assert_eq!(merged.custom_sessions.len(), 1);
        // The unchecked entry point keeps its old shape.
        assert_eq!(
            merge_project_extension_config(&global, tmp.path())
                .custom_sessions
                .len(),
            1
        );
    }

    #[test]
    fn a_missing_or_valid_aiw_json_reports_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(read_project_extension_config(tmp.path()).unwrap().is_none());
        assert!(
            merge_project_extension_config_checked(&ExtensionConfig::default(), tmp.path())
                .1
                .is_none()
        );

        std::fs::write(tmp.path().join("aiw.json"), "{}").unwrap();
        assert!(read_project_extension_config(tmp.path()).unwrap().is_some());
        assert!(
            merge_project_extension_config_checked(&ExtensionConfig::default(), tmp.path())
                .1
                .is_none()
        );
    }

    #[test]
    fn resolve_project_config_finds_aiw_json_only() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(resolve_project_config_path(tmp.path()), None);
        let current = project_config_path(tmp.path());
        std::fs::write(&current, "{}").unwrap();
        assert_eq!(resolve_project_config_path(tmp.path()), Some(current));
    }

    #[test]
    fn write_project_config_writes_aiw_json() {
        let tmp = TempDir::new().unwrap();
        let written = write_project_config(tmp.path(), "{}").unwrap();
        assert_eq!(written, project_config_path(tmp.path()));
        assert!(written.exists());
    }

    #[test]
    fn write_project_config_leaves_no_temp_files_behind() {
        let tmp = TempDir::new().unwrap();
        write_project_config(tmp.path(), "{}").unwrap();

        let strays: Vec<_> = std::fs::read_dir(tmp.path())
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.file_name().to_string_lossy().into_owned()))
            .filter(|name| name != "aiw.json")
            .collect();
        assert!(strays.is_empty(), "unexpected leftovers: {strays:?}");
    }

    #[test]
    fn generated_aiw_subdir_drops_a_readme_pointing_at_aiw_json() {
        let tmp = TempDir::new().unwrap();
        let dir = generated_aiw_subdir(tmp.path(), "session-status");
        assert!(dir.is_dir());

        let readme = tmp.path().join(".aiw").join("README.md");
        let body = std::fs::read_to_string(&readme).unwrap();
        assert!(
            body.contains("aiw.json"),
            "README should name the config path"
        );

        // A user's edits to the README survive the next launch.
        std::fs::write(&readme, "edited").unwrap();
        generated_aiw_subdir(tmp.path(), "session-status");
        assert_eq!(std::fs::read_to_string(&readme).unwrap(), "edited");
    }

    fn write_extension_config(dir: &TempDir, config: &ExtensionConfig) {
        let json = serde_json::to_string(config).unwrap();
        std::fs::write(project_config_path(dir.path()), json).unwrap();
    }

    // ── merge_project_extension_config ────────────────────────

    #[test]
    fn no_project_config_returns_base_unchanged() {
        let global = ExtensionConfig {
            custom_sessions: vec![CustomSessionConfig {
                name: "test".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };
        let tmp = TempDir::new().unwrap(); // no project config at all
        let merged = merge_project_extension_config(&global, tmp.path());
        assert_eq!(merged.custom_sessions.len(), 1);
        assert_eq!(merged.custom_sessions[0].name, "test");
    }

    #[test]
    fn project_hook_overrides_global_hook() {
        let global = ExtensionConfig {
            lifecycle_hooks: LifecycleHooks {
                on_start: Some(HookConfig::Script("global-start.sh".to_string())),
                ..Default::default()
            },
            ..Default::default()
        };
        let project_config = ExtensionConfig {
            lifecycle_hooks: LifecycleHooks {
                on_start: Some(HookConfig::Script("project-start.sh".to_string())),
                ..Default::default()
            },
            ..Default::default()
        };
        let tmp = TempDir::new().unwrap();
        write_extension_config(&tmp, &project_config);

        let merged = merge_project_extension_config(&global, tmp.path());
        let on_start = merged.lifecycle_hooks.on_start.unwrap();
        assert_eq!(on_start.script(), "project-start.sh");
    }

    #[test]
    fn global_hook_used_when_project_does_not_set_it() {
        let global = ExtensionConfig {
            lifecycle_hooks: LifecycleHooks {
                on_start: Some(HookConfig::Script("global-start.sh".to_string())),
                ..Default::default()
            },
            ..Default::default()
        };
        // Project config present but no on_start
        let project_config = ExtensionConfig::default();
        let tmp = TempDir::new().unwrap();
        write_extension_config(&tmp, &project_config);

        let merged = merge_project_extension_config(&global, tmp.path());
        let on_start = merged.lifecycle_hooks.on_start.unwrap();
        assert_eq!(on_start.script(), "global-start.sh");
    }

    #[test]
    fn custom_sessions_deduplicated_by_name_project_wins() {
        let global = ExtensionConfig {
            custom_sessions: vec![
                CustomSessionConfig {
                    name: "shared".to_string(),
                    command: Some("global-cmd".to_string()),
                    ..Default::default()
                },
                CustomSessionConfig {
                    name: "global-only".to_string(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let project_config = ExtensionConfig {
            custom_sessions: vec![
                CustomSessionConfig {
                    name: "shared".to_string(),
                    command: Some("project-cmd".to_string()),
                    ..Default::default()
                },
                CustomSessionConfig {
                    name: "project-only".to_string(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let tmp = TempDir::new().unwrap();
        write_extension_config(&tmp, &project_config);

        let merged = merge_project_extension_config(&global, tmp.path());
        // "shared" must appear exactly once (project version)
        let shared: Vec<_> = merged
            .custom_sessions
            .iter()
            .filter(|s| s.name == "shared")
            .collect();
        assert_eq!(shared.len(), 1);
        assert_eq!(shared[0].command.as_deref(), Some("project-cmd"));
        // Both unique entries should be present
        assert!(
            merged
                .custom_sessions
                .iter()
                .any(|s| s.name == "global-only")
        );
        assert!(
            merged
                .custom_sessions
                .iter()
                .any(|s| s.name == "project-only")
        );
        assert_eq!(merged.custom_sessions.len(), 3);
    }

    #[test]
    fn prompt_templates_merged_by_name_project_wins() {
        let global = ExtensionConfig {
            prompt_templates: vec![
                PromptTemplate::new("shared".to_string(), "global body".to_string()),
                PromptTemplate::new("global-only".to_string(), "g".to_string()),
            ],
            ..Default::default()
        };
        let project_config = ExtensionConfig {
            prompt_templates: vec![
                PromptTemplate::new("shared".to_string(), "project body".to_string()),
                PromptTemplate::new("project-only".to_string(), "p".to_string()),
            ],
            ..Default::default()
        };
        let tmp = TempDir::new().unwrap();
        write_extension_config(&tmp, &project_config);

        let merged = merge_project_extension_config(&global, tmp.path());
        // "shared" must appear exactly once (project version).
        let shared: Vec<_> = merged
            .prompt_templates
            .iter()
            .filter(|t| t.name == "shared")
            .collect();
        assert_eq!(shared.len(), 1);
        assert_eq!(shared[0].body, "project body");
        // Both unique entries should be present.
        assert!(
            merged
                .prompt_templates
                .iter()
                .any(|t| t.name == "global-only")
        );
        assert!(
            merged
                .prompt_templates
                .iter()
                .any(|t| t.name == "project-only")
        );
        assert_eq!(merged.prompt_templates.len(), 3);
    }

    #[test]
    fn keybindings_project_overrides_per_action() {
        let mut global_bindings = HashMap::new();
        global_bindings.insert("quit".to_string(), 'q');
        global_bindings.insert("delete".to_string(), 'd');

        let global = ExtensionConfig {
            keybindings: global_bindings,
            ..Default::default()
        };
        let mut project_bindings = HashMap::new();
        // Override quit only
        project_bindings.insert("quit".to_string(), 'Q');

        let project_config = ExtensionConfig {
            keybindings: project_bindings,
            ..Default::default()
        };
        let tmp = TempDir::new().unwrap();
        write_extension_config(&tmp, &project_config);

        let merged = merge_project_extension_config(&global, tmp.path());
        assert_eq!(merged.keybindings.get("quit"), Some(&'Q'));
        // Global key preserved when not overridden
        assert_eq!(merged.keybindings.get("delete"), Some(&'d'));
    }

    #[test]
    fn project_allowed_agents_override_global_allowed_agents() {
        let global = ExtensionConfig {
            allowed_agents: Some(vec![AgentKind::Claude]),
            ..Default::default()
        };
        let project_config = ExtensionConfig {
            allowed_agents: Some(vec![AgentKind::Pi]),
            ..Default::default()
        };
        let tmp = TempDir::new().unwrap();
        write_extension_config(&tmp, &project_config);

        let merged = merge_project_extension_config(&global, tmp.path());
        assert_eq!(merged.allowed_agents(), vec![AgentKind::Pi]);
    }

    #[test]
    fn empty_allowed_agents_means_allow_all() {
        let config = ExtensionConfig {
            allowed_agents: Some(vec![]),
            ..Default::default()
        };

        assert_eq!(config.allowed_agents(), AgentKind::ALL.to_vec());
    }

    #[test]
    fn merge_normalizes_legacy_review_preset_mode() {
        let global = ExtensionConfig::default();
        let tmp = TempDir::new().unwrap();
        let raw = r#"{
            "feature_presets": [
                {
                    "name": "review-preset",
                    "mode": "review",
                    "agent": "claude",
                    "review": false,
                    "enable_chrome": false
                }
            ]
        }"#;
        std::fs::write(project_config_path(tmp.path()), raw).unwrap();

        let merged = merge_project_extension_config(&global, tmp.path());
        assert_eq!(merged.feature_presets.len(), 1);
        assert_eq!(merged.feature_presets[0].mode, VibeMode::Vibeless);
    }

    #[test]
    fn save_project_extension_config_writes_raw_extension_json() {
        let tmp = TempDir::new().unwrap();
        let config = ExtensionConfig {
            custom_sessions: vec![CustomSessionConfig {
                name: "lint".to_string(),
                ..Default::default()
            }],
            ..Default::default()
        };

        save_project_extension_config(tmp.path(), &config).unwrap();

        let raw = std::fs::read_to_string(project_config_path(tmp.path())).unwrap();
        let loaded: ExtensionConfig = serde_json::from_str(&raw).unwrap();
        assert_eq!(loaded.custom_sessions.len(), 1);
        assert_eq!(loaded.custom_sessions[0].name, "lint");
    }
}
