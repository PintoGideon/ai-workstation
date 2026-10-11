//! FORK: memo for the few dialog values the dashboard used to recompute on
//! every frame (DESIGN.md §12.3). `repo_for_project_path` runs `git rev-parse`
//! and `allowed_agents_for_repo` re-reads the global config and the repo's
//! `aiw.json`, so an open new-project form spent a process and two file reads
//! per frame (and the throbber redraws several times a second).
//!
//! A value is reused only for the same dialog and path, and only for
//! [`MEMO_TTL`], so a config edited elsewhere still shows up within a second.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::App;
use crate::project::AgentKind;

const MEMO_TTL: Duration = Duration::from_secs(1);

#[derive(Default)]
pub(crate) struct RenderMemo {
    entry: Mutex<Option<(PathBuf, Instant, Vec<AgentKind>)>>,
}

impl RenderMemo {
    fn get_or(&self, key: &Path, compute: impl FnOnce() -> Vec<AgentKind>) -> Vec<AgentKind> {
        let mut entry = self.entry.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((cached_key, at, value)) = entry.as_ref()
            && cached_key == key
            && at.elapsed() < MEMO_TTL
        {
            return value.clone();
        }
        let value = compute();
        *entry = Some((key.to_path_buf(), Instant::now(), value.clone()));
        value
    }
}

impl App {
    /// `allowed_agents_for_project_path(path)`, memoised for drawing.
    pub(crate) fn render_allowed_agents_for_project_path(&self, path: &Path) -> Vec<AgentKind> {
        self.fork_render_memo
            .get_or(path, || self.allowed_agents_for_project_path(path))
    }
}
