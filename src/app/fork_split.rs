//! Split view: several live agent sessions tiled inside the TUI
//! (DESIGN.md §4.1). Pure layout/focus/key maths live in
//! `crate::fork::split`; drawing in `crate::ui::fork_split`; key and mouse
//! handling in `crate::handlers::fork_split`.
//!
//! Each *visible* tile gets the same snapshot worker the single session view
//! uses in direct mode (`App::run_pipe_pane_worker`: `pipe-pane` as a change
//! notifier, `capture-pane` for content, falling back to polling), with its
//! own latest-wins mailbox and the shared wakeup pipe, so the main loop wakes
//! the moment any tile has a new frame. Tiles on other pages have no worker.
//! While shown, each tile's tmux window is resized to the tile.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Condvar as StdCondvar, Mutex as StdMutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ratatui::layout::Rect;
use ratatui::text::Line;

use super::{
    App, AppMode, Selection, SnapshotSender, VIEW_SNAPSHOT_REFRESH_NORMAL,
    VIEW_SNAPSHOT_REFRESH_PANE_BURST, ViewSnapshot, ViewState,
};
use crate::app::attention::AttentionState;
use crate::fork::split::{self, Direction};
use crate::project::{ProjectStatus, SessionKind, VibeMode};
use crate::tmux::TmuxManager;

/// Terminal-size changes are applied to tmux windows only once the size has
/// held still this long, so dragging a window does not SIGWINCH every agent
/// on every intermediate size (same rule as the single view).
const RESIZE_DEBOUNCE: Duration = Duration::from_millis(250);

/// Unfocused tiles are captured at most this often (the focused one keeps the
/// single view's cadence) and poll no cursor: only the focused tile draws one.
/// Keeps four busy agents on screen from costing four times the tmux processes.
pub(crate) const BACKGROUND_TILE_REFRESH_INTERVAL: Duration = Duration::from_millis(250);

/// One agent session shown in a tile.
#[derive(Debug, Clone, PartialEq)]
pub struct SplitTile {
    pub project_name: String,
    /// `Feature::name` — what `ViewState` and the store key on.
    pub feature_name: String,
    /// Nickname if set, else the name: what the header shows.
    pub feature_label: String,
    pub session: String,
    pub window: String,
    pub label: String,
    pub kind: SessionKind,
    pub vibe_mode: VibeMode,
    pub review: bool,
}

/// What a tile currently shows.
#[derive(Debug, Default)]
pub struct TileFrame {
    pub lines: Vec<Line<'static>>,
    pub cursor: Option<(u16, u16)>,
    /// Content size the frame was captured at.
    pub size: (u16, u16),
    /// The tile's tmux window is gone.
    pub dead: bool,
}

/// A tile's snapshot worker thread and the handles to steer it.
struct TileWorker {
    target: (String, String, u16, u16),
    stop: Arc<AtomicBool>,
    dead: Arc<AtomicBool>,
    /// Unfocused: see [`BACKGROUND_TILE_REFRESH_INTERVAL`].
    background: Arc<AtomicBool>,
    refresh: Arc<AtomicU8>,
    condvar: Arc<(StdMutex<()>, StdCondvar)>,
    slot: Arc<StdMutex<Option<ViewSnapshot>>>,
    handle: Option<JoinHandle<()>>,
}

impl TileWorker {
    fn spawn(
        session: &str,
        window: &str,
        cols: u16,
        rows: u16,
        wakeup: Arc<std::os::fd::OwnedFd>,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let dead = Arc::new(AtomicBool::new(false));
        let background = Arc::new(AtomicBool::new(false));
        let refresh = Arc::new(AtomicU8::new(VIEW_SNAPSHOT_REFRESH_NORMAL));
        let condvar = Arc::new((StdMutex::new(()), StdCondvar::new()));
        let slot = Arc::new(StdMutex::new(None));
        let tx = SnapshotSender {
            slot: slot.clone(),
            wakeup_fd: wakeup,
        };
        let (s, w) = (session.to_string(), window.to_string());
        let (stop2, dead2, refresh2, condvar2) =
            (stop.clone(), dead.clone(), refresh.clone(), condvar.clone());
        let background2 = background.clone();
        let handle = std::thread::Builder::new()
            .name(format!("aiw-split-{s}:{w}"))
            .spawn(move || {
                if !TmuxManager::window_exists(&s, &w) {
                    dead2.store(true, Ordering::Relaxed);
                    tx.wake();
                    return;
                }
                let piped = App::run_pipe_pane_worker(
                    &s,
                    &w,
                    cols,
                    rows,
                    &stop2,
                    &refresh2,
                    &background2,
                    &condvar2,
                    &tx,
                );
                if piped.is_err() && !stop2.load(Ordering::Relaxed) {
                    if !TmuxManager::window_exists(&s, &w) {
                        dead2.store(true, Ordering::Relaxed);
                        tx.wake();
                        return;
                    }
                    App::run_capture_pane_worker(
                        &s, &w, cols, rows, &stop2, &refresh2, &condvar2, &tx,
                    );
                }
            })
            .ok();
        Self {
            target: (session.to_string(), window.to_string(), cols, rows),
            stop,
            dead,
            background,
            refresh,
            condvar,
            slot,
            handle,
        }
    }

    /// Mark the tile focused or not. Gaining focus asks for an immediate
    /// refresh, which also fetches the cursor the background cadence skipped.
    fn set_background(&self, background: bool) {
        let was = self.background.swap(background, Ordering::Relaxed);
        if was && !background {
            self.request(VIEW_SNAPSHOT_REFRESH_NORMAL);
        }
    }

    fn request(&self, kind: u8) {
        self.refresh.fetch_max(kind, Ordering::Relaxed);
        self.condvar.1.notify_one();
    }

    fn signal_stop(&self) {
        self.stop.store(true, Ordering::Relaxed);
        self.condvar.1.notify_one();
    }

    /// Wait for the thread, so its `pipe-pane` teardown cannot race a new
    /// worker (or the single view) starting on the same window.
    fn join(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for TileWorker {
    fn drop(&mut self) {
        self.signal_stop();
        self.join();
    }
}

/// `AppMode::SplitView` payload.
pub struct SplitViewState {
    /// What was opened, e.g. `RS › hx-12` or `RS (project)`.
    pub title: String,
    pub tiles: Vec<SplitTile>,
    /// Index into `tiles` of the focused tile.
    pub focus: usize,
    /// One per tile.
    pub frames: Vec<TileFrame>,
    /// Outer rects of the tiles on the current page, from the last tick
    /// (used for mouse hit testing and drawing).
    pub rects: Vec<Rect>,
    workers: Vec<Option<TileWorker>>,
    /// Size each tile's tmux window was last resized to.
    resized: Vec<Option<(u16, u16)>>,
    viewport: Option<(u16, u16)>,
    viewport_since: Instant,
}

impl SplitViewState {
    pub fn new(title: String, tiles: Vec<SplitTile>, focus: usize) -> Self {
        let n = tiles.len();
        Self {
            title,
            focus: focus.min(n.saturating_sub(1)),
            frames: (0..n).map(|_| TileFrame::default()).collect(),
            rects: Vec::new(),
            workers: (0..n).map(|_| None).collect(),
            resized: vec![None; n],
            viewport: None,
            viewport_since: Instant::now(),
            tiles,
        }
    }

    pub fn page(&self) -> usize {
        split::page_of(self.focus)
    }

    pub fn page_count(&self) -> usize {
        split::page_count(self.tiles.len())
    }

    /// Indices of the tiles on the current page.
    pub fn visible(&self) -> std::ops::Range<usize> {
        split::page_range(self.tiles.len(), self.page())
    }

    pub fn focused_tile(&self) -> Option<&SplitTile> {
        self.tiles.get(self.focus)
    }

    /// Stop every worker, in parallel, and wait for them.
    fn stop_workers(&mut self) {
        for worker in self.workers.iter().flatten() {
            worker.signal_stop();
        }
        for worker in self.workers.iter_mut() {
            if let Some(mut worker) = worker.take() {
                worker.join();
            }
        }
    }

    pub fn move_focus(&mut self, dir: Direction) {
        let visible = self.visible();
        let local = self.focus.saturating_sub(visible.start);
        if self.rects.len() != visible.len() {
            return;
        }
        self.focus = visible.start + split::move_focus(&self.rects, local, dir);
    }

    /// Flip to the next (`+1`) or previous (`-1`) page, keeping the focused
    /// slot where possible. Wraps around.
    pub fn turn_page(&mut self, delta: isize) {
        let pages = self.page_count() as isize;
        if pages <= 1 {
            return;
        }
        let slot = self.focus % split::PAGE_SIZE;
        let page = (self.page() as isize + delta).rem_euclid(pages) as usize;
        let range = split::page_range(self.tiles.len(), page);
        self.focus = (range.start + slot).min(range.end.saturating_sub(1));
        self.rects.clear();
    }

    pub fn focus_tile(&mut self, index: usize) {
        if index < self.tiles.len() {
            if split::page_of(index) != self.page() {
                self.rects.clear();
            }
            self.focus = index;
        }
    }

    /// The terminal was resized: drop the rects laid out for the old size.
    /// The renderer trusts `rects` whenever the count matches the visible
    /// tiles, so stale ones from a larger frame would be written past the
    /// new buffer; without them it recomputes from its own frame, and the
    /// next tick stores fresh ones.
    pub fn note_resize(&mut self) {
        self.rects.clear();
    }
}

impl Drop for SplitViewState {
    fn drop(&mut self) {
        self.stop_workers();
    }
}

/// Split-view state that has to outlive `AppMode::SplitView`.
#[derive(Default)]
pub struct SplitRuntime {
    /// The split view a zoomed tile returns to (`Ctrl+Space z` again).
    pub(crate) zoom_stash: Option<Box<SplitViewState>>,
    /// Split view resized tmux windows behind the single view's back; the
    /// main loop must forget the size it last applied to the single view.
    pub(crate) invalidate_view_size: bool,
}

impl SplitRuntime {
    pub fn take_view_resize_invalidation(&mut self) -> bool {
        std::mem::take(&mut self.invalidate_view_size)
    }
}

/// Header status of a tile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileStatus {
    pub text: String,
    pub kind: TileStatusKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TileStatusKind {
    NeedsYou,
    Working,
    Finished,
    Idle,
    Stopped,
}

impl TileStatusKind {
    pub fn needs_attention(self) -> bool {
        self == TileStatusKind::NeedsYou
    }
}

pub(crate) fn format_elapsed(secs: i64) -> String {
    let secs = secs.max(0);
    if secs < 60 {
        format!("0:{secs:02}")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    }
}

impl App {
    /// Agent tiles for one feature: its agent sessions whose tmux window is
    /// running, in dashboard order.
    fn split_tiles_for_feature(&self, pi: usize, fi: usize, first_only: bool) -> Vec<SplitTile> {
        let Some(project) = self.store.projects.get(pi) else {
            return Vec::new();
        };
        let Some(feature) = project.features.get(fi) else {
            return Vec::new();
        };
        if feature.status == ProjectStatus::Stopped
            || !self.tmux.session_exists(&feature.tmux_session)
        {
            return Vec::new();
        }
        let mut tiles = Vec::new();
        for session in &feature.sessions {
            if !session.kind.is_agent_harness() || session.stopped {
                continue;
            }
            if !self
                .tmux
                .window_exists(&feature.tmux_session, &session.tmux_window)
            {
                continue;
            }
            tiles.push(SplitTile {
                project_name: project.name.clone(),
                feature_name: feature.name.clone(),
                feature_label: feature
                    .nickname
                    .clone()
                    .unwrap_or_else(|| feature.name.clone()),
                session: feature.tmux_session.clone(),
                window: session.tmux_window.clone(),
                label: session.label.clone(),
                kind: session.kind.clone(),
                vibe_mode: feature.mode.clone(),
                review: feature.review,
            });
            if first_only {
                break;
            }
        }
        tiles
    }

    /// `v` on the dashboard: tile the selected feature's agents, or the first
    /// agent of every feature of the selected project.
    pub fn open_split_view(&mut self) {
        let (title, tiles, focus_window) = match self.selection.clone() {
            Selection::Feature(pi, fi) | Selection::Session(pi, fi, _) => {
                let focus_window = match self.selection {
                    Selection::Session(_, _, si) => self
                        .store
                        .projects
                        .get(pi)
                        .and_then(|p| p.features.get(fi))
                        .and_then(|f| f.sessions.get(si))
                        .map(|s| s.tmux_window.clone()),
                    _ => None,
                };
                let Some(feature) = self.store.projects.get(pi).and_then(|p| p.features.get(fi))
                else {
                    return;
                };
                let title = format!("{} › {}", self.store.projects[pi].name, feature.name);
                (
                    title,
                    self.split_tiles_for_feature(pi, fi, false),
                    focus_window,
                )
            }
            Selection::Project(pi) => {
                let Some(project) = self.store.projects.get(pi) else {
                    return;
                };
                let title = format!("{} (project)", project.name);
                let features = project.features.len();
                let tiles = (0..features)
                    .flat_map(|fi| self.split_tiles_for_feature(pi, fi, true))
                    .collect();
                (title, tiles, None)
            }
        };

        if tiles.is_empty() {
            self.message = Some(format!(
                "Split view: no running agent sessions in {title} — start it first (c)"
            ));
            return;
        }

        let focus = focus_window
            .and_then(|w| tiles.iter().position(|t| t.window == w))
            .unwrap_or(0);
        self.fork_split.zoom_stash = None;
        self.deactivate_leader();
        let count = tiles.len();
        self.mode = AppMode::SplitView(Box::new(SplitViewState::new(title, tiles, focus)));
        self.message = None;
        if count > split::PAGE_SIZE {
            self.push_toast_info(format!("{count} sessions: Ctrl+Space ] / [ for more pages"));
        }
    }

    /// Back to the dashboard (workers stop when the state is dropped).
    pub fn leave_split_view(&mut self) {
        let _ = self.flush_view_input_batch();
        self.deactivate_leader();
        self.fork_split.zoom_stash = None;
        if matches!(self.mode, AppMode::SplitView(_)) {
            self.mode = AppMode::Normal;
            self.message = Some("Returned to dashboard".into());
        }
    }

    /// `Ctrl+Space z`: show the focused tile in the regular single view.
    pub fn zoom_split_tile(&mut self) {
        let _ = self.flush_view_input_batch();
        let AppMode::SplitView(state) = std::mem::replace(&mut self.mode, AppMode::Normal) else {
            return;
        };
        let mut state = state;
        let Some(tile) = state.focused_tile().cloned() else {
            self.mode = AppMode::SplitView(state);
            return;
        };
        // The single view's own worker takes over this window.
        state.stop_workers();
        state.resized.iter_mut().for_each(|r| *r = None);
        state.rects.clear();
        self.fork_split.zoom_stash = Some(state);
        self.fork_split.invalidate_view_size = true;
        self.pane_content.clear();
        self.pane_lines.clear();
        self.tmux_cursor = None;
        self.mode = AppMode::Viewing(ViewState::new(
            tile.project_name,
            tile.feature_name,
            tile.session,
            tile.window,
            tile.label,
            tile.kind,
            tile.vibe_mode,
            tile.review,
        ));
        self.refresh_sidebar_for_current_view();
    }

    /// `Ctrl+Space z` in a view zoomed from split view: back to the tiles.
    /// Returns `false` (doing nothing) when there is no split view to return to.
    pub fn unzoom_split_tile(&mut self) -> bool {
        if !matches!(self.mode, AppMode::Viewing(_)) {
            return false;
        }
        let Some(state) = self.fork_split.zoom_stash.take() else {
            return false;
        };
        let _ = self.flush_view_input_batch();
        self.pane_content.clear();
        self.pane_lines.clear();
        self.tmux_cursor = None;
        self.fork_split.invalidate_view_size = true;
        self.mode = AppMode::SplitView(state);
        true
    }

    /// Focus the tile whose agent has needed attention the longest, turning
    /// to its page if needed.
    pub fn focus_split_attention(&mut self) {
        let AppMode::SplitView(state) = &self.mode else {
            return;
        };
        let oldest = state
            .tiles
            .iter()
            .enumerate()
            .filter_map(|(i, tile)| {
                let record = self.attention_for(&tile.session)?;
                matches!(
                    record.state,
                    AttentionState::Question | AttentionState::Waiting
                )
                .then_some((record.since, i))
            })
            .min();
        match oldest {
            Some((_, index)) => {
                if let AppMode::SplitView(state) = &mut self.mode {
                    state.focus_tile(index);
                }
            }
            None => self.push_toast_info("No tile needs you"),
        }
    }

    /// Ask the focused tile's worker for a fast refresh (after input).
    pub fn request_split_focus_refresh(&self) {
        if let AppMode::SplitView(state) = &self.mode
            && let Some(Some(worker)) = state.workers.get(state.focus)
        {
            worker.request(VIEW_SNAPSHOT_REFRESH_PANE_BURST);
        }
    }

    /// Header status of a tile, from the feature's attention/thinking state.
    /// Both are tracked per feature, so tiles of one feature share it.
    pub fn split_tile_status(&self, tile: &SplitTile) -> TileStatus {
        let feature_status = self
            .store
            .projects
            .iter()
            .flat_map(|p| p.features.iter())
            .find(|f| f.tmux_session == tile.session)
            .map(|f| f.status.clone());
        if feature_status == Some(ProjectStatus::Stopped) {
            return TileStatus {
                text: "stopped".into(),
                kind: TileStatusKind::Stopped,
            };
        }
        if let Some(record) = self.attention_for(&tile.session) {
            let elapsed = format_elapsed((chrono::Utc::now() - record.since).num_seconds());
            return match record.state {
                AttentionState::CompletedAwaitingReview => TileStatus {
                    text: format!("finished {elapsed}"),
                    kind: TileStatusKind::Finished,
                },
                AttentionState::Question => TileStatus {
                    text: format!("question {elapsed}"),
                    kind: TileStatusKind::NeedsYou,
                },
                AttentionState::Waiting => TileStatus {
                    text: format!("waiting {elapsed}"),
                    kind: TileStatusKind::NeedsYou,
                },
            };
        }
        if self.thinking_features.contains(&tile.session) {
            return TileStatus {
                text: "working".into(),
                kind: TileStatusKind::Working,
            };
        }
        TileStatus {
            text: "idle".into(),
            kind: TileStatusKind::Idle,
        }
    }

    /// Main-loop step while in split view: lay out the current page for a
    /// The terminal was resized (see [`SplitViewState::note_resize`]).
    pub fn split_view_note_resize(&mut self) {
        if let AppMode::SplitView(state) = &mut self.mode {
            state.note_resize();
        }
    }

    /// `frame`-sized terminal, resize tile windows, (re)start workers for
    /// visible tiles (stopping the rest) and take new frames. Returns whether
    /// anything on screen changed.
    pub fn split_view_tick(&mut self, frame: Rect) -> bool {
        let AppMode::SplitView(state) = &mut self.mode else {
            return false;
        };
        let mut changed = false;

        let dims = (frame.width, frame.height);
        if state.viewport != Some(dims) {
            state.viewport = Some(dims);
            state.viewport_since = Instant::now();
            changed = true;
        }
        let settled = state.viewport_since.elapsed() >= RESIZE_DEBOUNCE;

        let visible = state.visible();
        let rects = split::tile_rects(split::tiles_area(frame), visible.len());
        if state.rects != rects {
            state.rects = rects.clone();
            changed = true;
        }

        // Workers only for the visible page.
        for (i, worker) in state.workers.iter_mut().enumerate() {
            if !visible.contains(&i)
                && let Some(worker) = worker.as_ref()
            {
                worker.signal_stop();
            }
        }
        for i in (0..state.tiles.len()).filter(|i| !visible.contains(i)) {
            if let Some(mut worker) = state.workers[i].take() {
                worker.join();
            }
        }

        for (slot, i) in visible.clone().enumerate() {
            let Some(rect) = rects.get(slot) else {
                continue;
            };
            let content = split::tile_content(*rect);
            let (cols, rows) = (content.width, content.height);
            if cols == 0 || rows == 0 {
                continue;
            }
            let tile = &state.tiles[i];
            let size_applied = state.resized[i] == Some((cols, rows));
            if !size_applied && (settled || state.resized[i].is_none()) {
                let _ = self
                    .tmux
                    .resize_pane(&tile.session, &tile.window, cols, rows);
                state.resized[i] = Some((cols, rows));
                self.fork_split.invalidate_view_size = true;
            }
            // Capture at the size the window actually has.
            let Some((cols, rows)) = state.resized[i] else {
                continue;
            };
            let target = (tile.session.clone(), tile.window.clone(), cols, rows);
            let current = state.workers[i].as_ref().map(|w| &w.target);
            if current != Some(&target) {
                if let Some(mut old) = state.workers[i].take() {
                    old.signal_stop();
                    old.join();
                }
                state.workers[i] = Some(TileWorker::spawn(
                    &tile.session,
                    &tile.window,
                    cols,
                    rows,
                    self.view_snapshot_tx.wakeup_fd.clone(),
                ));
            }

            let Some(worker) = state.workers[i].as_ref() else {
                continue;
            };
            worker.set_background(i != state.focus);
            let frame_state = &mut state.frames[i];
            let dead = worker.dead.load(Ordering::Relaxed);
            if frame_state.dead != dead {
                frame_state.dead = dead;
                changed = true;
            }
            let pending = worker
                .slot
                .lock()
                .map(|mut slot| slot.take())
                .ok()
                .flatten();
            if let Some(snapshot) = pending {
                if let Some(duration) = snapshot.capture_duration {
                    self.perf
                        .record_duration("split.capture_pane_ansi", duration);
                }
                if let Some(lines) = snapshot.rendered_lines
                    && frame_state.lines != lines
                {
                    frame_state.lines = lines;
                    frame_state.size = (cols, rows);
                    changed = true;
                }
                if let Some(cursor) = snapshot.cursor
                    && frame_state.cursor != cursor
                {
                    frame_state.cursor = cursor;
                    changed = true;
                }
            }
        }
        changed
    }
}

impl SnapshotSender {
    /// Wake the main loop without a frame (e.g. a tile's window is gone).
    fn wake(&self) {
        let byte = 1u8;
        // SAFETY: writing one byte from a valid buffer to an fd we own.
        unsafe {
            libc::write(
                self.wakeup_fd.as_raw_fd(),
                &byte as *const u8 as *const _,
                1,
            )
        };
    }
}

use std::os::fd::AsRawFd;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{AgentKind, Feature, Project, ProjectStore};
    use crate::traits::{MockTmuxOps, MockWorktreeOps};
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn store(sessions: usize) -> ProjectStore {
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
        for i in 0..sessions {
            feature.add_session(if i % 2 == 0 {
                SessionKind::Claude
            } else {
                SessionKind::Codex
            });
        }
        feature.add_session(SessionKind::Terminal);
        let mut project = Project::new("RS".into(), PathBuf::from("/tmp"), true, AgentKind::Claude);
        project.features.push(feature);
        ProjectStore {
            version: 5,
            projects: vec![project],
            session_bookmarks: vec![],
            available_harnesses: vec![],
            prompt_templates: Vec::new(),
            extra: HashMap::new(),
        }
    }

    fn live_tmux() -> MockTmuxOps {
        let mut tmux = MockTmuxOps::new();
        tmux.expect_session_exists().returning(|_| true);
        tmux.expect_window_exists().returning(|_, _| true);
        tmux
    }

    pub(crate) fn app_in_split(sessions: usize) -> App {
        let mut app = App::new_for_test(
            store(sessions),
            Box::new(live_tmux()),
            Box::new(MockWorktreeOps::new()),
        );
        app.selection = Selection::Feature(0, 0);
        app.open_split_view();
        app
    }

    fn state(app: &App) -> &SplitViewState {
        match &app.mode {
            AppMode::SplitView(state) => state,
            _ => panic!("not in split view"),
        }
    }

    #[test]
    fn opens_with_the_features_running_agents_only() {
        let app = app_in_split(3);
        let state = state(&app);
        assert_eq!(state.tiles.len(), 3, "terminal session is not tiled");
        assert!(state.tiles.iter().all(|t| t.kind.is_agent_harness()));
        assert_eq!(state.title, "RS › hx-12");
        assert_eq!(state.focus, 0);
    }

    #[test]
    fn stopped_or_missing_sessions_do_not_open_split_view() {
        let mut tmux = MockTmuxOps::new();
        tmux.expect_session_exists().returning(|_| false);
        let mut app = App::new_for_test(store(2), Box::new(tmux), Box::new(MockWorktreeOps::new()));
        app.selection = Selection::Feature(0, 0);
        app.open_split_view();
        assert!(matches!(app.mode, AppMode::Normal));
        assert!(
            app.message
                .as_deref()
                .unwrap_or("")
                .contains("no running agent")
        );
    }

    #[test]
    fn selected_session_gets_the_focus() {
        let mut app = App::new_for_test(
            store(3),
            Box::new(live_tmux()),
            Box::new(MockWorktreeOps::new()),
        );
        app.selection = Selection::Session(0, 0, 2);
        app.open_split_view();
        assert_eq!(state(&app).focus, 2);
    }

    #[test]
    fn project_selection_tiles_first_agent_of_each_feature() {
        let mut store = store(2);
        let mut second = store.projects[0].features[0].clone();
        second.name = "hx-13".into();
        second.tmux_session = "aiw-hx-13".into();
        store.projects[0].features.push(second);
        let mut app = App::new_for_test(
            store,
            Box::new(live_tmux()),
            Box::new(MockWorktreeOps::new()),
        );
        app.selection = Selection::Project(0);
        app.open_split_view();
        let state = state(&app);
        assert_eq!(state.tiles.len(), 2);
        assert_eq!(state.tiles[0].session, "aiw-hx-12");
        assert_eq!(state.tiles[1].session, "aiw-hx-13");
    }

    #[test]
    fn paging_keeps_the_slot_and_wraps() {
        let mut state = SplitViewState::new("t".into(), state(&app_in_split(6)).tiles.clone(), 1);
        assert_eq!(state.page_count(), 2);
        assert_eq!(state.visible(), 0..4);
        state.turn_page(1);
        assert_eq!((state.page(), state.focus), (1, 5));
        assert_eq!(state.visible(), 4..6);
        state.turn_page(1);
        assert_eq!((state.page(), state.focus), (0, 1));
        state.turn_page(-1);
        assert_eq!(state.page(), 1);
        state.focus_tile(2);
        assert_eq!((state.page(), state.focus), (0, 2));
    }

    #[test]
    fn elapsed_formatting() {
        assert_eq!(format_elapsed(40), "0:40");
        assert_eq!(format_elapsed(6 * 60 + 5), "6m");
        assert_eq!(format_elapsed(3600 + 120), "1h02m");
        assert_eq!(format_elapsed(-5), "0:00");
    }

    #[test]
    fn tile_status_reflects_attention_and_thinking() {
        let mut app = app_in_split(1);
        let tile = state(&app).tiles[0].clone();
        assert_eq!(app.split_tile_status(&tile).kind, TileStatusKind::Idle);
        app.thinking_features.insert(tile.session.clone());
        assert_eq!(app.split_tile_status(&tile).kind, TileStatusKind::Working);
        app.record_attention(
            &tile.session,
            &AgentKind::Claude,
            AttentionState::Question,
            crate::app::attention::AttentionSource::Hook,
        );
        let status = app.split_tile_status(&tile);
        assert_eq!(status.kind, TileStatusKind::NeedsYou);
        assert!(status.text.starts_with("question"), "{}", status.text);
    }

    #[test]
    fn tick_resizes_visible_windows_to_their_tiles() {
        let mut tmux = live_tmux();
        tmux.expect_resize_pane()
            .withf(|_, _, cols, rows| (*cols, *rows) == (98, 52))
            .times(2)
            .returning(|_, _, _, _| Ok(()));
        let mut app = App::new_for_test(store(2), Box::new(tmux), Box::new(MockWorktreeOps::new()));
        app.selection = Selection::Feature(0, 0);
        app.open_split_view();
        app.split_view_tick(Rect::new(0, 0, 200, 55));
        assert_eq!(state(&app).rects.len(), 2);
        // Same size again: no second resize.
        app.split_view_tick(Rect::new(0, 0, 200, 55));
        assert!(app.fork_split.take_view_resize_invalidation());
        assert!(!app.fork_split.take_view_resize_invalidation());
    }

    // Regression: the main loop ticked with the size read before the resize
    // event, and the renderer trusted those rects (laid out for 120×40) while
    // drawing an 80×24 frame — ratatui panicked with "index outside of
    // buffer". After a resize the cached rects must not survive to the draw.
    #[test]
    fn drawing_after_a_shrink_never_writes_past_the_new_frame() {
        let mut tmux = live_tmux();
        tmux.expect_resize_pane().returning(|_, _, _, _| Ok(()));
        let mut app = App::new_for_test(store(3), Box::new(tmux), Box::new(MockWorktreeOps::new()));
        app.selection = Selection::Feature(0, 0);
        app.open_split_view();
        app.split_view_tick(Rect::new(0, 0, 200, 60));
        assert_eq!(state(&app).rects.len(), 3);

        app.split_view_note_resize();
        assert!(state(&app).rects.is_empty());

        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(80, 24)).unwrap();
        terminal
            .draw(|frame| crate::ui::draw(frame, &mut app))
            .unwrap();
        // The next tick lays out for the real frame again.
        app.split_view_tick(Rect::new(0, 0, 80, 24));
        for rect in &state(&app).rects {
            assert!(
                rect.x + rect.width <= 80 && rect.y + rect.height <= 23,
                "{rect:?}"
            );
        }
    }

    #[test]
    fn zoom_opens_the_single_view_and_unzoom_returns() {
        let mut app = app_in_split(2);
        if let AppMode::SplitView(state) = &mut app.mode {
            state.focus = 1;
        }
        app.zoom_split_tile();
        match &app.mode {
            AppMode::Viewing(view) => assert_eq!(view.window, "codex"),
            _ => panic!("zoom should open the single view"),
        }
        assert!(app.unzoom_split_tile());
        assert_eq!(state(&app).focus, 1);
        // Nothing stashed any more: a plain view is left alone.
        app.zoom_split_tile();
        app.exit_view();
        assert!(app.fork_split.zoom_stash.is_none());
        assert!(!app.unzoom_split_tile());
    }

    #[test]
    fn leaving_returns_to_the_dashboard() {
        let mut app = app_in_split(2);
        app.leave_split_view();
        assert!(matches!(app.mode, AppMode::Normal));
    }
}
