use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::project::{AgentKind, VibeMode};

pub const AUTOMATION_REQUEST_TYPE: &str = "automation";
pub const AUTOMATION_RESULT_TYPE: &str = "automation-result";
pub const CREATE_PROJECT_ACTION: &str = "create_project";
pub const CREATE_FEATURE_ACTION: &str = "create_feature";
pub const CREATE_BATCH_FEATURES_ACTION: &str = "create_batch_features";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CreateProjectRequest {
    pub path: PathBuf,
    pub project_name: String,
    pub preferred_agent: Option<AgentKind>,
    pub dry_run: bool,
}

impl Default for CreateProjectRequest {
    fn default() -> Self {
        Self {
            path: PathBuf::new(),
            project_name: String::new(),
            preferred_agent: None,
            dry_run: false,
        }
    }
}

impl CreateProjectRequest {
    pub fn ipc_payload(&self) -> serde_json::Value {
        serde_json::json!({
            "type": AUTOMATION_REQUEST_TYPE,
            "action": CREATE_PROJECT_ACTION,
            "path": self.path,
            "project_name": self.project_name,
            "preferred_agent": self.preferred_agent,
            "dry_run": self.dry_run,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CreateFeatureRequest {
    pub project_name: String,
    pub branch: String,
    pub agent: AgentKind,
    pub mode: VibeMode,
    pub create_terminal: bool,
    pub use_worktree: Option<bool>,
    pub enable_chrome: bool,
    pub hook_choice: Option<String>,
    pub dry_run: bool,
    /// FORK: §19.3 — several harnesses, first = primary. Empty means
    /// `agent` alone (the one-element form).
    pub agents: Vec<AgentKind>,
    /// FORK: §19.3 — `"shared"` (a window per agent) or `"worktree_each"`
    /// (sibling features `<branch>-<agent>`). Defaults to `worktree_each`
    /// when the feature gets a worktree, else `shared`.
    pub fan_out: Option<crate::fork::fan_out::FanOutLayout>,
    /// FORK: §19.3 — the task prompt every agent receives once its pane is
    /// ready, pasted unsent unless `send` is true.
    pub prompt: Option<String>,
    /// FORK: §19.3 — read `prompt` from this file (the CLI reads it).
    pub prompt_file: Option<PathBuf>,
    /// FORK: §19.3 — press Enter after the paste, once per agent.
    pub send: bool,
    /// FORK: §19.4 — a `layout_presets` entry (by name) opened in each
    /// created feature once it runs.
    pub layout: Option<String>,
}

impl CreateFeatureRequest {
    pub fn ipc_payload(&self) -> serde_json::Value {
        serde_json::json!({
            "type": AUTOMATION_REQUEST_TYPE,
            "action": CREATE_FEATURE_ACTION,
            "project_name": self.project_name,
            "branch": self.branch,
            "agent": self.agent,
            "mode": self.mode,
            "create_terminal": self.create_terminal,
            "use_worktree": self.use_worktree,
            "enable_chrome": self.enable_chrome,
            "hook_choice": self.hook_choice,
            "dry_run": self.dry_run,
            "agents": self.agents,
            "fan_out": self.fan_out,
            "prompt": self.prompt,
            "send": self.send,
            "layout": self.layout,
        })
    }

    /// FORK: §19.3 — the harnesses to start, primary first.
    pub fn agent_list(&self) -> Vec<AgentKind> {
        if self.agents.is_empty() {
            vec![self.agent.clone()]
        } else {
            self.agents.clone()
        }
    }
}

/// FORK: §19.3 — one feature a fan-out created.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct FanOutFeature {
    pub branch: String,
    pub agent: AgentKind,
    pub workdir: PathBuf,
    pub tmux_session: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feature_id: Option<String>,
    /// The agent sessions in it (label, session id).
    pub sessions: Vec<(String, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CreateBatchFeaturesRequest {
    pub workspace_path: PathBuf,
    pub project_name: String,
    pub feature_count: usize,
    pub feature_prefix: String,
    pub agent: AgentKind,
    pub mode: VibeMode,
    pub enable_chrome: bool,
    pub dry_run: bool,
}

impl Default for CreateBatchFeaturesRequest {
    fn default() -> Self {
        Self {
            workspace_path: PathBuf::new(),
            project_name: String::new(),
            feature_count: 3,
            feature_prefix: "feature".to_string(),
            agent: AgentKind::default(),
            mode: VibeMode::default(),
            enable_chrome: false,
            dry_run: false,
        }
    }
}

impl CreateBatchFeaturesRequest {
    pub fn ipc_payload(&self) -> serde_json::Value {
        serde_json::json!({
            "type": AUTOMATION_REQUEST_TYPE,
            "action": CREATE_BATCH_FEATURES_ACTION,
            "workspace_path": self.workspace_path,
            "project_name": self.project_name,
            "feature_count": self.feature_count,
            "feature_prefix": self.feature_prefix,
            "agent": self.agent,
            "mode": self.mode,
            "enable_chrome": self.enable_chrome,
            "dry_run": self.dry_run,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateProjectResponse {
    #[serde(rename = "type")]
    pub msg_type: &'static str,
    pub action: &'static str,
    pub ok: bool,
    pub dry_run: bool,
    pub input_path: PathBuf,
    pub project_name: String,
    pub project_path: PathBuf,
    pub is_git: bool,
    pub message: String,
    /// The created project's stable id, so a caller can address it by id
    /// from here on instead of re-deriving a name lookup. `None` for a
    /// `dry_run` response, since no project (and so no id) was created.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}

impl CreateProjectResponse {
    pub fn success(
        request: &CreateProjectRequest,
        project_path: PathBuf,
        is_git: bool,
        message: String,
        project_id: Option<String>,
    ) -> Self {
        Self {
            msg_type: AUTOMATION_RESULT_TYPE,
            action: CREATE_PROJECT_ACTION,
            ok: true,
            dry_run: request.dry_run,
            input_path: request.path.clone(),
            project_name: request.project_name.clone(),
            project_path,
            is_git,
            message,
            project_id,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AutomationHookPrompt {
    pub title: String,
    pub options: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateFeatureResponse {
    #[serde(rename = "type")]
    pub msg_type: &'static str,
    pub action: &'static str,
    pub ok: bool,
    pub dry_run: bool,
    pub project_name: String,
    pub branch: String,
    pub workdir: PathBuf,
    pub is_worktree: bool,
    pub tmux_session: String,
    pub started: bool,
    pub worktree_hook_ran: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_hook_succeeded: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_hook_prompt: Option<AutomationHookPrompt>,
    pub message: String,
    /// The created feature's (and its project's) stable id. `None` for a
    /// `dry_run` response, since no feature was created.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub feature_id: Option<String>,
    /// FORK: §19.3 — every feature a fan-out created (or would create, for a
    /// dry run), in order. Empty for a single-agent request.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<FanOutFeature>,
}

impl CreateFeatureResponse {
    #[allow(clippy::too_many_arguments)]
    pub fn success(
        request: &CreateFeatureRequest,
        workdir: PathBuf,
        is_worktree: bool,
        tmux_session: String,
        started: bool,
        worktree_hook_ran: bool,
        worktree_hook_succeeded: Option<bool>,
        worktree_hook_prompt: Option<AutomationHookPrompt>,
        message: String,
        ids: Option<(String, String)>,
    ) -> Self {
        let (project_id, feature_id) = match ids {
            Some((p, f)) => (Some(p), Some(f)),
            None => (None, None),
        };
        Self {
            msg_type: AUTOMATION_RESULT_TYPE,
            action: CREATE_FEATURE_ACTION,
            ok: true,
            dry_run: request.dry_run,
            project_name: request.project_name.clone(),
            branch: request.branch.clone(),
            workdir,
            is_worktree,
            tmux_session,
            started,
            worktree_hook_ran,
            worktree_hook_succeeded,
            worktree_hook_prompt,
            message,
            project_id,
            feature_id,
            features: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BatchFeatureAutomationResult {
    pub name: String,
    pub branch: String,
    pub workdir: PathBuf,
    pub tmux_session: String,
    pub started: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateBatchFeaturesResponse {
    #[serde(rename = "type")]
    pub msg_type: &'static str,
    pub action: &'static str,
    pub ok: bool,
    pub dry_run: bool,
    pub workspace_path: PathBuf,
    pub project_name: String,
    pub project_repo: PathBuf,
    pub features: Vec<BatchFeatureAutomationResult>,
    pub message: String,
}

impl CreateBatchFeaturesResponse {
    pub fn success(
        request: &CreateBatchFeaturesRequest,
        project_repo: PathBuf,
        features: Vec<BatchFeatureAutomationResult>,
        message: String,
    ) -> Self {
        Self {
            msg_type: AUTOMATION_RESULT_TYPE,
            action: CREATE_BATCH_FEATURES_ACTION,
            ok: true,
            dry_run: request.dry_run,
            workspace_path: request.workspace_path.clone(),
            project_name: request.project_name.clone(),
            project_repo,
            features,
            message,
        }
    }
}

pub fn automation_error_response(action: &str, error: impl Into<String>) -> serde_json::Value {
    serde_json::json!({
        "type": AUTOMATION_RESULT_TYPE,
        "action": action,
        "ok": false,
        "error": error.into(),
    })
}
