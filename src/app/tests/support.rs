use crate::app::*;
use crate::project::{AgentKind, Feature, FeatureSession, Project, SessionKind};
use chrono::Utc;
use crossterm::event::KeyCode;
use std::collections::HashMap;

pub(super) fn prompt_entry(text: &str) -> String {
    text.to_string()
}

/// Build a minimal `ProjectStore` with one project and one
/// feature at the requested status.
pub(super) fn store_with_feature(status: ProjectStatus) -> ProjectStore {
    let now = Utc::now();
    let feature = Feature {
        id: "feat-1".to_string(),
        name: "my-feat".to_string(),
        branch: "my-feat".to_string(),
        workdir: PathBuf::from("/tmp/test-workdir"),
        is_worktree: false,
        tmux_session: "aiw-my-feat".to_string(),
        sessions: vec![],
        collapsed: false,
        mode: VibeMode::default(),
        agent: AgentKind::default(),
        enable_chrome: false,
        pending_worktree_script: false,
        ready: false,
        status,
        created_at: now,
        last_accessed: now,
        summary: None,
        summary_updated_at: None,
        nickname: None,
        issue_source: None,
    };
    let project = Project {
        id: "proj-1".to_string(),
        name: "my-project".to_string(),
        repo: PathBuf::from("/tmp/test-repo"),
        collapsed: false,
        features: vec![feature],
        created_at: now,
        preferred_agent: AgentKind::default(),
        is_git: false,
    };
    ProjectStore {
        version: 2,
        projects: vec![project],
        session_bookmarks: vec![],
        available_harnesses: vec![],
        prompt_templates: Vec::new(),
        extra: HashMap::new(),
    }
}

pub(super) fn store_with_repo(repo: PathBuf, status: ProjectStatus) -> ProjectStore {
    let now = Utc::now();
    let feature = Feature {
        id: "feat-1".to_string(),
        name: "my-feat".to_string(),
        branch: "my-feat".to_string(),
        workdir: repo.clone(),
        is_worktree: false,
        tmux_session: "aiw-my-feat".to_string(),
        sessions: vec![],
        collapsed: false,
        mode: VibeMode::default(),
        agent: AgentKind::default(),
        enable_chrome: false,
        pending_worktree_script: false,
        ready: false,
        status,
        created_at: now,
        last_accessed: now,
        summary: None,
        summary_updated_at: None,
        nickname: None,
        issue_source: None,
    };
    let project = Project {
        id: "proj-1".to_string(),
        name: "my-project".to_string(),
        repo,
        collapsed: false,
        features: vec![feature],
        created_at: now,
        preferred_agent: AgentKind::default(),
        is_git: false,
    };
    ProjectStore {
        version: 2,
        projects: vec![project],
        session_bookmarks: vec![],
        available_harnesses: vec![],
        prompt_templates: Vec::new(),
        extra: HashMap::new(),
    }
}

pub(super) fn make_session(label: &str, status_text: Option<&str>) -> FeatureSession {
    FeatureSession {
        id: format!("session-{label}"),
        kind: SessionKind::Claude,
        label: label.to_string(),
        tmux_window: label.to_string(),
        claude_session_id: None,
        todo_reference: None,
        token_usage_source: None,
        token_usage_source_match: None,
        created_at: Utc::now(),
        command: None,
        on_stop: None,
        pre_check: None,
        status_text: status_text.map(str::to_string),
        token_usage: None,
        stopped: false,
    }
}

pub(super) fn store_with_worktree_agent(
    repo: &std::path::Path,
    workdir: &std::path::Path,
    agent: AgentKind,
    status: ProjectStatus,
    sessions: Vec<crate::project::FeatureSession>,
) -> ProjectStore {
    let now = Utc::now();
    let feature = Feature {
        id: "feat-1".to_string(),
        name: "my-feat".to_string(),
        branch: "my-feat".to_string(),
        workdir: workdir.to_path_buf(),
        is_worktree: true,
        tmux_session: "aiw-my-feat".to_string(),
        sessions,
        collapsed: false,
        mode: VibeMode::default(),
        agent: agent.clone(),
        enable_chrome: false,
        pending_worktree_script: false,
        ready: false,
        status,
        created_at: now,
        last_accessed: now,
        summary: None,
        summary_updated_at: None,
        nickname: None,
        issue_source: None,
    };
    let project = Project {
        id: "proj-1".to_string(),
        name: "my-project".to_string(),
        repo: repo.to_path_buf(),
        collapsed: false,
        features: vec![feature],
        created_at: now,
        preferred_agent: AgentKind::default(),
        is_git: true,
    };
    ProjectStore {
        version: 2,
        projects: vec![project],
        session_bookmarks: vec![],
        available_harnesses: vec![],
        prompt_templates: Vec::new(),
        extra: HashMap::new(),
    }
}

// ── sync_session_status ──────────────────────────────────────

pub(super) fn store_with_custom_session(
    workdir: &std::path::Path,
    session_id: &str,
) -> ProjectStore {
    let now = Utc::now();
    let session = FeatureSession {
        id: session_id.to_string(),
        kind: SessionKind::Custom,
        label: "Dev Servers".to_string(),
        tmux_window: "custom".to_string(),
        claude_session_id: None,
        todo_reference: None,
        token_usage_source: None,
        token_usage_source_match: None,
        created_at: now,
        command: Some("./start.sh".to_string()),
        on_stop: None,
        pre_check: None,
        status_text: None,
        token_usage: None,
        stopped: false,
    };
    let feature = Feature {
        id: "feat-1".to_string(),
        name: "my-feat".to_string(),
        branch: "my-feat".to_string(),
        workdir: workdir.to_path_buf(),
        is_worktree: false,
        tmux_session: "aiw-my-feat".to_string(),
        sessions: vec![session],
        collapsed: false,
        mode: VibeMode::default(),
        agent: AgentKind::default(),
        enable_chrome: false,
        pending_worktree_script: false,
        ready: false,
        status: ProjectStatus::Idle,
        created_at: now,
        last_accessed: now,
        summary: None,
        summary_updated_at: None,
        nickname: None,
        issue_source: None,
    };
    let project = Project {
        id: "proj-1".to_string(),
        name: "my-project".to_string(),
        repo: workdir.to_path_buf(),
        collapsed: false,
        features: vec![feature],
        created_at: now,
        preferred_agent: AgentKind::default(),
        is_git: false,
    };
    ProjectStore {
        version: 2,
        projects: vec![project],
        session_bookmarks: vec![],
        available_harnesses: vec![],
        prompt_templates: Vec::new(),
        extra: HashMap::new(),
    }
}

pub(super) fn store_with_codex_session(
    workdir: &std::path::Path,
    is_worktree: bool,
) -> ProjectStore {
    let now = Utc::now();
    let session = FeatureSession {
        id: "codex-sess".to_string(),
        kind: SessionKind::Codex,
        label: "Codex".to_string(),
        tmux_window: "codex".to_string(),
        claude_session_id: None,
        todo_reference: None,
        token_usage_source: None,
        token_usage_source_match: None,
        created_at: now,
        command: None,
        on_stop: None,
        pre_check: None,
        status_text: None,
        token_usage: None,
        stopped: false,
    };
    let feature = Feature {
        id: "feat-1".to_string(),
        name: "my-feat".to_string(),
        branch: "my-feat".to_string(),
        workdir: workdir.to_path_buf(),
        is_worktree,
        tmux_session: "aiw-my-feat".to_string(),
        sessions: vec![session],
        collapsed: false,
        mode: VibeMode::default(),
        agent: AgentKind::Codex,
        enable_chrome: false,
        pending_worktree_script: false,
        ready: false,
        status: ProjectStatus::Idle,
        created_at: now,
        last_accessed: now,
        summary: None,
        summary_updated_at: None,
        nickname: None,
        issue_source: None,
    };
    let project = Project {
        id: "proj-1".to_string(),
        name: "my-project".to_string(),
        repo: workdir.to_path_buf(),
        collapsed: false,
        features: vec![feature],
        created_at: now,
        preferred_agent: AgentKind::default(),
        is_git: false,
    };
    ProjectStore {
        version: 2,
        projects: vec![project],
        session_bookmarks: vec![],
        available_harnesses: vec![],
        prompt_templates: Vec::new(),
        extra: HashMap::new(),
    }
}

pub(super) fn store_with_empty_project(repo: PathBuf, is_git: bool) -> ProjectStore {
    let now = Utc::now();
    let project = Project {
        id: "proj-1".to_string(),
        name: "automation-project".to_string(),
        repo,
        collapsed: false,
        features: vec![],
        created_at: now,
        preferred_agent: AgentKind::default(),
        is_git,
    };
    ProjectStore {
        version: 4,
        projects: vec![project],
        session_bookmarks: vec![],
        available_harnesses: vec![],
        prompt_templates: Vec::new(),
        extra: HashMap::new(),
    }
}

pub(super) fn view_state_for(project_name: &str, feature_name: &str) -> ViewState {
    ViewState::new(
        project_name.to_string(),
        feature_name.to_string(),
        "aiw-my-feat".to_string(),
        "claude".to_string(),
        "Claude".to_string(),
        SessionKind::Claude,
        VibeMode::default(),
    )
}

pub(super) fn ke(code: KeyCode) -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
}

// ----- scoped TODO lists ---------------------------------------------------

pub(super) fn worktree_scope(project_id: &str, workdir: &str) -> crate::db::todos::TodoScope {
    crate::db::todos::TodoScope::Worktree {
        project_id: project_id.to_string(),
        workdir: workdir.to_string(),
    }
}

/// Turn the fixture's single feature into a real worktree, so it has a
/// worktree list of its own.
pub(super) fn make_feature_a_worktree(app: &mut App) {
    app.store.projects[0].features[0].is_worktree = true;
}

/// A real shell with a real child, standing in for a tmux pane that has a
/// harness running in it. The census asks `ps` whether the pane's shell has
/// anything under it, so a made-up pid would read as an idle prompt — which is
/// exactly the distinction being tested elsewhere.
pub(super) struct BusyPane(pub(super) std::process::Child);

impl Drop for BusyPane {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
