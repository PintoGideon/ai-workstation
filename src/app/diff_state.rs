//! State for the read-only diff viewer (`d` on the dashboard) and the
//! Vibeless per-edit approval popup. Pure data: no App, persistence or
//! rendering dependencies.

use std::path::PathBuf;
use std::time::Instant;

use super::{DiffScope, DiffViewerFocus, DiffViewerLayout, ViewState};

/// One row of the viewer's file tree: a directory header or a file beneath it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileTreeRow {
    Dir {
        path: String,
        label: String,
        depth: usize,
        collapsed: bool,
        files: usize,
    },
    File {
        index: usize,
        depth: usize,
        name: String,
    },
}

/// Every directory above `path`, shallowest first: `a/b/c.rs` → `[a, a/b]`.
pub fn ancestor_dirs(path: &str) -> Vec<String> {
    let Some(dir_end) = path.rfind('/') else {
        return Vec::new();
    };
    let mut dirs = Vec::new();
    for (i, _) in path[..dir_end].match_indices('/') {
        dirs.push(path[..i].to_string());
    }
    dirs.push(path[..dir_end].to_string());
    dirs
}

/// The diff viewer: a snapshot of changed files, the selected one, and how
/// the patch is shown.
pub struct DiffViewerState {
    pub from_view: ViewState,
    pub workdir: PathBuf,
    pub scope: DiffScope,
    pub branch: String,
    pub base_ref: String,
    pub base_commit: String,
    pub override_base_ref: Option<String>,
    pub editing_base_ref: bool,
    pub base_ref_input: String,
    pub files: Vec<crate::diff::DiffFile>,
    pub selected_file: usize,
    pub patch_scroll: usize,
    pub focus: DiffViewerFocus,
    pub layout: DiffViewerLayout,
    pub error: Option<String>,
    /// Directories folded in the file list.
    pub collapsed_dirs: std::collections::BTreeSet<String>,
    /// The directory row the file-list cursor is parked on, if any.
    pub tree_cursor_dir: Option<String>,
    pub ignore_whitespace: bool,
    /// Per-file context level (lines around each hunk) when widened past the
    /// default; absent files are at `crate::diff::DIFF_DEFAULT_CONTEXT`.
    pub context_expansion: std::collections::HashMap<String, usize>,
}

impl DiffViewerState {
    pub fn new(from_view: ViewState, workdir: PathBuf) -> Self {
        Self {
            from_view,
            workdir,
            scope: DiffScope::CurrentChanges,
            branch: String::new(),
            base_ref: String::new(),
            base_commit: String::new(),
            override_base_ref: None,
            editing_base_ref: false,
            base_ref_input: String::new(),
            files: Vec::new(),
            selected_file: 0,
            patch_scroll: 0,
            focus: DiffViewerFocus::FileList,
            layout: DiffViewerLayout::Unified,
            error: None,
            collapsed_dirs: std::collections::BTreeSet::new(),
            tree_cursor_dir: None,
            ignore_whitespace: false,
            context_expansion: std::collections::HashMap::new(),
        }
    }

    /// Re-widen every file that had a non-default context level before the
    /// snapshot was reloaded. A file that can no longer be widened (left the
    /// changeset, became binary) drops its level.
    pub fn reapply_context_expansion(&mut self) {
        if self.context_expansion.is_empty() {
            return;
        }
        let levels = std::mem::take(&mut self.context_expansion);
        for file in self.files.iter_mut() {
            let Some(&level) = levels.get(&file.path) else {
                continue;
            };
            if let Some(hunks) = file.hunks_with_context(level) {
                file.hunks = hunks;
                self.context_expansion.insert(file.path.clone(), level);
            }
        }
    }

    pub fn on_file_changed(&mut self) {
        self.patch_scroll = 0;
        // The cursor is on a file again, and that file must be visible: every
        // file-order navigation path funnels through here, so no caller has to
        // know the tree can be folded.
        self.tree_cursor_dir = None;
        self.reveal_selected_file();
    }

    /// Indices into `files` of the files shown in the list, in file order.
    pub fn visible_file_indices(&self) -> Vec<usize> {
        (0..self.files.len()).collect()
    }

    /// The file list as a directory tree, in the same order as
    /// `visible_file_indices` — `files` is sorted by full path
    /// (`crate::diff`), and comparing a directory as `name/` against a file as
    /// `name` reproduces exactly that ordering, so grouping never reorders the
    /// list. Directory rows are emitted when the path prefix changes; a
    /// collapsed directory emits its own row and swallows everything beneath
    /// it.
    pub fn file_tree_rows(&self) -> Vec<FileTreeRow> {
        let visible = self.visible_file_indices();
        // Visible-file count per ancestor directory, for the row's `(n)` badge.
        let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for &idx in &visible {
            for dir in ancestor_dirs(&self.files[idx].path) {
                *counts.entry(dir).or_default() += 1;
            }
        }

        let mut rows = Vec::new();
        // Directory segments of the previous file, so a shared prefix is only
        // emitted once.
        let mut open: Vec<&str> = Vec::new();
        for &idx in &visible {
            let path = self.files[idx].path.as_str();
            let (dir_part, name) = match path.rfind('/') {
                Some(pos) => (&path[..pos], &path[pos + 1..]),
                None => ("", path),
            };
            let comps: Vec<&str> = if dir_part.is_empty() {
                Vec::new()
            } else {
                dir_part.split('/').collect()
            };

            let mut common = 0;
            while common < open.len() && common < comps.len() && open[common] == comps[common] {
                common += 1;
            }
            open.truncate(common);

            // A directory already on the stack may be collapsed, in which case
            // its row was emitted earlier and everything below it is hidden.
            let mut hidden = (1..=open.len())
                .any(|depth| self.collapsed_dirs.contains(&comps[..depth].join("/")));

            for depth in common..comps.len() {
                open.push(comps[depth]);
                if hidden {
                    continue;
                }
                let full = comps[..=depth].join("/");
                let collapsed = self.collapsed_dirs.contains(&full);
                rows.push(FileTreeRow::Dir {
                    label: comps[depth].to_string(),
                    depth,
                    collapsed,
                    files: counts.get(&full).copied().unwrap_or(0),
                    path: full,
                });
                if collapsed {
                    hidden = true;
                }
            }

            if !hidden {
                rows.push(FileTreeRow::File {
                    index: idx,
                    depth: comps.len(),
                    name: name.to_string(),
                });
            }
        }
        rows
    }

    /// Expand every collapsed ancestor of the selected file so the selection is
    /// always on a row the user can see. Called from `on_file_changed`, so
    /// every file-order navigation path (n/p, g/G) reveals its target without
    /// having to know about the tree.
    pub fn reveal_selected_file(&mut self) {
        let Some(file) = self.files.get(self.selected_file) else {
            return;
        };
        for dir in ancestor_dirs(&file.path) {
            self.collapsed_dirs.remove(&dir);
        }
    }

    /// Toggle a directory's collapsed state. Collapsing an ancestor of the
    /// selected file is allowed — the file stays selected and the patch panel
    /// keeps showing it; only the row is folded away.
    pub fn toggle_dir_collapsed(&mut self, dir: &str) {
        if !self.collapsed_dirs.remove(dir) {
            self.collapsed_dirs.insert(dir.to_string());
        }
    }

    /// Every directory that currently has a row in the tree (regardless of
    /// collapse state), in row order.
    pub fn tree_dirs(&self) -> Vec<String> {
        self.file_tree_rows()
            .into_iter()
            .filter_map(|row| match row {
                FileTreeRow::Dir { path, .. } => Some(path),
                FileTreeRow::File { .. } => None,
            })
            .collect()
    }

    /// Row index the file-list cursor sits on: the directory row when the
    /// cursor is parked on one, else the selected file's row. Falls back to the
    /// deepest visible ancestor directory if the selected file happens to be
    /// folded away, so a row is always highlighted.
    pub fn tree_cursor_row(&self, rows: &[FileTreeRow]) -> Option<usize> {
        if let Some(dir) = &self.tree_cursor_dir
            && let Some(pos) = rows
                .iter()
                .position(|row| matches!(row, FileTreeRow::Dir { path, .. } if path == dir))
        {
            return Some(pos);
        }
        if let Some(pos) = rows.iter().position(
            |row| matches!(row, FileTreeRow::File { index, .. } if *index == self.selected_file),
        ) {
            return Some(pos);
        }
        let path = self.files.get(self.selected_file)?.path.as_str();
        ancestor_dirs(path).into_iter().rev().find_map(|dir| {
            rows.iter()
                .position(|row| matches!(row, FileTreeRow::Dir { path, .. } if *path == dir))
        })
    }

    /// Directory the fold commands act on, derived from whichever row
    /// `tree_cursor_row` highlights: a directory row folds itself, a file row
    /// folds its own directory.
    pub fn tree_cursor_target_dir(&self, rows: &[FileTreeRow]) -> Option<String> {
        match rows.get(self.tree_cursor_row(rows)?)? {
            FileTreeRow::Dir { path, .. } => Some(path.clone()),
            FileTreeRow::File { index, .. } => self
                .files
                .get(*index)
                .and_then(|file| ancestor_dirs(&file.path).pop()),
        }
    }
}

/// The Vibeless per-edit approval popup: a hook holds one file write until
/// the user answers (proceed / reject with feedback / skip).
pub struct DiffReviewState {
    pub session_id: String,
    pub relative_path: String,
    pub tool: String,
    pub old_snippet: String,
    pub new_snippet: String,
    pub diff_file: Option<crate::diff::DiffFile>,
    pub diff_error: Option<String>,
    pub patch_scroll: usize,
    pub reason: String,
    pub editing_feedback: bool,
    pub layout: DiffViewerLayout,
    pub response_file: PathBuf,
    pub proceed_signal: PathBuf,
    pub request_id: Option<String>,
    pub reply_socket: Option<String>,
    pub return_to_view: Option<ViewState>,
    pub opened_at: Instant,
    pub hold_secs: f64,
}

impl DiffReviewState {
    pub fn hold_remaining_secs(&self) -> f64 {
        let elapsed = self.opened_at.elapsed().as_secs_f64();
        (self.hold_secs - elapsed).max(0.0)
    }

    pub fn hold_active(&self) -> bool {
        self.hold_remaining_secs() > 0.0
    }
}
