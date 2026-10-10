//! AIW-only keys in `config.json`, flattened into AIW's `AppConfig` so they
//! live at the top level of the same file and survive AIW's own rewrites.

use serde::{Deserialize, Serialize};

/// Patterns copied into a fresh worktree when the repo has no
/// `.worktreeinclude` file.
pub(crate) fn default_worktree_include() -> Vec<String> {
    vec![".env*".to_string()]
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct AiwConfig {
    /// Where new feature worktrees are created. `None` (the default) means
    /// `<data-local-dir>/ai_workstation/worktrees`. A leading `~` is
    /// expanded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_base_dir: Option<String>,
    /// Gitignore-style patterns for git-ignored files to copy from the source
    /// checkout into a fresh worktree. A repo's `.worktreeinclude` overrides
    /// this.
    pub worktree_include: Vec<String>,
    /// Ask before `q` quits the dashboard while agents are running: the first
    /// press shows how many and the second press (within a few seconds)
    /// quits. Off by default, so `q` quits at once as it always has.
    pub confirm_quit: bool,
    /// The dashboard layout: the workspace (a session sidebar beside a live
    /// pane, DESIGN.md §15 — the default) or the project tree. `b` on the
    /// dashboard toggles it and saves at once.
    pub dashboard_layout: DashboardLayout,
    /// How long a permission hook waits for an answer from the dashboard
    /// before letting the harness show its own prompt (P6).
    pub permission_wait_seconds: u64,
}

impl Default for AiwConfig {
    fn default() -> Self {
        Self {
            worktree_base_dir: None,
            worktree_include: default_worktree_include(),
            confirm_quit: false,
            dashboard_layout: DashboardLayout::Workspace,
            permission_wait_seconds: crate::fork::permissions::DEFAULT_WAIT_SECONDS,
        }
    }
}

/// `dashboard_layout` in `config.json`. The workspace is the default; a
/// config that says `"tree"` keeps the tree.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum DashboardLayout {
    Tree,
    #[default]
    Workspace,
}

impl DashboardLayout {
    pub fn toggled(self) -> Self {
        match self {
            DashboardLayout::Tree => DashboardLayout::Workspace,
            DashboardLayout::Workspace => DashboardLayout::Tree,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            DashboardLayout::Tree => "tree",
            DashboardLayout::Workspace => "workspace",
        }
    }
}

/// The AIW keys from the on-disk config (read-only; nothing is written).
pub(crate) fn current() -> AiwConfig {
    crate::app::setup::read_config().aiw
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_flattened_into_app_config() {
        let config: crate::app::AppConfig = serde_json::from_str(
            r#"{"worktree_base_dir":"~/wt","worktree_include":[".env",".secrets/*"]}"#,
        )
        .unwrap();
        assert_eq!(config.aiw.worktree_base_dir.as_deref(), Some("~/wt"));
        assert_eq!(config.aiw.worktree_include, vec![".env", ".secrets/*"]);

        let rendered = serde_json::to_value(&config).unwrap();
        assert_eq!(rendered["worktree_base_dir"], "~/wt");
        assert!(rendered.get("aiw").is_none());
    }

    #[test]
    fn missing_keys_use_defaults() {
        let config: crate::app::AppConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(config.aiw, AiwConfig::default());
        assert_eq!(config.aiw.worktree_include, vec![".env*"]);
        let rendered = serde_json::to_value(&config).unwrap();
        assert!(rendered.get("worktree_base_dir").is_none());
        assert_eq!(rendered["worktree_include"][0], ".env*");
        assert_eq!(rendered["dashboard_layout"], "workspace");
    }

    #[test]
    fn dashboard_layout_round_trips() {
        let config: crate::app::AppConfig =
            serde_json::from_str(r#"{"dashboard_layout":"workspace"}"#).unwrap();
        assert_eq!(config.aiw.dashboard_layout, DashboardLayout::Workspace);
        assert_eq!(DashboardLayout::Workspace.toggled(), DashboardLayout::Tree);
        let rendered = serde_json::to_value(&config).unwrap();
        assert_eq!(rendered["dashboard_layout"], "workspace");
    }

    #[test]
    fn an_existing_tree_setting_is_kept_over_the_workspace_default() {
        assert_eq!(DashboardLayout::default(), DashboardLayout::Workspace);
        let config: crate::app::AppConfig =
            serde_json::from_str(r#"{"dashboard_layout":"tree"}"#).unwrap();
        assert_eq!(config.aiw.dashboard_layout, DashboardLayout::Tree);
        let rendered = serde_json::to_value(&config).unwrap();
        assert_eq!(rendered["dashboard_layout"], "tree");
    }
}
