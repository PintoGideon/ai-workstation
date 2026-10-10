//! FORK: the launch-time arguments for a session, looked up from the store
//! (DESIGN.md §16). The sites that start Claude / Codex with `&self` free
//! call [`App::fork_launch_args`]; the ones holding a `&mut Feature` build a
//! [`LaunchContext`] from its fields and call `fork::launch` directly.

use super::App;
use crate::fork::launch::{LaunchContext, claude_launch_args, codex_launch_args};
use crate::project::AgentKind;

impl App {
    /// The launch context of the session `session_id` in the feature whose
    /// tmux session is `tmux_session`.
    pub(crate) fn fork_launch_context(
        &self,
        tmux_session: &str,
        session_id: &str,
    ) -> Option<LaunchContext> {
        for project in &self.store.projects {
            if let Some(feature) = project
                .features
                .iter()
                .find(|f| f.tmux_session == tmux_session)
            {
                return Some(LaunchContext::for_feature(
                    &project.name,
                    &project.repo,
                    feature,
                    session_id,
                ));
            }
        }
        None
    }

    /// `args` plus what AIW adds for `agent` at launch (nothing for the
    /// agents whose hooks carry the context instead).
    pub(crate) fn fork_launch_args(
        &self,
        agent: &AgentKind,
        tmux_session: &str,
        session_id: &str,
        args: Vec<String>,
    ) -> Vec<String> {
        let Some(ctx) = self.fork_launch_context(tmux_session, session_id) else {
            return args;
        };
        match agent {
            AgentKind::Claude => claude_launch_args(&ctx, &self.config.aiw, args),
            AgentKind::Codex => codex_launch_args(&ctx, &self.config.aiw, args),
            _ => args,
        }
    }
}
