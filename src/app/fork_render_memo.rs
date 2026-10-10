//! FORK: memo for the few dialog values the dashboard used to recompute on
//! every frame (DESIGN.md §12.3). `repo_for_project_path` runs `git rev-parse`
//! and `review_memory_paths` / `allowed_agents_for_repo` re-read the global
//! config and the repo's `aiw.json`, so an open PR picker, compact-memory
//! confirmation or new-project form spent a process and two file reads per
//! frame (and the throbber redraws several times a second).
//!
//! A value is reused only for the same dialog and path, and only for
//! [`MEMO_TTL`], so a config edited elsewhere still shows up within a second.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::App;
use super::review_memory::ReviewMemoryPaths;
use crate::project::AgentKind;

const MEMO_TTL: Duration = Duration::from_secs(1);

#[derive(Clone)]
enum Value {
    MemoryPaths(ReviewMemoryPaths),
    Agents(Vec<AgentKind>),
}

#[derive(Default)]
pub(crate) struct RenderMemo {
    entry: Mutex<Option<(PathBuf, Instant, Value)>>,
}

impl RenderMemo {
    fn get_or(
        &self,
        key: &Path,
        kind: fn(&Value) -> bool,
        compute: impl FnOnce() -> Value,
    ) -> Value {
        let mut entry = self.entry.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((cached_key, at, value)) = entry.as_ref()
            && cached_key == key
            && at.elapsed() < MEMO_TTL
            && kind(value)
        {
            return value.clone();
        }
        let value = compute();
        *entry = Some((key.to_path_buf(), Instant::now(), value.clone()));
        value
    }
}

impl App {
    /// `review_memory_paths(repo_for_project_path(workdir))`, memoised for drawing.
    pub(crate) fn render_review_memory_paths(&self, workdir: &Path) -> ReviewMemoryPaths {
        let value = self.fork_render_memo.get_or(
            workdir,
            |value| matches!(value, Value::MemoryPaths(_)),
            || {
                let repo = self.repo_for_project_path(workdir);
                Value::MemoryPaths(self.review_memory_paths(&repo))
            },
        );
        match value {
            Value::MemoryPaths(paths) => paths,
            Value::Agents(_) => unreachable!("kind checked above"),
        }
    }

    /// `allowed_agents_for_project_path(path)`, memoised for drawing.
    pub(crate) fn render_allowed_agents_for_project_path(&self, path: &Path) -> Vec<AgentKind> {
        let value = self.fork_render_memo.get_or(
            path,
            |value| matches!(value, Value::Agents(_)),
            || Value::Agents(self.allowed_agents_for_project_path(path)),
        );
        match value {
            Value::Agents(agents) => agents,
            Value::MemoryPaths(_) => unreachable!("kind checked above"),
        }
    }
}
