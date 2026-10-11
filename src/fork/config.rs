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
    /// How long a permission hook waits for an answer from the dashboard
    /// before letting the harness show its own prompt (P6).
    pub permission_wait_seconds: u64,
    /// Tell each agent its project, feature, branch, worktree and AIW session
    /// in its system prompt (item 6). On by default.
    pub agent_context_prompt: bool,
    /// Pass Claude's hooks and status line with `--settings <file>` (default)
    /// instead of writing hook entries into the repo's
    /// `.claude/settings.local.json` (item 7, DESIGN.md §16.6).
    pub claude_hooks_via_settings_flag: bool,
    /// Width of the cockpit's sidebar pane in columns (DESIGN.md §18);
    /// at least 28.
    pub sidebar_width: u16,
}

impl Default for AiwConfig {
    fn default() -> Self {
        Self {
            worktree_base_dir: None,
            worktree_include: default_worktree_include(),
            confirm_quit: false,
            permission_wait_seconds: crate::fork::permissions::DEFAULT_WAIT_SECONDS,
            agent_context_prompt: true,
            claude_hooks_via_settings_flag: true,
            sidebar_width: crate::fork::cockpit::DEFAULT_SIDEBAR_WIDTH,
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

    #[test]
    fn unknown_keys_are_ignored_and_the_sidebar_width_has_a_floor() {
        let config: crate::app::AppConfig = serde_json::from_str(
            r#"{"dashboard_layout":"tree","compose_box":true,"view_auto_refresh":false,"no_such_key":1,"sidebar_width":40}"#,
        )
        .unwrap();
        assert_eq!(config.aiw.sidebar_width, 40);
        let config: crate::app::AppConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(config.aiw.sidebar_width, 36);
        assert_eq!(
            crate::fork::cockpit::sidebar_width(config.aiw.sidebar_width),
            36
        );
        let rendered = serde_json::to_value(&config).unwrap();
        assert_eq!(rendered["sidebar_width"], 36);
        // The removed keys are neither read nor written back.
        for gone in ["dashboard_layout", "compose_box", "view_auto_refresh"] {
            assert!(rendered.get(gone).is_none(), "{gone} is still written");
        }
    }
}
