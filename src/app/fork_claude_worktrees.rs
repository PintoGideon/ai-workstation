//! FORK: the worktrees Claude created through AIW's `WorktreeCreate` hook,
//! on the dashboard (DESIGN.md §19.6; the hook itself is
//! `crate::fork::claude_worktrees`).
//!
//! The hook process records each worktree in `aiw.db` and says so on the
//! socket; the dashboard (re)reads the table then and at start-up, works out
//! once which feature each one belongs to (the AIW session whose Claude asked
//! for it, else the checkout the request came from), and draws it as a dimmed
//! `⎇ <name> · <origin>` line under that feature — or under the project when no
//! feature matches. `W` lists them, opens a terminal in one, copies its path,
//! or removes a clean one (the same rules as Claude's `WorktreeRemove`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{App, AppMode};
use crate::fork::claude_worktrees::{self, ClaudeWorktree, HookPaths};

/// Sidebar lines shown per feature or project before `+N more (W)`.
pub const MAX_LINES: usize = 3;

/// Recorded Claude worktrees and where each one belongs.
#[derive(Debug, Default)]
pub struct ClaudeWorktreeRuntime {
    pub(crate) items: Vec<ClaudeWorktree>,
    /// Worktree id → the feature id it is listed under.
    pub(crate) feature_of: HashMap<String, String>,
    /// Worktree id → the project (by name) it is listed under.
    pub(crate) project_of: HashMap<String, String>,
}

/// The `W` dialog.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClaudeWorktreesState {
    /// The worktree ids listed, in order.
    pub ids: Vec<String>,
    pub selected: usize,
    /// `d` pressed: the next `y` removes the selected worktree.
    pub confirm_remove: bool,
    /// The scope, for the title.
    pub title: String,
}

/// Sidebar line count for `n` worktrees: up to [`MAX_LINES`], plus the
/// `+N more` line.
pub fn line_count(n: usize) -> usize {
    if n > MAX_LINES { MAX_LINES + 1 } else { n }
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// A shell-quoted path for the `cd` typed into a fresh terminal.
fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
}

impl App {
    // ── loading ──────────────────────────────────────────────────

    /// Read the recorded worktrees (start-up and after every hook message).
    /// A recorded worktree whose directory is gone (removed outside AIW, or a
    /// finished subagent's worktree deleted by hand) is forgotten here.
    pub(crate) fn load_claude_worktrees(&mut self) {
        let Some(db) = self.db.as_ref() else {
            return;
        };
        let all = match db.all_claude_worktrees() {
            Ok(all) => all,
            Err(e) => {
                self.log_warn(
                    "claude_worktrees",
                    format!("could not load Claude worktrees: {e}"),
                );
                return;
            }
        };
        let (live, gone): (Vec<_>, Vec<_>) = all.into_iter().partition(|wt| wt.path.is_dir());
        for wt in &gone {
            crate::fork::ports::release(&wt.port_key());
            if let Some(db) = self.db.as_ref() {
                let _ = db.delete_claude_worktree(&wt.id);
            }
            self.log_info(
                "claude_worktrees",
                format!("forgot {} (directory gone)", wt.path.display()),
            );
        }
        self.claude_worktrees.items = live;
        self.resolve_claude_worktree_owners();
    }

    /// Work out, once, the feature and project each worktree is listed under.
    pub(crate) fn resolve_claude_worktree_owners(&mut self) {
        let mut feature_of = HashMap::new();
        let mut project_of = HashMap::new();
        // Canonical paths once per call, not per frame.
        let repos: Vec<PathBuf> = self
            .store
            .projects
            .iter()
            .map(|p| canonical(&p.repo))
            .collect();
        let workdirs: Vec<Vec<PathBuf>> = self
            .store
            .projects
            .iter()
            .map(|p| p.features.iter().map(|f| canonical(&f.workdir)).collect())
            .collect();
        for wt in &self.claude_worktrees.items {
            let by_session = wt
                .parent_session_id
                .as_deref()
                .and_then(|sid| self.session_indices_by_id(sid))
                .map(|(pi, fi, _)| (pi, fi));
            let by_checkout = || {
                let cwd = canonical(&wt.requested_cwd);
                // The deepest feature workdir containing the requesting
                // checkout (a repo-dir feature contains every subfolder).
                workdirs
                    .iter()
                    .enumerate()
                    .flat_map(|(pi, dirs)| {
                        dirs.iter().enumerate().map(move |(fi, dir)| (pi, fi, dir))
                    })
                    .filter(|(_, _, dir)| cwd.starts_with(dir))
                    .max_by_key(|(_, _, dir)| dir.components().count())
                    .map(|(pi, fi, _)| (pi, fi))
            };
            if let Some((pi, fi)) = by_session.or_else(by_checkout) {
                let project = &self.store.projects[pi];
                feature_of.insert(wt.id.clone(), project.features[fi].id.clone());
                project_of.insert(wt.id.clone(), project.name.clone());
                continue;
            }
            let repo = canonical(&wt.repo);
            if let Some(pi) = repos.iter().position(|r| *r == repo) {
                project_of.insert(wt.id.clone(), self.store.projects[pi].name.clone());
            }
        }
        self.claude_worktrees.feature_of = feature_of;
        self.claude_worktrees.project_of = project_of;
    }

    /// A `claude-worktree` message from the hook: re-read the table. Returns
    /// whether the message was one.
    pub(crate) fn handle_claude_worktree_message(&mut self, raw: &Value) -> bool {
        if raw.get("type").and_then(Value::as_str) != Some(claude_worktrees::IPC_TYPE) {
            return false;
        }
        let event = raw.get("event").and_then(Value::as_str).unwrap_or_default();
        let id = raw.get("id").and_then(Value::as_str).unwrap_or_default();
        self.load_claude_worktrees();
        if event == "created"
            && let Some(wt) = self.claude_worktrees.items.iter().find(|wt| wt.id == id)
        {
            let place = self
                .claude_worktrees
                .feature_of
                .get(id)
                .and_then(|fid| {
                    self.store
                        .projects
                        .iter()
                        .flat_map(|p| p.features.iter())
                        .find(|f| f.id == *fid)
                })
                .map(|f| format!(" under {}", f.name))
                .unwrap_or_default();
            let text = format!("Claude made worktree {}{place} ({})", wt.name, wt.origin);
            self.log_info("claude_worktrees", text.clone());
            self.push_toast_info(text);
        }
        true
    }

    // ── lookups ──────────────────────────────────────────────────

    /// The worktrees listed under feature `feature_id`.
    pub fn claude_worktrees_for_feature(&self, feature_id: &str) -> Vec<&ClaudeWorktree> {
        self.claude_worktrees
            .items
            .iter()
            .filter(|wt| {
                self.claude_worktrees
                    .feature_of
                    .get(&wt.id)
                    .map(String::as_str)
                    == Some(feature_id)
            })
            .collect()
    }

    /// The worktrees in project `name` that no (still existing) feature
    /// claims.
    pub fn claude_worktrees_for_project(&self, name: &str) -> Vec<&ClaudeWorktree> {
        let feature_ids: Vec<&str> = self
            .store
            .projects
            .iter()
            .find(|p| p.name == name)
            .map(|p| p.features.iter().map(|f| f.id.as_str()).collect())
            .unwrap_or_default();
        self.claude_worktrees
            .items
            .iter()
            .filter(|wt| {
                let claimed = self
                    .claude_worktrees
                    .feature_of
                    .get(&wt.id)
                    .is_some_and(|f| feature_ids.contains(&f.as_str()));
                !claimed
                    && self
                        .claude_worktrees
                        .project_of
                        .get(&wt.id)
                        .map(String::as_str)
                        == Some(name)
            })
            .collect()
    }

    fn claude_worktree(&self, id: &str) -> Option<&ClaudeWorktree> {
        self.claude_worktrees.items.iter().find(|wt| wt.id == id)
    }

    // ── the `W` dialog ───────────────────────────────────────────

    /// `W`: the Claude worktrees of the selected feature (any row of it), or
    /// of the whole project on a project row.
    pub fn open_claude_worktrees(&mut self) {
        use super::Selection;
        let (ids, title) = match self.selection {
            Selection::Feature(pi, fi) | Selection::Session(pi, fi, _) => {
                let feature = &self.store.projects[pi].features[fi];
                (
                    self.claude_worktrees_for_feature(&feature.id)
                        .iter()
                        .map(|wt| wt.id.clone())
                        .collect::<Vec<_>>(),
                    feature.name.clone(),
                )
            }
            Selection::Project(pi) => {
                let project = &self.store.projects[pi];
                let feature_ids: Vec<&str> =
                    project.features.iter().map(|f| f.id.as_str()).collect();
                let ids = self
                    .claude_worktrees
                    .items
                    .iter()
                    .filter(|wt| {
                        self.claude_worktrees
                            .feature_of
                            .get(&wt.id)
                            .is_some_and(|f| feature_ids.contains(&f.as_str()))
                            || self.claude_worktrees.project_of.get(&wt.id) == Some(&project.name)
                    })
                    .map(|wt| wt.id.clone())
                    .collect();
                (ids, project.name.clone())
            }
        };
        if ids.is_empty() {
            self.push_toast_info(if self.config.aiw.claude_worktree_hooks {
                "No worktrees Claude created here"
            } else {
                "No worktrees Claude created here (claude_worktree_hooks is off)"
            });
            return;
        }
        self.mode = AppMode::ClaudeWorktrees(ClaudeWorktreesState {
            ids,
            selected: 0,
            confirm_remove: false,
            title,
        });
    }

    /// The worktrees the open dialog lists (ids still recorded).
    pub fn claude_worktrees_in_dialog(&self) -> Vec<&ClaudeWorktree> {
        let AppMode::ClaudeWorktrees(state) = &self.mode else {
            return Vec::new();
        };
        state
            .ids
            .iter()
            .filter_map(|id| self.claude_worktree(id))
            .collect()
    }

    fn selected_claude_worktree(&self) -> Option<ClaudeWorktree> {
        let AppMode::ClaudeWorktrees(state) = &self.mode else {
            return None;
        };
        state
            .ids
            .get(state.selected)
            .and_then(|id| self.claude_worktree(id))
            .cloned()
    }

    /// Keep the dialog's list in step with the table; close it when empty.
    pub(crate) fn refresh_claude_worktrees_dialog(&mut self) {
        let live: Vec<String> = self
            .claude_worktrees
            .items
            .iter()
            .map(|wt| wt.id.clone())
            .collect();
        if let AppMode::ClaudeWorktrees(state) = &mut self.mode {
            state.ids.retain(|id| live.contains(id));
            if state.ids.is_empty() {
                self.mode = AppMode::Normal;
            } else if state.selected >= state.ids.len() {
                state.selected = state.ids.len() - 1;
            }
        }
    }

    /// `y`: copy the selected worktree's path.
    pub fn copy_claude_worktree_path(&mut self) {
        let Some(wt) = self.selected_claude_worktree() else {
            return;
        };
        let path = wt.path.to_string_lossy().into_owned();
        match super::util::copy_to_clipboard(&path) {
            Ok(()) => self.push_toast_success(format!("Copied: {path}")),
            Err(_) => self.push_toast_info(path),
        }
    }

    /// `t`: a terminal in the selected worktree, as a session of its feature,
    /// with the worktree's port block exported.
    pub fn open_claude_worktree_terminal(&mut self) -> anyhow::Result<()> {
        let Some(wt) = self.selected_claude_worktree() else {
            return Ok(());
        };
        let Some(feature_id) = self.claude_worktrees.feature_of.get(&wt.id).cloned() else {
            self.push_toast_warning(format!(
                "{} belongs to no feature; open a shell there yourself: cd {}",
                wt.name,
                wt.path.display()
            ));
            return Ok(());
        };
        let Some((pi, fi)) = self.store.projects.iter().enumerate().find_map(|(pi, p)| {
            p.features
                .iter()
                .position(|f| f.id == feature_id)
                .map(|fi| (pi, fi))
        }) else {
            return Ok(());
        };
        let tmux_session = self.store.projects[pi].features[fi].tmux_session.clone();
        if !self.tmux.session_exists(&tmux_session) {
            self.push_toast_warning("Start the feature first (c), then open the terminal");
            return Ok(());
        }
        let session_id =
            match self.add_terminal_session_for_picker(pi, fi, Some(format!("⎇ {}", wt.name))) {
                Ok(id) => id,
                Err(e) => {
                    self.push_toast_error(format!("Could not open a terminal: {e}"));
                    return Ok(());
                }
            };
        let window = self.store.projects[pi].features[fi]
            .sessions
            .iter()
            .find(|s| s.id == session_id)
            .map(|s| s.tmux_window.clone());
        if let Some(window) = window {
            let mut line = format!("cd {}", shell_quote(&wt.path));
            if wt.port_start.is_some() {
                line.push_str(&format!(
                    " && . ./{}",
                    crate::fork::claude_worktrees::PORTS_FILE
                ));
            }
            let _ = self.tmux.send_literal(&tmux_session, &window, &line);
            let _ = self.tmux.send_key_name(&tmux_session, &window, "Enter");
        }
        self.mode = AppMode::Normal;
        self.push_toast_success(format!("Terminal in {}", wt.path.display()));
        Ok(())
    }

    /// `d` then `y`: remove the selected worktree if it is clean.
    pub fn remove_selected_claude_worktree(&mut self) {
        let Some(wt) = self.selected_claude_worktree() else {
            return;
        };
        if let AppMode::ClaudeWorktrees(state) = &mut self.mode {
            state.confirm_remove = false;
        }
        let paths = HookPaths {
            db: self
                .db
                .as_ref()
                .map(|db| db.path.clone())
                .unwrap_or_else(crate::project::db_path),
            socket: crate::ipc::socket_path(),
        };
        match claude_worktrees::remove(&wt.path, &paths) {
            Ok(outcome) => {
                let text = outcome.summary(&wt.path);
                self.log_info("claude_worktrees", text.clone());
                self.push_toast_success(text);
            }
            Err(e) => self.push_toast_error(format!("Kept {}: {e:#}", wt.name)),
        }
        self.load_claude_worktrees();
        self.refresh_claude_worktrees_dialog();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_counts_cap_with_a_more_line() {
        assert_eq!(line_count(0), 0);
        assert_eq!(line_count(3), 3);
        assert_eq!(line_count(4), 4);
        assert_eq!(line_count(9), 4);
    }

    #[test]
    fn terminal_paths_are_shell_quoted() {
        assert_eq!(shell_quote(Path::new("/a b/c")), "'/a b/c'");
        assert_eq!(shell_quote(Path::new("/it's")), "'/it'\\''s'");
    }
}
