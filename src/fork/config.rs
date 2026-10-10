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
}

impl Default for AiwConfig {
    fn default() -> Self {
        Self {
            worktree_base_dir: None,
            worktree_include: default_worktree_include(),
            confirm_quit: false,
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
    }
}
