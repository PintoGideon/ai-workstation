//! FORK: the sidebar's selection → agent pane link (DESIGN.md §18).
//!
//! The dashboard is the cockpit's left tmux pane; the selected session's
//! *real* tmux window is shown in the right pane by `fork_cockpit`. This
//! file holds what the sidebar wants shown: the `LiveTarget` the selection
//! resolves to, debounced so holding `j` never respawns the pane per row.
//! Drawing lives in `ui/fork_workspace.rs`, keys in
//! `handlers/fork_workspace.rs`. Nothing here touches tmux.

use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use ratatui::layout::Rect;

use super::{App, AppMode, Selection, ViewState, VisibleItem};
use crate::project::{ProjectStatus, SessionKind, VibeMode};

/// The agent pane follows the sidebar selection once it has rested this
/// long, so scrolling through rows does not respawn the pane per row.
pub const SELECTION_DEBOUNCE: Duration = Duration::from_millis(150);

/// The session shown in the agent pane.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveTarget {
    pub project_name: String,
    pub feature_name: String,
    /// Nickname if set, else the name: what the title shows.
    pub feature_label: String,
    pub session: String,
    pub window: String,
    pub label: String,
    pub session_id: String,
    pub kind: SessionKind,
    pub vibe_mode: VibeMode,
}

impl LiveTarget {
    /// `project › feature › session`: the agent pane's title.
    pub fn pane_title(&self) -> String {
        format!(
            "{} › {} › {}",
            self.project_name, self.feature_label, self.label
        )
    }

    /// The same session as the dialogs describe one.
    pub fn view_state(&self) -> ViewState {
        ViewState::new(
            self.project_name.clone(),
            self.feature_name.clone(),
            self.session.clone(),
            self.window.clone(),
            self.label.clone(),
            self.kind.clone(),
            self.vibe_mode.clone(),
        )
    }
}

/// Runtime state of the link (nothing persisted).
#[derive(Default)]
pub struct WorkspaceRuntime {
    /// What the pane shows (committed after the debounce).
    pub live: Option<LiveTarget>,
    /// What the selection asks for, and since when.
    pending: Option<(Option<LiveTarget>, Instant)>,
    /// Where the sidebar was drawn last (mouse hit testing).
    pub sidebar_rect: Option<Rect>,
}

impl WorkspaceRuntime {
    /// Part of `App::redraw_signature`: a new live target is a frame.
    pub fn signature(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        if let Some(live) = &self.live {
            live.session.hash(&mut hasher);
            live.window.hash(&mut hasher);
        }
        hasher.finish()
    }
}

impl App {
    /// The sidebar has something to show.
    pub fn workspace_active(&self) -> bool {
        !self.store.projects.is_empty()
    }

    /// Glyph and name only: the sidebar pane is narrower than the floor.
    pub fn workspace_compact(&self) -> bool {
        // A viewport of 0 is "not measured yet" (tests, the first frame).
        self.viewport_cols != 0 && self.viewport_cols < crate::fork::cockpit::MIN_SIDEBAR_WIDTH
    }

    fn live_target_for(&self, pi: usize, fi: usize, si: usize) -> Option<LiveTarget> {
        let project = self.store.projects.get(pi)?;
        let feature = project.features.get(fi)?;
        let session = feature.sessions.get(si)?;
        if !session.kind.is_tmux_backed() || feature.status == ProjectStatus::Stopped {
            return None;
        }
        // An agent that exited at its shell still has a window: show the
        // shell. A session stopped for real has nothing to show.
        if session.stopped && !self.session_is_exited(&session.id) {
            return None;
        }
        Some(LiveTarget {
            project_name: project.name.clone(),
            feature_name: feature.name.clone(),
            feature_label: feature
                .nickname
                .clone()
                .unwrap_or_else(|| feature.name.clone()),
            session: feature.tmux_session.clone(),
            window: session.tmux_window.clone(),
            label: session.label.clone(),
            session_id: session.id.clone(),
            kind: session.kind.clone(),
            vibe_mode: feature.mode.clone(),
        })
    }

    /// The first running agent session of a feature.
    fn first_running_agent(&self, pi: usize, fi: usize) -> Option<LiveTarget> {
        let feature = self.store.projects.get(pi)?.features.get(fi)?;
        if feature.status == ProjectStatus::Stopped {
            return None;
        }
        let si = feature
            .sessions
            .iter()
            .position(|s| s.kind.is_agent_harness() && s.runs_with_feature())?;
        self.live_target_for(pi, fi, si)
    }

    /// Whether `Enter` has an agent pane to focus (a running agent under
    /// the selection).
    pub fn desired_live_target_exists(&self) -> bool {
        self.desired_live_target().is_some()
    }

    /// What the selection asks the pane to show.
    pub(super) fn desired_live_target(&self) -> Option<LiveTarget> {
        match self.selection {
            Selection::Session(pi, fi, si) => self.live_target_for(pi, fi, si),
            Selection::Feature(pi, fi) => self.first_running_agent(pi, fi),
            Selection::Project(pi) => {
                let project = self.store.projects.get(pi)?;
                (0..project.features.len()).find_map(|fi| self.first_running_agent(pi, fi))
            }
        }
    }

    /// Main-loop step: follow the selection once it has rested for
    /// [`SELECTION_DEBOUNCE`]. A dialog freezes the selection; the pane
    /// keeps showing the agent behind it. Returns whether the target changed.
    pub fn workspace_tick(&mut self) -> bool {
        if !matches!(self.mode, AppMode::Normal) {
            return false;
        }
        if !self.workspace_active() {
            let had = self.fork_workspace.live.is_some() || self.fork_workspace.pending.is_some();
            self.fork_workspace.live = None;
            self.fork_workspace.pending = None;
            return had;
        }
        let desired = self.desired_live_target();
        if desired == self.fork_workspace.live {
            self.fork_workspace.pending = None;
            return false;
        }
        match &self.fork_workspace.pending {
            Some((target, since)) if *target == desired => {
                if since.elapsed() >= SELECTION_DEBOUNCE {
                    self.commit_live_target(desired);
                    return true;
                }
                false
            }
            _ => {
                self.fork_workspace.pending = Some((desired, Instant::now()));
                false
            }
        }
    }

    /// Commit what the selection asks for right now, debounce or not (the
    /// user acted on the row: `Enter`, a pasted prompt).
    pub(crate) fn workspace_commit_now(&mut self) {
        if !self.workspace_active() {
            return;
        }
        let desired = self.desired_live_target();
        self.commit_live_target(desired);
    }

    fn commit_live_target(&mut self, target: Option<LiveTarget>) {
        self.fork_workspace.pending = None;
        self.fork_workspace.live = target;
    }

    /// When the debounce lapses, so the main loop wakes for it.
    pub fn workspace_debounce_due(&self) -> Option<Instant> {
        self.fork_workspace
            .pending
            .as_ref()
            .map(|(_, since)| *since + SELECTION_DEBOUNCE)
    }

    /// Rows a sidebar item takes: a session shows its reason on a second
    /// line while selected or needing you (never in the compact sidebar).
    pub(crate) fn workspace_item_height(&self, item: &VisibleItem) -> usize {
        if self.workspace_compact() {
            return 1;
        }
        match item {
            VisibleItem::Project(pi) => {
                let project = &self.store.projects[*pi];
                let base = if project.features.is_empty() && !project.collapsed {
                    2
                } else {
                    1
                };
                // FORK: §19.2 — external Claude sessions in the repo.
                let external = if project.collapsed {
                    0
                } else {
                    super::fork_identity::external_line_count(
                        self.external_sessions_for_project(&project.name).len(),
                    ) + super::fork_claude_worktrees::line_count(
                        self.claude_worktrees_for_project(&project.name).len(),
                    )
                };
                base + external
            }
            VisibleItem::Feature(pi, fi) => {
                // FORK: §19.2 — external Claude sessions in the checkout.
                let feature = &self.store.projects[*pi].features[*fi];
                // FORK: §19.4 — a collapsed group is one row; an expanded
                // one has its header above its first feature.
                let slot = self.feature_slot(*pi, *fi);
                if slot == crate::fork::organize::FeatureSlot::CollapsedGroup {
                    return 1;
                }
                let group_header =
                    usize::from(slot == crate::fork::organize::FeatureSlot::GroupHead);
                // FORK: §19.3 — the fan-out group header above its first
                // sibling.
                let header = group_header + usize::from(self.fan_out_heads_group(*pi, *fi));
                if feature.collapsed {
                    1 + header
                } else {
                    1 + header
                        + super::fork_identity::external_line_count(
                            self.external_sessions_for_feature(&feature.id).len(),
                        )
                        + super::fork_claude_worktrees::line_count(
                            self.claude_worktrees_for_feature(&feature.id).len(),
                        )
                }
            }
            VisibleItem::Session(pi, fi, si) => {
                let selected = matches!(
                    self.selection,
                    Selection::Session(a, b, c) if a == *pi && b == *fi && c == *si
                );
                if selected {
                    return 2;
                }
                match self.session_badge_at(*pi, *fi, *si) {
                    Some(badge) if badge.kind.needs_you() => 2,
                    _ => 1,
                }
            }
        }
    }

    /// Select the sidebar row of tmux `session:window`, expanding its
    /// project and feature so the row is visible.
    pub fn workspace_select_window(&mut self, tmux_session: &str, window: &str) -> bool {
        let Some((pi, fi, si)) = self.session_indices_by_window(tmux_session, window) else {
            return false;
        };
        let project = &mut self.store.projects[pi];
        project.collapsed = false;
        project.features[fi].collapsed = false;
        self.selection = Selection::Session(pi, fi, si);
        true
    }

    /// Back on the dashboard, the row on the session a dialog was about:
    /// what every "return to the view" path does now that the agent's real
    /// pane is the view.
    pub fn return_to_dashboard(&mut self, view: &ViewState) {
        self.mode = AppMode::Normal;
        if self.workspace_select_window(&view.session, &view.window) {
            return;
        }
        // The window is gone (or was never a window): the feature's row.
        let found = self.store.projects.iter().enumerate().find_map(|(pi, p)| {
            p.features
                .iter()
                .position(|f| f.tmux_session == view.session)
                .map(|fi| (pi, fi))
        });
        if let Some((pi, fi)) = found {
            self.store.projects[pi].collapsed = false;
            self.selection = Selection::Feature(pi, fi);
        }
    }
}

#[cfg(test)]
pub(crate) use tests::workspace_app;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{AgentKind, Feature, Project, ProjectStore};
    use crate::traits::{MockTmuxOps, MockWorktreeOps};
    use std::path::PathBuf;

    pub(crate) fn workspace_app(sessions: &[SessionKind]) -> App {
        let mut feature = Feature::new(
            "hx-12".into(),
            "hx-12".into(),
            PathBuf::from("/tmp"),
            false,
            VibeMode::Vibeless,
            AgentKind::Claude,
            false,
        );
        feature.status = ProjectStatus::Active;
        for kind in sessions {
            feature.add_session(kind.clone());
        }
        let mut project = Project::new("RS".into(), PathBuf::from("/tmp"), true, AgentKind::Claude);
        project.features.push(feature);
        let store = ProjectStore {
            version: 5,
            projects: vec![project],
            session_bookmarks: vec![],
            available_harnesses: vec![],
            prompt_templates: Vec::new(),
            extra: Default::default(),
        };
        let mut app = App::new_for_test(
            store,
            Box::new(MockTmuxOps::new()),
            Box::new(MockWorktreeOps::new()),
        );
        app.viewport_cols = 36;
        app.viewport_rows = 57;
        app.viewport_total_rows = 60;
        app.selection = Selection::Session(0, 0, 0);
        app
    }

    #[test]
    fn the_pane_follows_the_selection_after_the_debounce() {
        let mut app = workspace_app(&[SessionKind::Claude, SessionKind::Codex]);
        assert!(!app.workspace_tick(), "first tick only arms the debounce");
        assert!(app.fork_workspace.live.is_none());
        assert!(app.workspace_debounce_due().is_some());
        std::thread::sleep(SELECTION_DEBOUNCE + Duration::from_millis(20));
        assert!(app.workspace_tick());
        assert_eq!(
            app.fork_workspace.live.as_ref().map(|t| t.window.as_str()),
            Some("claude")
        );
        // Moving on: the old target stays until the new one has rested.
        app.selection = Selection::Session(0, 0, 1);
        assert!(!app.workspace_tick());
        assert_eq!(
            app.fork_workspace.live.as_ref().map(|t| t.window.as_str()),
            Some("claude")
        );
        app.selection = Selection::Session(0, 0, 0);
        assert!(
            !app.workspace_tick(),
            "back on the live row: nothing pending"
        );
        assert!(app.workspace_debounce_due().is_none());
        app.selection = Selection::Session(0, 0, 1);
        app.workspace_tick();
        std::thread::sleep(SELECTION_DEBOUNCE + Duration::from_millis(20));
        assert!(app.workspace_tick());
        assert_eq!(
            app.fork_workspace.live.as_ref().map(|t| t.window.as_str()),
            Some("codex")
        );
        // Acting on a row commits at once.
        app.selection = Selection::Session(0, 0, 0);
        app.workspace_commit_now();
        assert_eq!(
            app.fork_workspace.live.as_ref().map(|t| t.window.as_str()),
            Some("claude")
        );
        assert_eq!(
            app.fork_workspace.live.as_ref().map(|t| t.pane_title()),
            Some("RS › hx-12 › Claude 1".into())
        );
    }

    #[test]
    fn a_dialog_freezes_the_pane_and_an_empty_store_drops_it() {
        let mut app = workspace_app(&[SessionKind::Claude]);
        app.workspace_commit_now();
        assert!(app.fork_workspace.live.is_some());
        app.mode = AppMode::Help(crate::app::HelpState {
            from_view: None,
            scroll_offset: 0,
        });
        app.selection = Selection::Project(0);
        assert!(!app.workspace_tick());
        assert_eq!(
            app.fork_workspace.live.as_ref().map(|t| t.window.as_str()),
            Some("claude"),
            "the pane keeps the agent behind the dialog"
        );
        app.mode = AppMode::Normal;
        app.store.projects.clear();
        assert!(!app.workspace_active());
        assert!(app.workspace_tick());
        assert!(app.fork_workspace.live.is_none());
    }

    #[test]
    fn feature_and_project_rows_show_the_first_running_agent() {
        let mut app = workspace_app(&[
            SessionKind::Terminal,
            SessionKind::Claude,
            SessionKind::Codex,
        ]);
        app.selection = Selection::Feature(0, 0);
        assert_eq!(
            app.desired_live_target().map(|t| t.window),
            Some("claude".into())
        );
        app.store.projects[0].features[0].sessions[1].stopped = true;
        assert_eq!(
            app.desired_live_target().map(|t| t.window),
            Some("codex".into())
        );
        app.selection = Selection::Project(0);
        assert_eq!(
            app.desired_live_target().map(|t| t.window),
            Some("codex".into())
        );
        app.store.projects[0].features[0].status = ProjectStatus::Stopped;
        assert_eq!(app.desired_live_target(), None);
    }

    #[test]
    fn a_stopped_session_has_no_pane_but_an_exited_one_shows_its_shell() {
        let mut app = workspace_app(&[SessionKind::Claude]);
        app.store.projects[0].features[0].sessions[0].stopped = true;
        assert_eq!(app.desired_live_target(), None);
        let id = app.store.projects[0].features[0].sessions[0].id.clone();
        app.exited_sessions.insert(id);
        assert_eq!(
            app.desired_live_target().map(|t| t.window),
            Some("claude".into())
        );
    }

    #[test]
    fn the_compact_sidebar_is_below_the_width_floor() {
        let mut app = workspace_app(&[SessionKind::Claude]);
        assert!(!app.workspace_compact());
        app.viewport_cols = 27;
        assert!(app.workspace_compact());
        assert_eq!(app.workspace_item_height(&VisibleItem::Session(0, 0, 0)), 1);
        app.viewport_cols = 36;
        assert_eq!(
            app.workspace_item_height(&VisibleItem::Session(0, 0, 0)),
            2,
            "the selected row shows its reason line"
        );
    }

    #[test]
    fn selecting_by_window_expands_the_path_to_the_row() {
        let mut app = workspace_app(&[SessionKind::Claude, SessionKind::Codex]);
        app.store.projects[0].collapsed = true;
        app.store.projects[0].features[0].collapsed = true;
        assert!(app.workspace_select_window("aiw-hx-12", "codex"));
        assert!(matches!(app.selection, Selection::Session(0, 0, 1)));
        assert!(!app.store.projects[0].collapsed);
        assert!(!app.store.projects[0].features[0].collapsed);
        assert!(!app.workspace_select_window("aiw-none", "codex"));
        // A dialog's "back to the session" lands on the dashboard row.
        app.mode = AppMode::Help(crate::app::HelpState {
            from_view: None,
            scroll_offset: 0,
        });
        let view = app.desired_live_target().unwrap().view_state();
        app.selection = Selection::Project(0);
        app.return_to_dashboard(&view);
        assert!(matches!(app.mode, AppMode::Normal));
        assert!(matches!(app.selection, Selection::Session(0, 0, 1)));
    }
}
