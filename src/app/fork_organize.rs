//! FORK: feature groups, notes and layout presets on `App` (DESIGN.md
//! §19.4). The pure rules live in `fork::organize`, the tables in
//! `db::organize`.
//!
//! - **Groups.** `G` opens the project's groups: move the selected feature
//!   into one (or out), create, rename, recolour, reorder and delete them.
//!   `C` collapses or expands the selected feature's group; a collapsed
//!   group shows as one row (its first feature stands for it), so the
//!   selection model is unchanged. Group headers are drawn above the first
//!   feature of an expanded group, like the fan-out header (§19.3).
//! - **Notes.** `#` edits the note of the selected session (or feature).
//!   Notes show in the session info block and match in `/` search.
//! - **Layout presets.** The New Feature dialog's Layout row (and a feature
//!   preset's `layout`) names a `layout_presets` entry; its sessions are
//!   opened once the new feature is running.

use std::collections::HashMap;

use anyhow::Result;
use chrono::{DateTime, Utc};

use super::{App, AppMode, Selection, StartIntent};
use crate::fork::organize::{
    FeatureGroup, FeatureKey, FeatureSlot, LayoutSession, NoteTarget, default_color, feature_slots,
    group_name_error, next_color, normalize_note, order_features, resolve_layout,
};
use crate::project::{AgentKind, Feature, Project};

/// Groups, memberships, notes and a layout waiting for its feature.
#[derive(Debug, Default)]
pub struct OrganizeRuntime {
    pub groups: Vec<FeatureGroup>,
    /// Group id by feature id.
    pub members: HashMap<String, String>,
    pub notes: HashMap<NoteTarget, String>,
    /// A layout chosen in the dialog, applied as its features launch.
    pub pending_layout: Option<PendingLayout>,
}

/// A layout preset to open in the next `remaining` features launched in
/// `project` (one, or one per worktree-each sibling).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingLayout {
    pub project: String,
    pub preset: String,
    pub remaining: usize,
}

/// A row of the `G` dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupRow {
    Ungroup,
    Group(usize),
    New,
}

/// What the `G` dialog's text field is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GroupEdit {
    New,
    Rename(String),
}

/// The `G` dialog.
#[derive(Debug, Clone)]
pub struct FeatureGroupsState {
    pub project_id: String,
    pub project_name: String,
    /// The feature `Enter` moves, when a feature (or its session) was
    /// selected.
    pub feature_id: Option<String>,
    pub feature_name: Option<String>,
    pub selected: usize,
    pub edit: Option<(GroupEdit, String)>,
    /// A group waiting for `y` to delete it.
    pub confirm_delete: Option<String>,
    pub error: Option<String>,
}

/// The `#` note editor.
#[derive(Debug, Clone)]
pub struct NoteEditorState {
    pub target: NoteTarget,
    /// What the note is on, for the title (`Claude 1 · auth-fix`).
    pub title: String,
    pub text: String,
    /// The note when the editor opened, so Esc can tell nothing changed.
    pub original: String,
}

impl App {
    // ── start-up and cleanup ─────────────────────────────────────

    /// Load groups, memberships and notes (start-up).
    pub(crate) fn load_organize_state(&mut self) {
        let Some(db) = self.db.as_ref() else {
            return;
        };
        let groups = db.all_feature_groups();
        let members = db.all_feature_group_members();
        let notes = db.all_notes();
        match groups {
            Ok(groups) => self.organize.groups = groups,
            Err(e) => self.log_warn("groups", format!("could not load feature groups: {e}")),
        }
        match members {
            Ok(members) => {
                let known: Vec<&str> = self.organize.groups.iter().map(|g| g.id.as_str()).collect();
                self.organize.members = members
                    .into_iter()
                    .filter(|(_, group)| known.contains(&group.as_str()))
                    .collect();
            }
            Err(e) => self.log_warn("groups", format!("could not load group members: {e}")),
        }
        match notes {
            Ok(notes) => self.organize.notes = notes.into_iter().collect(),
            Err(e) => self.log_warn("notes", format!("could not load notes: {e}")),
        }
    }

    /// A session was deleted: drop its note.
    pub(crate) fn forget_organize_session(&mut self, session_id: &str) {
        self.forget_schedules_for_session(session_id); // FORK: §19.9
        let target = NoteTarget::Session(session_id.to_string());
        if self.organize.notes.remove(&target).is_some()
            && let Some(db) = self.db.as_ref()
            && let Err(e) = db.set_note(&target, None)
        {
            self.log_warn("notes", format!("could not delete a note: {e}"));
        }
    }

    /// A feature was deleted: drop its notes and its group seat.
    pub(crate) fn forget_organize_feature(&mut self, feature: &Feature) {
        self.forget_schedules_for_feature(&feature.id); // FORK: §19.9
        self.forget_land_feature(&feature.id); // FORK: §19.10
        self.forget_feature_inputs(feature); // FORK: §19.11
        for session in &feature.sessions {
            self.forget_organize_session(&session.id);
        }
        let target = NoteTarget::Feature(feature.id.clone());
        if self.organize.notes.remove(&target).is_some()
            && let Some(db) = self.db.as_ref()
            && let Err(e) = db.set_note(&target, None)
        {
            self.log_warn("notes", format!("could not delete a note: {e}"));
        }
        if self.organize.members.remove(&feature.id).is_some()
            && let Some(db) = self.db.as_ref()
            && let Err(e) = db.set_feature_group(&feature.id, None)
        {
            self.log_warn("groups", format!("could not delete a group seat: {e}"));
        }
    }

    /// A project was deleted: drop its groups and every feature's notes.
    pub(crate) fn forget_organize_project(&mut self, project: &Project) {
        for feature in &project.features {
            self.forget_organize_feature(feature);
        }
        let ids: Vec<String> = self
            .organize
            .groups
            .iter()
            .filter(|g| g.project_id == project.id)
            .map(|g| g.id.clone())
            .collect();
        for id in ids {
            self.organize.groups.retain(|g| g.id != id);
            self.organize.members.retain(|_, g| *g != id);
            if let Some(db) = self.db.as_ref()
                && let Err(e) = db.delete_feature_group(&id)
            {
                self.log_warn("groups", format!("could not delete a group: {e}"));
            }
        }
    }

    // ── order and slots ──────────────────────────────────────────

    /// The project's groups, by position.
    pub fn project_groups(&self, pi: usize) -> Vec<&FeatureGroup> {
        let Some(project) = self.store.projects.get(pi) else {
            return Vec::new();
        };
        let mut groups: Vec<&FeatureGroup> = self
            .organize
            .groups
            .iter()
            .filter(|g| g.project_id == project.id)
            .collect();
        groups.sort_by(|a, b| a.position.cmp(&b.position).then(a.id.cmp(&b.id)));
        groups
    }

    /// The group `(pi, fi)` is in, if any.
    pub fn feature_group(&self, pi: usize, fi: usize) -> Option<&FeatureGroup> {
        let project = self.store.projects.get(pi)?;
        let feature = project.features.get(fi)?;
        let group_id = self.organize.members.get(&feature.id)?;
        self.organize
            .groups
            .iter()
            .find(|g| g.id == *group_id && g.project_id == project.id)
    }

    /// The project's features in display order (ungrouped first, then each
    /// group by position).
    pub fn project_feature_order(&self, pi: usize) -> Vec<usize> {
        let Some(project) = self.store.projects.get(pi) else {
            return Vec::new();
        };
        let keys: Vec<FeatureKey> = project
            .features
            .iter()
            .enumerate()
            .map(|(fi, f)| FeatureKey {
                index: fi,
                is_worktree: f.is_worktree,
                created_at: f.created_at,
                group: self
                    .feature_group(pi, fi)
                    .map(|g| (g.position, g.id.clone())),
            })
            .collect();
        order_features(&keys)
    }

    /// Every feature of the project, in display order, with its slot.
    pub fn project_feature_slots(&self, pi: usize) -> Vec<(usize, FeatureSlot)> {
        let order = self.project_feature_order(pi);
        feature_slots(&order, |fi| {
            self.feature_group(pi, fi)
                .map(|g| (g.id.clone(), g.collapsed))
        })
    }

    /// `(pi, fi)`'s slot.
    pub fn feature_slot(&self, pi: usize, fi: usize) -> FeatureSlot {
        if self.organize.groups.is_empty() {
            return FeatureSlot::Ungrouped;
        }
        self.project_feature_slots(pi)
            .into_iter()
            .find(|(i, _)| *i == fi)
            .map(|(_, slot)| slot)
            .unwrap_or(FeatureSlot::Ungrouped)
    }

    /// Features in the group and how many of them need the user.
    pub fn group_rollup(&self, pi: usize, group_id: &str, now: DateTime<Utc>) -> (usize, usize) {
        let Some(project) = self.store.projects.get(pi) else {
            return (0, 0);
        };
        let members: Vec<usize> = project
            .features
            .iter()
            .enumerate()
            .filter(|(_, f)| self.organize.members.get(&f.id).map(String::as_str) == Some(group_id))
            .map(|(fi, _)| fi)
            .collect();
        let needs_you = members
            .iter()
            .filter(|&&fi| {
                crate::fork::badges::feature_badge(self, pi, fi, now).kind
                    == crate::fork::badges::BadgeKind::NeedsYou
            })
            .count();
        (members.len(), needs_you)
    }

    // ── group changes ────────────────────────────────────────────

    fn persist_group(&mut self, group: &FeatureGroup) {
        if let Some(db) = self.db.as_ref()
            && let Err(e) = db.save_feature_group(group)
        {
            self.log_warn(
                "groups",
                format!("could not save group '{}': {e}", group.name),
            );
        }
    }

    /// Create a group in the project; returns its id.
    pub fn create_feature_group(&mut self, pi: usize, name: &str) -> Result<String> {
        let project_id = self
            .store
            .projects
            .get(pi)
            .map(|p| p.id.clone())
            .ok_or_else(|| anyhow::anyhow!("project not found"))?;
        let existing = self.project_groups(pi);
        let names: Vec<&str> = existing.iter().map(|g| g.name.as_str()).collect();
        if let Some(error) = group_name_error(name, &names) {
            anyhow::bail!(error);
        }
        let colors: Vec<&str> = existing.iter().map(|g| g.color.as_str()).collect();
        let group = FeatureGroup {
            id: uuid::Uuid::new_v4().to_string(),
            project_id,
            name: name.trim().to_string(),
            color: default_color(&colors).to_string(),
            position: existing.iter().map(|g| g.position + 1).max().unwrap_or(0),
            collapsed: false,
        };
        let id = group.id.clone();
        self.persist_group(&group);
        self.organize.groups.push(group);
        Ok(id)
    }

    fn group_index(&self, group_id: &str) -> Option<usize> {
        self.organize.groups.iter().position(|g| g.id == group_id)
    }

    pub fn rename_feature_group(&mut self, group_id: &str, name: &str) -> Result<()> {
        let index = self
            .group_index(group_id)
            .ok_or_else(|| anyhow::anyhow!("group not found"))?;
        let project_id = self.organize.groups[index].project_id.clone();
        let others: Vec<&str> = self
            .organize
            .groups
            .iter()
            .filter(|g| g.project_id == project_id && g.id != group_id)
            .map(|g| g.name.as_str())
            .collect();
        if let Some(error) = group_name_error(name, &others) {
            anyhow::bail!(error);
        }
        self.organize.groups[index].name = name.trim().to_string();
        let group = self.organize.groups[index].clone();
        self.persist_group(&group);
        Ok(())
    }

    pub fn recolor_feature_group(&mut self, group_id: &str) {
        if let Some(index) = self.group_index(group_id) {
            let next = next_color(&self.organize.groups[index].color).to_string();
            self.organize.groups[index].color = next;
            let group = self.organize.groups[index].clone();
            self.persist_group(&group);
        }
    }

    /// Swap the group with its neighbour (`delta` -1 up, +1 down).
    pub fn move_feature_group(&mut self, group_id: &str, delta: i64) {
        let Some(index) = self.group_index(group_id) else {
            return;
        };
        let project_id = self.organize.groups[index].project_id.clone();
        let mut ordered: Vec<(i64, String)> = self
            .organize
            .groups
            .iter()
            .filter(|g| g.project_id == project_id)
            .map(|g| (g.position, g.id.clone()))
            .collect();
        ordered.sort();
        let Some(at) = ordered.iter().position(|(_, id)| id == group_id) else {
            return;
        };
        let target = at as i64 + delta;
        if target < 0 || target as usize >= ordered.len() {
            return;
        }
        ordered.swap(at, target as usize);
        // Positions become 0..n, so a later swap never meets two equal ones.
        for (position, (_, id)) in ordered.iter().enumerate() {
            if let Some(i) = self.group_index(id) {
                self.organize.groups[i].position = position as i64;
                let group = self.organize.groups[i].clone();
                self.persist_group(&group);
            }
        }
    }

    /// Delete a group; its features become ungrouped.
    pub fn delete_feature_group(&mut self, group_id: &str) {
        self.organize.groups.retain(|g| g.id != group_id);
        self.organize.members.retain(|_, g| g != group_id);
        if let Some(db) = self.db.as_ref()
            && let Err(e) = db.delete_feature_group(group_id)
        {
            self.log_warn("groups", format!("could not delete a group: {e}"));
        }
    }

    /// Put `(pi, fi)` in `group_id`, or take it out (`None`). The group
    /// opens so the moved feature stays visible.
    pub fn set_feature_group(&mut self, pi: usize, fi: usize, group_id: Option<&str>) {
        let Some(feature_id) = self
            .store
            .projects
            .get(pi)
            .and_then(|p| p.features.get(fi))
            .map(|f| f.id.clone())
        else {
            return;
        };
        match group_id {
            Some(id) => {
                self.organize
                    .members
                    .insert(feature_id.clone(), id.to_string());
                if let Some(i) = self.group_index(id)
                    && self.organize.groups[i].collapsed
                {
                    self.organize.groups[i].collapsed = false;
                    let group = self.organize.groups[i].clone();
                    self.persist_group(&group);
                }
            }
            None => {
                self.organize.members.remove(&feature_id);
            }
        }
        if let Some(db) = self.db.as_ref()
            && let Err(e) = db.set_feature_group(&feature_id, group_id)
        {
            self.log_warn("groups", format!("could not save a group seat: {e}"));
        }
    }

    /// Collapse or expand `group_id`. A selection that the collapse hides
    /// moves to the row that now stands for the group.
    pub fn toggle_feature_group(&mut self, group_id: &str) {
        let Some(index) = self.group_index(group_id) else {
            return;
        };
        self.organize.groups[index].collapsed = !self.organize.groups[index].collapsed;
        let group = self.organize.groups[index].clone();
        self.persist_group(&group);
        if group.collapsed {
            self.reselect_after_group_collapse();
        }
    }

    /// `C`: collapse or expand the selected feature's group.
    pub fn toggle_selected_feature_group(&mut self) {
        let (pi, fi) = match self.selection {
            Selection::Feature(pi, fi) | Selection::Session(pi, fi, _) => (pi, fi),
            Selection::Project(_) => {
                self.message = Some("Select a feature in a group".into());
                return;
            }
        };
        match self.feature_group(pi, fi).map(|g| g.id.clone()) {
            Some(id) => self.toggle_feature_group(&id),
            None => self.message = Some("This feature is not in a group (G adds it)".into()),
        }
    }

    /// The group whose collapsed row `(pi, fi)` is, if it is one.
    pub fn collapsed_group_at(&self, pi: usize, fi: usize) -> Option<String> {
        (self.feature_slot(pi, fi) == FeatureSlot::CollapsedGroup)
            .then(|| self.feature_group(pi, fi).map(|g| g.id.clone()))
            .flatten()
    }

    /// The group `(pi, fi)` is in when that group is expanded.
    pub fn expanded_group_of(&self, pi: usize, fi: usize) -> Option<String> {
        self.feature_group(pi, fi)
            .filter(|g| !g.collapsed)
            .map(|g| g.id.clone())
    }

    /// After a collapse: a hidden feature or a session under a collapsed
    /// group's row selects that row instead.
    fn reselect_after_group_collapse(&mut self) {
        let (pi, fi) = match self.selection {
            Selection::Feature(pi, fi) | Selection::Session(pi, fi, _) => (pi, fi),
            Selection::Project(_) => return,
        };
        let slot = self.feature_slot(pi, fi);
        let in_session = matches!(self.selection, Selection::Session(..));
        match slot {
            FeatureSlot::Hidden => {
                let group = self.feature_group(pi, fi).map(|g| g.id.clone());
                let anchor = self
                    .project_feature_slots(pi)
                    .into_iter()
                    .find(|(i, s)| {
                        *s == FeatureSlot::CollapsedGroup
                            && self.feature_group(pi, *i).map(|g| g.id.clone()) == group
                    })
                    .map(|(i, _)| i);
                if let Some(anchor) = anchor {
                    self.selection = Selection::Feature(pi, anchor);
                }
            }
            FeatureSlot::CollapsedGroup if in_session => {
                self.selection = Selection::Feature(pi, fi);
            }
            _ => {}
        }
    }

    /// Search jumped to `(pi, fi)`: open its group.
    pub(crate) fn reveal_feature_group(&mut self, pi: usize, fi: usize) {
        if let Some(group) = self.feature_group(pi, fi).cloned()
            && group.collapsed
        {
            self.toggle_feature_group(&group.id);
        }
    }

    // ── the G dialog ─────────────────────────────────────────────

    /// `G`: the project's groups, moving the selected feature.
    pub fn open_feature_groups(&mut self) {
        let (pi, feature) = match self.selection {
            Selection::Project(pi) => (pi, None),
            Selection::Feature(pi, fi) | Selection::Session(pi, fi, _) => (pi, Some(fi)),
        };
        let Some(project) = self.store.projects.get(pi) else {
            return;
        };
        let feature_ref = feature.and_then(|fi| project.features.get(fi));
        let current = feature
            .and_then(|fi| self.feature_group(pi, fi))
            .map(|g| g.id.clone());
        let groups = self.project_groups(pi);
        let selected = match &current {
            Some(id) => groups
                .iter()
                .position(|g| g.id == *id)
                .map(|i| i + 1)
                .unwrap_or(0),
            None if feature.is_none() && !groups.is_empty() => 1,
            None => 0,
        };
        self.mode = AppMode::FeatureGroups(FeatureGroupsState {
            project_id: project.id.clone(),
            project_name: project.name.clone(),
            feature_id: feature_ref.map(|f| f.id.clone()),
            feature_name: feature_ref.map(|f| f.nickname.clone().unwrap_or_else(|| f.name.clone())),
            selected,
            edit: None,
            confirm_delete: None,
            error: None,
        });
    }

    /// The project index of the `G` dialog's project.
    pub(crate) fn groups_dialog_project(&self, state: &FeatureGroupsState) -> Option<usize> {
        self.store
            .projects
            .iter()
            .position(|p| p.id == state.project_id)
    }

    /// The rows the `G` dialog shows.
    pub fn group_dialog_rows(&self, state: &FeatureGroupsState) -> Vec<GroupRow> {
        let count = self
            .groups_dialog_project(state)
            .map(|pi| self.project_groups(pi).len())
            .unwrap_or(0);
        let mut rows = vec![GroupRow::Ungroup];
        rows.extend((0..count).map(GroupRow::Group));
        rows.push(GroupRow::New);
        rows
    }

    /// Enter on the `G` dialog's selected row.
    pub fn group_dialog_choose(&mut self) {
        let AppMode::FeatureGroups(state) = &self.mode else {
            return;
        };
        let state = state.clone();
        let rows = self.group_dialog_rows(&state);
        let Some(row) = rows.get(state.selected).copied() else {
            return;
        };
        let Some(pi) = self.groups_dialog_project(&state) else {
            self.mode = AppMode::Normal;
            return;
        };
        if row == GroupRow::New {
            if let AppMode::FeatureGroups(s) = &mut self.mode {
                s.edit = Some((GroupEdit::New, String::new()));
                s.error = None;
            }
            return;
        }
        let Some(feature_id) = state.feature_id.clone() else {
            if let AppMode::FeatureGroups(s) = &mut self.mode {
                s.error = Some(
                    "Select a feature first to move it; r renames, c recolours, d deletes".into(),
                );
            }
            return;
        };
        let Some(fi) = self.store.projects[pi]
            .features
            .iter()
            .position(|f| f.id == feature_id)
        else {
            self.mode = AppMode::Normal;
            return;
        };
        let name = state.feature_name.clone().unwrap_or_default();
        match row {
            GroupRow::Ungroup => {
                self.set_feature_group(pi, fi, None);
                self.message = Some(format!("'{name}' is no longer in a group"));
            }
            GroupRow::Group(i) => {
                let Some(group) = self.project_groups(pi).get(i).map(|g| (*g).clone()) else {
                    return;
                };
                self.set_feature_group(pi, fi, Some(&group.id));
                self.message = Some(format!("Moved '{name}' to {}", group.name));
            }
            GroupRow::New => unreachable!(),
        }
        self.selection = Selection::Feature(pi, fi);
        self.mode = AppMode::Normal;
    }

    /// Enter in the `G` dialog's name field.
    pub fn group_dialog_submit_edit(&mut self) {
        let AppMode::FeatureGroups(state) = &self.mode else {
            return;
        };
        let state = state.clone();
        let Some((edit, text)) = state.edit.clone() else {
            return;
        };
        let Some(pi) = self.groups_dialog_project(&state) else {
            self.mode = AppMode::Normal;
            return;
        };
        let result = match &edit {
            GroupEdit::New => self.create_feature_group(pi, &text).map(Some),
            GroupEdit::Rename(id) => self.rename_feature_group(id, &text).map(|_| None),
        };
        match result {
            Err(e) => {
                if let AppMode::FeatureGroups(s) = &mut self.mode {
                    s.error = Some(e.to_string());
                }
            }
            Ok(created) => {
                // A new group with a feature selected: move it there at once.
                if let Some(id) = created {
                    if let Some(feature_id) = state.feature_id.as_deref()
                        && let Some(fi) = self.store.projects[pi]
                            .features
                            .iter()
                            .position(|f| f.id == feature_id)
                    {
                        self.set_feature_group(pi, fi, Some(&id));
                        self.selection = Selection::Feature(pi, fi);
                        self.message = Some(format!(
                            "Moved '{}' to new group {}",
                            state.feature_name.clone().unwrap_or_default(),
                            text.trim()
                        ));
                        self.mode = AppMode::Normal;
                        return;
                    }
                    let index = self
                        .project_groups(pi)
                        .iter()
                        .position(|g| g.id == id)
                        .unwrap_or(0);
                    if let AppMode::FeatureGroups(s) = &mut self.mode {
                        s.selected = index + 1;
                    }
                }
                if let AppMode::FeatureGroups(s) = &mut self.mode {
                    s.edit = None;
                    s.error = None;
                }
            }
        }
    }

    /// The group on the `G` dialog's selected row.
    pub(crate) fn group_dialog_selected_group(&self) -> Option<FeatureGroup> {
        let AppMode::FeatureGroups(state) = &self.mode else {
            return None;
        };
        let rows = self.group_dialog_rows(state);
        let pi = self.groups_dialog_project(state)?;
        match rows.get(state.selected)? {
            GroupRow::Group(i) => self.project_groups(pi).get(*i).map(|g| (*g).clone()),
            _ => None,
        }
    }

    // ── notes ────────────────────────────────────────────────────

    pub fn note(&self, target: &NoteTarget) -> Option<&str> {
        self.organize.notes.get(target).map(String::as_str)
    }

    pub fn session_note(&self, session_id: &str) -> Option<&str> {
        self.note(&NoteTarget::Session(session_id.to_string()))
    }

    pub fn feature_note(&self, feature_id: &str) -> Option<&str> {
        self.note(&NoteTarget::Feature(feature_id.to_string()))
    }

    /// Save (or, when empty, delete) a note. Returns whether a note remains.
    pub fn set_note(&mut self, target: NoteTarget, text: &str) -> bool {
        let text = normalize_note(text);
        if let Some(db) = self.db.as_ref()
            && let Err(e) = db.set_note(&target, text.as_deref())
        {
            self.log_warn("notes", format!("could not save a note: {e}"));
        }
        match text {
            Some(text) => {
                self.organize.notes.insert(target, text);
                true
            }
            None => {
                self.organize.notes.remove(&target);
                false
            }
        }
    }

    /// `#`: edit the note of the selected session, or feature.
    pub fn open_note_editor(&mut self) {
        let (target, title) = match self.selection {
            Selection::Session(pi, fi, si) => {
                let Some(feature) = self.store.projects.get(pi).and_then(|p| p.features.get(fi))
                else {
                    return;
                };
                let Some(session) = feature.sessions.get(si) else {
                    return;
                };
                let name = feature
                    .nickname
                    .clone()
                    .unwrap_or_else(|| feature.name.clone());
                (
                    NoteTarget::Session(session.id.clone()),
                    format!("{} · {name}", session.label),
                )
            }
            Selection::Feature(pi, fi) => {
                let Some(feature) = self.store.projects.get(pi).and_then(|p| p.features.get(fi))
                else {
                    return;
                };
                let name = feature
                    .nickname
                    .clone()
                    .unwrap_or_else(|| feature.name.clone());
                (NoteTarget::Feature(feature.id.clone()), name)
            }
            Selection::Project(_) => {
                self.message = Some("Notes go on a feature or a session".into());
                return;
            }
        };
        let text = self.note(&target).unwrap_or_default().to_string();
        self.mode = AppMode::NoteEditor(NoteEditorState {
            target,
            title,
            original: text.clone(),
            text,
        });
    }

    /// Enter in the note editor: save and close.
    pub fn save_note_editor(&mut self) {
        let AppMode::NoteEditor(state) = &self.mode else {
            return;
        };
        let state = state.clone();
        let kept = self.set_note(state.target.clone(), &state.text);
        self.mode = AppMode::Normal;
        self.message = Some(match (kept, state.original.is_empty()) {
            (true, _) => format!("Saved the note on {}", state.title),
            (false, false) => format!("Deleted the note on {}", state.title),
            (false, true) => "No note saved".to_string(),
        });
    }

    // ── layout presets ───────────────────────────────────────────

    /// The dialog (or automation) chose `preset` for the next `count`
    /// features launched in `project`.
    pub(crate) fn queue_layout_for_launch(&mut self, project: &str, preset: &str, count: usize) {
        self.organize.pending_layout = Some(PendingLayout {
            project: project.to_string(),
            preset: preset.to_string(),
            remaining: count.max(1),
        });
    }

    /// A feature finished launching: open the pending layout in it.
    pub(crate) fn layout_feature_launched(&mut self, project_name: &str, feature_name: &str) {
        let Some(mut pending) = self.organize.pending_layout.take() else {
            return;
        };
        if pending.project != project_name {
            self.organize.pending_layout = Some(pending);
            return;
        }
        pending.remaining = pending.remaining.saturating_sub(1);
        let preset = pending.preset.clone();
        if pending.remaining > 0 {
            self.organize.pending_layout = Some(pending);
        }
        let Some((pi, fi)) = self.store.projects.iter().enumerate().find_map(|(pi, p)| {
            (p.name == project_name)
                .then(|| p.features.iter().position(|f| f.name == feature_name))
                .flatten()
                .map(|fi| (pi, fi))
        }) else {
            return;
        };
        match self.apply_layout_to_feature(pi, fi, &preset) {
            Ok(opened) if opened > 0 => {
                self.push_toast_success(format!(
                    "Layout '{preset}': opened {opened} session{} in '{feature_name}'",
                    if opened == 1 { "" } else { "s" }
                ));
            }
            Ok(_) => {}
            Err(e) => self.report_logged_error("layout", format!("{e}")),
        }
    }

    /// The layout presets the project's config defines, by name.
    pub fn layout_preset_names(&self) -> Vec<String> {
        self.active_extension
            .layout_presets
            .iter()
            .map(|p| p.name.clone())
            .collect()
    }

    /// Check a layout preset against the current (project) config.
    pub(crate) fn resolve_layout_named(
        &self,
        repo: &std::path::Path,
        preset: &str,
    ) -> std::result::Result<Vec<LayoutSession>, String> {
        let extension = self.extension_for_repo(repo);
        let Some(found) = extension.layout_preset(preset) else {
            let known: Vec<&str> = extension
                .layout_presets
                .iter()
                .map(|p| p.name.as_str())
                .collect();
            return Err(if known.is_empty() {
                format!(
                    "layout '{preset}' is not defined (no layout_presets in aiw.json or the global config)"
                )
            } else {
                format!(
                    "layout '{preset}' is not defined (known: {})",
                    known.join(", ")
                )
            });
        };
        let custom: Vec<&str> = extension
            .custom_sessions
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        let allowed = self.allowed_agents_for_repo(repo);
        resolve_layout(found, &custom, &allowed)
    }

    /// Open `preset`'s sessions in the running feature `(pi, fi)`. Returns
    /// how many opened. The selection stays where it was.
    pub(crate) fn apply_layout_to_feature(
        &mut self,
        pi: usize,
        fi: usize,
        preset: &str,
    ) -> Result<usize> {
        let (repo, feature_id, feature_name, tmux_session) = {
            let project = &self.store.projects[pi];
            let feature = &project.features[fi];
            (
                project.repo.clone(),
                feature.id.clone(),
                feature.name.clone(),
                feature.tmux_session.clone(),
            )
        };
        let sessions = self
            .resolve_layout_named(&repo, preset)
            .map_err(|e| anyhow::anyhow!(e))?;
        if !self.tmux.session_exists(&tmux_session) {
            anyhow::bail!(
                "layout '{preset}': '{feature_name}' is not running, so its sessions were not opened — start it (c), then add them with s"
            );
        }
        let extension = self.extension_for_repo(&repo);
        let selection = self.selection.clone();
        let mut opened = 0;
        let mut failures = Vec::new();
        for session in sessions {
            // A save conflict can move the feature; find it again each time.
            let Some((pi, fi)) = self.feature_indices_by_id(&feature_id) else {
                break;
            };
            let result = match &session {
                LayoutSession::Agent { agent, label } => {
                    let label = label
                        .clone()
                        .unwrap_or_else(|| self.next_agent_label(pi, fi, agent));
                    self.create_agent_session_labeled_identified(
                        pi,
                        fi,
                        &label,
                        Some(agent.clone()),
                        StartIntent::Approved,
                    )
                    .map(|_| ())
                }
                LayoutSession::Terminal { label, command } => self
                    .add_terminal_session_for_picker(pi, fi, label.clone())
                    .and_then(|session_id| match command {
                        Some(command) => self.run_in_session_window(&session_id, command),
                        None => Ok(()),
                    }),
                LayoutSession::Nvim { label } => self
                    .add_nvim_session_for_picker(pi, fi, label.clone())
                    .map(|_| ()),
                LayoutSession::Custom { name, label } => {
                    match extension.custom_sessions.iter().find(|c| c.name == *name) {
                        Some(config) => match config
                            .run_pre_check(&self.store.projects[pi].features[fi].workdir)
                        {
                            Ok(()) => self
                                .add_custom_session_type_named(
                                    pi,
                                    fi,
                                    config,
                                    label.clone().unwrap_or_else(|| config.name.clone()),
                                )
                                .map(|_| ()),
                            Err(why) => Err(anyhow::anyhow!("pre_check failed: {why}")),
                        },
                        None => Err(anyhow::anyhow!("custom session '{name}' is gone")),
                    }
                }
            };
            match result {
                Ok(()) => opened += 1,
                Err(e) => failures.push(format!("{}: {e}", describe(&session))),
            }
        }
        self.selection = selection;
        if let Err(e) = self.save() {
            self.log_warn(
                "layout",
                format!("could not save after opening a layout: {e}"),
            );
        }
        if !failures.is_empty() {
            anyhow::bail!(
                "layout '{preset}' in '{feature_name}': {} did not open — {}",
                failures.len(),
                failures.join("; ")
            );
        }
        Ok(opened)
    }

    /// `aiw automation create-feature`, with `layout` opened in each
    /// created feature once it runs. A bad layout fails the request before
    /// anything is created; a session that does not open is reported in the
    /// response message (the features exist by then).
    pub fn create_feature_from_request_organized(
        &mut self,
        request: &crate::automation::CreateFeatureRequest,
    ) -> Result<crate::automation::CreateFeatureResponse> {
        let layout = request
            .layout
            .as_deref()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string);
        if let Some(name) = &layout {
            let repo = self
                .store
                .find_project(&request.project_name)
                .map(|p| p.repo.clone())
                .ok_or_else(|| anyhow::anyhow!("Project '{}' not found", request.project_name))?;
            self.resolve_layout_named(&repo, name)
                .map_err(|e| anyhow::anyhow!(e))?;
        }
        let mut response = self.create_feature_from_request_fan_out(request)?;
        let Some(name) = layout else {
            return Ok(response);
        };
        if response.dry_run {
            response
                .message
                .push_str(&format!("; layout '{name}' would open"));
            return Ok(response);
        }
        let ids: Vec<String> = if response.features.is_empty() {
            response.feature_id.clone().into_iter().collect()
        } else {
            response
                .features
                .iter()
                .filter_map(|f| f.feature_id.clone())
                .collect()
        };
        let mut opened = 0;
        let mut problems = Vec::new();
        for id in ids {
            let Some((pi, fi)) = self.feature_indices_by_id(&id) else {
                continue;
            };
            match self.apply_layout_to_feature(pi, fi, &name) {
                Ok(n) => opened += n,
                Err(e) => problems.push(e.to_string()),
            }
        }
        response
            .message
            .push_str(&format!("; layout '{name}' opened {opened} session(s)"));
        if !problems.is_empty() {
            response
                .message
                .push_str(&format!(" ({})", problems.join("; ")));
        }
        Ok(response)
    }

    /// `Codex 2` when the feature already has `Codex 1`.
    fn next_agent_label(&self, pi: usize, fi: usize, agent: &AgentKind) -> String {
        let feature = &self.store.projects[pi].features[fi];
        let base = agent.display_name();
        (1..)
            .map(|n| format!("{base} {n}"))
            .find(|label| !feature.sessions.iter().any(|s| s.label == *label))
            .unwrap_or_else(|| format!("{base} 1"))
    }

    /// Type `command` into a session's (shell) window and run it.
    fn run_in_session_window(&mut self, session_id: &str, command: &str) -> Result<()> {
        let Some((pi, fi, si)) = self.store.projects.iter().enumerate().find_map(|(pi, p)| {
            p.features.iter().enumerate().find_map(|(fi, f)| {
                f.sessions
                    .iter()
                    .position(|s| s.id == session_id)
                    .map(|si| (pi, fi, si))
            })
        }) else {
            anyhow::bail!("the new terminal is gone");
        };
        let feature = &self.store.projects[pi].features[fi];
        let window = feature.sessions[si].tmux_window.clone();
        self.tmux
            .run_shell_command(&feature.tmux_session, &window, command)
    }
}

fn describe(session: &LayoutSession) -> String {
    match session {
        LayoutSession::Agent { agent, .. } => agent.display_name().to_string(),
        LayoutSession::Terminal { label, .. } => label.clone().unwrap_or_else(|| "terminal".into()),
        LayoutSession::Nvim { .. } => "nvim".into(),
        LayoutSession::Custom { name, .. } => name.clone(),
    }
}
