//! FORK: the workspace dashboard layout (DESIGN.md §15): a session sidebar on
//! the left and the selected session live on the right, so moving between
//! agents is moving the cursor. Drawing lives in `ui/fork_workspace.rs`, keys
//! and mouse in `handlers/fork_workspace.rs`; this file holds the runtime
//! state, the live-pane target (debounced from the selection) and the
//! geometry the main loop sizes the captured window to.
//!
//! The live pane reuses the single view's snapshot worker: while the
//! dashboard is up in this layout, `App::current_view_snapshot_target`
//! answers with the live target, so there is exactly one captured window and
//! the capture-rate rules of DESIGN.md §12.3 apply unchanged.

use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use ratatui::layout::Rect;

use super::{App, AppMode, Selection, ViewState, VisibleItem};
use crate::fork::config::DashboardLayout;
use crate::project::{ProjectStatus, SessionKind, VibeMode};

/// The live pane follows the sidebar selection once it has rested this long,
/// so scrolling through rows does not restart a capture per row.
pub const SELECTION_DEBOUNCE: Duration = Duration::from_millis(150);

/// Below this width the layout falls back to the tree (with a one-time note).
pub const MIN_WIDTH: u16 = 72;

/// Below this width the sidebar shows glyph and name only.
pub const COMPACT_BELOW: u16 = 100;

/// The footer note shown once when the terminal is too narrow for the layout.
pub const FALLBACK_NOTE: &str =
    "Workspace layout needs 72 columns — showing the tree until the window is wider";

/// Which half has the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum WorkspaceFocus {
    #[default]
    Sidebar,
    Pane,
}

/// The session shown in the live pane.
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
    pub review: bool,
}

impl LiveTarget {
    /// `project › feature › session`: the agent pane's title.
    pub fn pane_title(&self) -> String {
        format!(
            "{} › {} › {}",
            self.project_name, self.feature_label, self.label
        )
    }

    /// The single view's description of the same session, for the compose
    /// and leader rules that take one.
    pub fn view_state(&self) -> ViewState {
        ViewState::new(
            self.project_name.clone(),
            self.feature_name.clone(),
            self.session.clone(),
            self.window.clone(),
            self.label.clone(),
            self.kind.clone(),
            self.vibe_mode.clone(),
            self.review,
        )
    }
}

/// Runtime state of the layout (nothing persisted; the layout choice itself
/// is `config.dashboard_layout`).
#[derive(Default)]
pub struct WorkspaceRuntime {
    pub focus: WorkspaceFocus,
    /// What the pane shows (committed after the debounce).
    pub live: Option<LiveTarget>,
    /// What the selection asks for, and since when.
    pending: Option<(Option<LiveTarget>, Instant)>,
    /// The too-narrow note was written to the footer for this shrink.
    pub fallback_noted: bool,
    /// Where the halves were drawn last (mouse hit testing).
    pub sidebar_rect: Option<Rect>,
    pub pane_rect: Option<Rect>,
}

impl WorkspaceRuntime {
    /// Part of `App::redraw_signature`: a focus change or a new live target
    /// is a frame.
    pub fn signature(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.focus.hash(&mut hasher);
        if let Some(live) = &self.live {
            live.session.hash(&mut hasher);
            live.window.hash(&mut hasher);
        }
        hasher.finish()
    }
}

/// The sidebar's width for a dashboard `total` columns wide, or `None` when
/// the layout does not fit.
pub fn sidebar_width(total: u16) -> Option<u16> {
    if total < MIN_WIDTH {
        None
    } else if total < COMPACT_BELOW {
        Some(22)
    } else if total < 120 {
        Some(26)
    } else if total < 160 {
        Some(30)
    } else {
        Some(34)
    }
}

/// The two halves of the dashboard body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkspaceLayout {
    pub sidebar: Rect,
    pub pane: Rect,
    /// Glyph and name only in the sidebar.
    pub compact: bool,
}

pub fn layout(area: Rect) -> Option<WorkspaceLayout> {
    let width = sidebar_width(area.width)?;
    if area.height < 3 {
        return None;
    }
    Some(WorkspaceLayout {
        sidebar: Rect::new(area.x, area.y, width, area.height),
        pane: Rect::new(area.x + width, area.y, area.width - width, area.height),
        compact: area.width < COMPACT_BELOW,
    })
}

/// The live terminal inside the pane: its border taken off every side. This
/// is the size the captured window is resized to.
pub fn pane_content(pane: Rect) -> Rect {
    crate::fork::split::tile_content(pane)
}

/// The dashboard body (between the three-row header and footer) for a
/// terminal of `width` × `height`, as `ui/dashboard.rs` lays it out.
pub fn body_rect(width: u16, height: u16) -> Option<Rect> {
    if height < 11 || width == 0 {
        return None;
    }
    Some(Rect::new(0, 3, width, height - 6))
}

impl App {
    pub fn workspace_layout_enabled(&self) -> bool {
        self.config.aiw.dashboard_layout == DashboardLayout::Workspace
    }

    /// The layout is on and fits the current terminal. In the cockpit the
    /// sidebar is the whole pane, whatever its width (DESIGN.md §18).
    pub fn workspace_active(&self) -> bool {
        if self.cockpit_active() {
            return !self.store.projects.is_empty();
        }
        self.workspace_layout_enabled()
            && !self.store.projects.is_empty()
            && self.viewport_cols >= MIN_WIDTH
    }

    pub fn workspace_compact(&self) -> bool {
        // FORK(cockpit): the sidebar is a 28+ column pane; only a narrower
        // one drops to glyph and name (DESIGN.md §18).
        if self.cockpit_active() {
            return self.viewport_cols < crate::fork::cockpit::MIN_SIDEBAR_WIDTH;
        }
        self.viewport_cols < COMPACT_BELOW
    }

    /// Keys go to the agent in the pane.
    pub fn workspace_pane_focused(&self) -> bool {
        matches!(self.mode, AppMode::Normal)
            && self.workspace_active()
            && self.fork_workspace.focus == WorkspaceFocus::Pane
    }

    /// The pane is showing a captured window (the main loop treats the
    /// dashboard as a live pane then).
    pub fn workspace_pane_live(&self) -> bool {
        !self.cockpit_active() // FORK(cockpit): tmux shows the pane; nothing is captured
            && matches!(self.mode, AppMode::Normal)
            && self.workspace_active()
            && self.fork_workspace.live.is_some()
    }

    /// `b` on the dashboard: tree ⇄ workspace, saved at once.
    pub fn toggle_dashboard_layout(&mut self) {
        let next = self.config.aiw.dashboard_layout.toggled();
        self.config.aiw.dashboard_layout = next;
        self.save_config();
        if next == DashboardLayout::Tree {
            self.fork_workspace.focus = WorkspaceFocus::Sidebar;
            self.fork_workspace.live = None;
            self.fork_workspace.pending = None;
        }
        self.message = Some(format!("Layout: {} (b toggles)", next.name()));
    }

    fn live_target_for(&self, pi: usize, fi: usize, si: usize) -> Option<LiveTarget> {
        let project = self.store.projects.get(pi)?;
        let feature = project.features.get(fi)?;
        let session = feature.sessions.get(si)?;
        if !session.kind.is_tmux_backed() || feature.status == ProjectStatus::Stopped {
            return None;
        }
        // An agent that exited at its shell still has a window: show the
        // shell. A session stopped for real has nothing to capture.
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
            review: feature.review,
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

    /// What the selection asks the pane to show.
    /// Whether `Enter` has something to attach to (a running agent under
    /// the selection).
    pub fn desired_live_target_exists(&self) -> bool {
        self.desired_live_target().is_some()
    }

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
    /// [`SELECTION_DEBOUNCE`]. Returns whether the pane target changed.
    pub fn workspace_tick(&mut self) -> bool {
        // FORK(cockpit): a dialog freezes the selection; the pane keeps
        // showing the agent behind it.
        if self.cockpit_active() && !matches!(self.mode, AppMode::Normal) {
            return false;
        }
        if !self.workspace_active() || !matches!(self.mode, AppMode::Normal) {
            let had = self.fork_workspace.live.is_some() || self.fork_workspace.pending.is_some();
            self.fork_workspace.live = None;
            self.fork_workspace.pending = None;
            if !self.workspace_active() {
                self.fork_workspace.focus = WorkspaceFocus::Sidebar;
            }
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

    /// Commit a pending selection at once (the user asked for it).
    pub(crate) fn workspace_commit_pending(&mut self) {
        if let Some((target, _)) = self.fork_workspace.pending.take() {
            self.commit_live_target(target);
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
        if self.fork_workspace.live != target {
            self.fork_workspace.live = target;
            self.pane_lines.clear();
            self.tmux_cursor = None;
        }
    }

    /// When the debounce lapses, so the main loop wakes for it.
    pub fn workspace_debounce_due(&self) -> Option<Instant> {
        self.fork_workspace
            .pending
            .as_ref()
            .map(|(_, since)| *since + SELECTION_DEBOUNCE)
    }

    /// The capture target while the dashboard shows the live pane (see
    /// `App::current_view_snapshot_target`).
    pub(crate) fn workspace_snapshot_target(&self) -> Option<(String, String, u16, u16)> {
        if !self.workspace_active() || self.cockpit_active() {
            return None;
        }
        let live = self.fork_workspace.live.as_ref()?;
        if self.pane_content_cols == 0 || self.pane_content_rows == 0 {
            return None;
        }
        Some((
            live.session.clone(),
            live.window.clone(),
            self.pane_content_cols,
            self.pane_content_rows,
        ))
    }

    /// The window to size and capture for a terminal of `width` × `height`:
    /// the live target at the pane's inner size.
    pub fn workspace_capture_geometry(
        &self,
        width: u16,
        height: u16,
    ) -> Option<(String, String, u16, u16)> {
        if !matches!(self.mode, AppMode::Normal)
            || !self.workspace_layout_enabled()
            || self.cockpit_active()
        {
            return None;
        }
        let live = self.fork_workspace.live.as_ref()?;
        let content = pane_content(layout(body_rect(width, height)?)?.pane);
        if content.width == 0 || content.height == 0 {
            return None;
        }
        Some((
            live.session.clone(),
            live.window.clone(),
            content.width,
            content.height,
        ))
    }

    /// `Tab` / `Enter` / a click: keys go to the pane. A pending selection
    /// is committed at once (the user asked for it). Returns `false` when
    /// there is nothing to focus.
    pub fn workspace_focus_pane(&mut self) -> bool {
        if !self.workspace_active() {
            return false;
        }
        self.workspace_commit_pending();
        let Some(live) = self.fork_workspace.live.clone() else {
            self.fork_workspace.focus = WorkspaceFocus::Sidebar;
            return false;
        };
        self.fork_workspace.focus = WorkspaceFocus::Pane;
        self.mark_finished_seen(&live.session_id);
        self.message = None;
        self.request_view_snapshot_refresh();
        true
    }

    /// `Ctrl+Q` in the pane: back to the sidebar (not to the tree).
    pub fn workspace_focus_sidebar(&mut self) {
        let _ = self.flush_view_input_batch();
        self.deactivate_leader();
        self.fork_workspace.focus = WorkspaceFocus::Sidebar;
    }

    /// Rows a sidebar item takes: a session shows its reason on a second
    /// line while selected or needing you (never in the compact sidebar).
    pub(crate) fn workspace_item_height(&self, item: &VisibleItem) -> usize {
        if self.workspace_compact() {
            return 1;
        }
        match item {
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
            _ => 1,
        }
    }

    /// Select the sidebar row of tmux `session:window`, expanding its
    /// project and feature so the row is visible.
    pub fn workspace_select_window(&mut self, tmux_session: &str, window: &str) -> bool {
        let found = self
            .store
            .projects
            .iter()
            .enumerate()
            .find_map(|(pi, project)| {
                project
                    .features
                    .iter()
                    .enumerate()
                    .find_map(|(fi, feature)| {
                        (feature.tmux_session == tmux_session)
                            .then(|| {
                                feature
                                    .sessions
                                    .iter()
                                    .position(|s| s.tmux_window == window)
                                    .map(|si| (pi, fi, si))
                            })
                            .flatten()
                    })
            });
        let Some((pi, fi, si)) = found else {
            return false;
        };
        let project = &mut self.store.projects[pi];
        project.collapsed = false;
        project.features[fi].collapsed = false;
        self.selection = Selection::Session(pi, fi, si);
        true
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
            false,
            false,
            AgentKind::Claude,
            false,
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
        app.config.aiw.dashboard_layout = DashboardLayout::Workspace;
        app.viewport_cols = 120;
        app.viewport_rows = 37;
        app.viewport_total_rows = 40;
        app.selection = Selection::Session(0, 0, 0);
        app
    }

    #[test]
    fn sidebar_widths_step_with_the_terminal() {
        assert_eq!(sidebar_width(60), None);
        assert_eq!(sidebar_width(71), None);
        assert_eq!(sidebar_width(72), Some(22));
        assert_eq!(sidebar_width(99), Some(22));
        assert_eq!(sidebar_width(100), Some(26));
        assert_eq!(sidebar_width(120), Some(30));
        assert_eq!(sidebar_width(200), Some(34));
        let area = Rect::new(0, 3, 120, 34);
        let layout = layout(area).unwrap();
        assert_eq!(layout.sidebar, Rect::new(0, 3, 30, 34));
        assert_eq!(layout.pane, Rect::new(30, 3, 90, 34));
        assert!(!layout.compact);
        assert!(super::layout(Rect::new(0, 3, 80, 20)).unwrap().compact);
        assert_eq!(body_rect(120, 40), Some(Rect::new(0, 3, 120, 34)));
        assert_eq!(body_rect(120, 8), None);
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
        // The capture target is that one window at the pane's inner size.
        assert_eq!(
            app.workspace_capture_geometry(120, 40),
            Some(("aiw-hx-12".into(), "codex".into(), 88, 32))
        );
        app.pane_content_cols = 88;
        app.pane_content_rows = 32;
        assert_eq!(
            app.workspace_snapshot_target(),
            Some(("aiw-hx-12".into(), "codex".into(), 88, 32))
        );
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
    fn focus_toggles_and_ctrl_q_semantics() {
        let mut app = workspace_app(&[SessionKind::Claude]);
        // Focusing the pane commits a pending selection at once.
        app.workspace_tick();
        assert!(app.workspace_focus_pane());
        assert_eq!(app.fork_workspace.focus, WorkspaceFocus::Pane);
        assert!(app.fork_workspace.live.is_some());
        assert!(app.workspace_pane_focused());
        app.workspace_focus_sidebar();
        assert_eq!(app.fork_workspace.focus, WorkspaceFocus::Sidebar);
        assert!(
            matches!(app.mode, AppMode::Normal),
            "stays on the dashboard"
        );
        // Nothing to show: focus stays on the sidebar.
        app.store.projects[0].features[0].sessions[0].stopped = true;
        app.workspace_tick();
        std::thread::sleep(SELECTION_DEBOUNCE + Duration::from_millis(20));
        app.workspace_tick();
        assert!(!app.workspace_focus_pane());
        assert_eq!(app.fork_workspace.focus, WorkspaceFocus::Sidebar);
    }

    #[test]
    fn too_narrow_or_tree_layout_drops_the_pane() {
        let mut app = workspace_app(&[SessionKind::Claude]);
        app.workspace_tick();
        app.workspace_focus_pane();
        app.viewport_cols = 60;
        assert!(!app.workspace_active());
        assert!(app.workspace_tick());
        assert!(app.fork_workspace.live.is_none());
        assert_eq!(app.fork_workspace.focus, WorkspaceFocus::Sidebar);
        assert_eq!(app.workspace_capture_geometry(60, 40), None);
        app.viewport_cols = 120;
        app.toggle_dashboard_layout();
        assert_eq!(app.config.aiw.dashboard_layout, DashboardLayout::Tree);
        assert!(!app.workspace_active());
        app.toggle_dashboard_layout();
        assert!(app.workspace_active());
    }

    #[test]
    fn focusing_the_pane_marks_a_finished_turn_seen() {
        use crate::app::attention::{AttentionSource, AttentionState};
        use crate::fork::badges::BadgeKind;
        let mut app = workspace_app(&[SessionKind::Claude]);
        app.record_attention(
            "aiw-hx-12",
            &AgentKind::Claude,
            AttentionState::CompletedAwaitingReview,
            AttentionSource::Hook,
        );
        assert_eq!(
            app.session_badge_at(0, 0, 0).unwrap().kind,
            BadgeKind::FinishedUnread
        );
        app.workspace_tick();
        app.workspace_focus_pane();
        assert_eq!(
            app.session_badge_at(0, 0, 0).unwrap().kind,
            BadgeKind::FinishedRead
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
    }
}
