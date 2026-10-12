//! FORK: §19.8 — the watchdog: agents that exited, look stuck, sit idle with
//! something queued for them, or sit at the prompt past the context critical
//! threshold get a reason on their row, a row in the needs-you list (`i`),
//! and recovery actions on `!` (or `Enter` on that needs-you row).
//!
//! The detectors are pure (`fork::watchdog`); this module feeds them. Hook
//! activity is stamped as it arrives (the IPC handler and `hook-event`); the
//! panes of working sessions are captured at a low cadence on a helper thread
//! (never on the draw path) and reduced to a fingerprint; everything else is
//! read from what the app already knows. Nothing is done without the user,
//! except an opt-in automatic relaunch of an exited agent
//! (`watchdog_auto_restart`), at most once per session per hour.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Instant;

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};

use super::App;
use crate::app::{AppMode, PendingStart, Selection};
use crate::context_tracking::ContextBand;
use crate::fork::background::BgState;
use crate::fork::watchdog::{
    self, Observation, RESTART_DEADLINE_SECS, Thresholds, WatchAction, WatchFlag, WatchKind,
};
use crate::project::ProjectStatus;

/// Ends a stuck agent: given its pane's shell pid, terminates the shell's
/// children (the agent and whatever it runs) and leaves the shell.
pub(crate) type Killer = Arc<dyn Fn(i64) + Send + Sync>;

/// Captures one pane's visible text, for the fingerprint.
/// What one capture round sends back: a fingerprint per session (`None`
/// when its pane could not be read).
type CaptureResults = Vec<(String, Option<u64>)>;

pub(crate) type Capturer = Arc<dyn Fn(&str, &str) -> Option<String> + Send + Sync>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PaneTrack {
    fingerprint: u64,
    unchanged_since: DateTime<Utc>,
}

/// The watchdog's state, on `App::watchdog`.
pub(crate) struct WatchdogRuntime {
    /// The last hook event of any kind, per feature session.
    last_activity: HashMap<String, DateTime<Utc>>,
    /// Pane fingerprints of working sessions.
    panes: HashMap<String, PaneTrack>,
    /// Since when each running session has been seen not working.
    idle_since: HashMap<String, DateTime<Utc>>,
    /// Raised flags.
    flags: HashMap<String, WatchFlag>,
    /// Dismissed flags stay quiet until then.
    snoozed: HashMap<(String, WatchKind), DateTime<Utc>>,
    /// The last automatic restart, per session.
    auto_restarts: HashMap<String, DateTime<Utc>>,
    /// Stuck agents being stopped for a restart, with their deadline.
    pending_restarts: HashMap<String, DateTime<Utc>>,
    last_eval: Option<Instant>,
    last_capture: Option<Instant>,
    captures: Option<Receiver<CaptureResults>>,
    /// Capture panes on a helper thread. Off in tests, which feed
    /// fingerprints through `watchdog_record_pane`.
    pub(crate) capture_enabled: bool,
    pub(crate) capturer: Capturer,
    pub(crate) killer: Killer,
}

impl Default for WatchdogRuntime {
    fn default() -> Self {
        Self {
            last_activity: HashMap::new(),
            panes: HashMap::new(),
            idle_since: HashMap::new(),
            flags: HashMap::new(),
            snoozed: HashMap::new(),
            auto_restarts: HashMap::new(),
            pending_restarts: HashMap::new(),
            last_eval: None,
            last_capture: None,
            captures: None,
            capture_enabled: false,
            capturer: Arc::new(|session, window| {
                crate::tmux::TmuxManager::capture_pane_ansi(session, window).ok()
            }),
            killer: Arc::new(kill_shell_children),
        }
    }
}

impl WatchdogRuntime {
    /// The runtime of the real dashboard: panes are captured.
    pub(crate) fn production() -> Self {
        Self {
            capture_enabled: true,
            ..Self::default()
        }
    }
}

/// Terminate every child tree of `shell_pid` (SIGTERM, then SIGKILL after a
/// grace), on a helper thread. The shell itself stays, so the pane is left at
/// its prompt and the exited sweep sees it.
fn kill_shell_children(shell_pid: i64) {
    std::thread::spawn(move || {
        let processes = crate::resources::procs::list_processes();
        for child in processes.iter().filter(|p| p.ppid == shell_pid) {
            crate::resources::procs::terminate_tree(child.pid, std::time::Duration::from_secs(3));
        }
    });
}

/// The `!` dialog: what can be done about one session.
#[derive(Debug, Clone)]
pub struct WatchdogDialogState {
    pub session_id: String,
    pub label: String,
    pub feature: String,
    /// The flag's reason, when the session is flagged.
    pub reason: Option<String>,
    pub actions: Vec<WatchAction>,
    pub selected: usize,
}

impl App {
    fn watchdog_thresholds(&self) -> Thresholds {
        Thresholds::from_config(
            self.config.aiw.watchdog_stuck_minutes,
            self.config.aiw.watchdog_queued_idle_secs,
        )
    }

    /// A hook event of any kind arrived for `session_id`.
    pub(crate) fn watchdog_saw_activity(&mut self, session_id: &str) {
        self.watchdog
            .last_activity
            .insert(session_id.to_string(), Utc::now());
        if self
            .watchdog
            .flags
            .get(session_id)
            .is_some_and(|f| f.kind == WatchKind::Stuck)
        {
            self.watchdog.flags.remove(session_id);
        }
    }

    /// Record a pane fingerprint for `session_id` taken at `now` (the capture
    /// thread's results land here; tests call it directly).
    pub(crate) fn watchdog_record_pane(
        &mut self,
        session_id: &str,
        fingerprint: u64,
        now: DateTime<Utc>,
    ) {
        match self.watchdog.panes.get(session_id) {
            Some(track) if track.fingerprint == fingerprint => {}
            _ => {
                self.watchdog.panes.insert(
                    session_id.to_string(),
                    PaneTrack {
                        fingerprint,
                        unchanged_since: now,
                    },
                );
            }
        }
    }

    /// Test hook: pretend the last hook event came at `at`.
    #[cfg(test)]
    pub(crate) fn watchdog_set_activity(&mut self, session_id: &str, at: DateTime<Utc>) {
        self.watchdog
            .last_activity
            .insert(session_id.to_string(), at);
    }

    /// The flag raised on `session_id`, if any.
    pub fn watchdog_flag(&self, session_id: &str) -> Option<&WatchFlag> {
        self.watchdog.flags.get(session_id)
    }

    /// Flagged sessions as `(session id, flag)`, most urgent kind first, then
    /// oldest.
    pub fn watchdog_rows(&self) -> Vec<(String, WatchFlag)> {
        let mut rows: Vec<(String, WatchFlag)> = self
            .watchdog
            .flags
            .iter()
            .filter(|(id, _)| self.session_indices_by_id(id).is_some())
            .map(|(id, f)| (id.clone(), f.clone()))
            .collect();
        rows.sort_by(|a, b| {
            a.1.kind
                .cmp(&b.1.kind)
                .then(a.1.since.cmp(&b.1.since))
                .then(a.0.cmp(&b.0))
        });
        rows
    }

    /// What the app knows about the agent session at `(pi, fi, si)`; `None`
    /// for anything that is not an agent.
    fn watchdog_observe(&self, pi: usize, fi: usize, si: usize) -> Option<Observation> {
        let feature = self.store.projects.get(pi)?.features.get(fi)?;
        let session = feature.sessions.get(si)?;
        if !session.kind.is_agent_harness() {
            return None;
        }
        let id = session.id.as_str();
        let mut obs = Observation {
            subagents: self.subagent_count(id),
            last_activity: self.watchdog.last_activity.get(id).copied(),
            pane_unchanged_since: self.watchdog.panes.get(id).map(|p| p.unchanged_since),
            queued: self.queued_followup_count(id),
            idle_since: self.watchdog.idle_since.get(id).copied(),
            ..Default::default()
        };
        if session.is_background() {
            // A background session has no pane to paste into or watch: only
            // its failure is the watchdog's business (§19.7 shows the rest).
            if let BgState::Failed(why) = self.background_state(id) {
                obs.background_failed = Some(why);
            }
            return Some(obs);
        }
        let feature_runs = feature.status != ProjectStatus::Stopped;
        obs.exited = feature_runs
            && (self.session_is_exited(id)
                || session.status_text.as_deref()
                    == Some(crate::fork::session_reconcile::EXITED_STATUS));
        obs.running = feature_runs && !session.stopped && !obs.exited;
        obs.waiting_on_user = obs.running && self.watchdog_waiting_on_user(feature, session);
        if obs.running {
            obs.working = crate::fork::badges::session_working(self, feature, session);
            obs.context_critical = self
                .session_context_percent(id)
                .filter(|(_, band)| *band == ContextBand::Critical)
                .map(|(pct, _)| pct);
        }
        Some(obs)
    }

    /// Whether the session already waits on the user: a permission request,
    /// a question or input wait the hooks reported, an input request nothing
    /// explained, or a wait Claude's roster saw.
    fn watchdog_waiting_on_user(
        &self,
        feature: &crate::project::Feature,
        session: &crate::project::FeatureSession,
    ) -> bool {
        use crate::app::attention::AttentionState;
        self.roster_waiting_for(&session.id).is_some()
            || self
                .pending_permissions
                .iter()
                .any(|p| match &p.feature_session_id {
                    Some(id) => *id == session.id,
                    None => p.tmux_session == feature.tmux_session,
                })
            || self
                .attention_for(&feature.tmux_session)
                .is_some_and(|record| {
                    matches!(
                        record.state,
                        AttentionState::Question | AttentionState::Waiting
                    ) && crate::fork::badges::signal_applies(feature, Some(&record.agent), session)
                })
            || self.is_feature_waiting_for_input(&feature.name)
    }

    /// Re-evaluate the flags (rate-limited); capture working panes when due.
    /// Called every loop turn; returns whether anything visible changed.
    pub fn watchdog_tick(&mut self) -> bool {
        if !self.config.aiw.watchdog {
            if self.watchdog.flags.is_empty() {
                return false;
            }
            self.watchdog.flags.clear();
            return true;
        }
        let mut changed = self.watchdog_poll_captures(Utc::now());
        if self
            .watchdog
            .last_eval
            .is_some_and(|at| at.elapsed() < watchdog::EVAL_INTERVAL)
        {
            return changed;
        }
        self.watchdog.last_eval = Some(Instant::now());
        changed |= self.watchdog_tick_at(Utc::now());
        changed
    }

    /// Fold finished pane captures in.
    fn watchdog_poll_captures(&mut self, now: DateTime<Utc>) -> bool {
        let Some(rx) = self.watchdog.captures.as_ref() else {
            return false;
        };
        match rx.try_recv() {
            Ok(results) => {
                self.watchdog.captures = None;
                for (id, fingerprint) in results {
                    if let Some(fp) = fingerprint {
                        self.watchdog_record_pane(&id, fp, now);
                    }
                }
                false
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                self.watchdog.captures = None;
                false
            }
        }
    }

    /// One evaluation at `now` (tests drive this directly).
    pub(crate) fn watchdog_tick_at(&mut self, now: DateTime<Utc>) -> bool {
        let thresholds = self.watchdog_thresholds();
        let mut seen: HashSet<String> = HashSet::new();
        let mut detected: Vec<(String, Option<(WatchKind, String)>)> = Vec::new();
        let mut capture: Vec<(String, String, String)> = Vec::new();

        for pi in 0..self.store.projects.len() {
            for fi in 0..self.store.projects[pi].features.len() {
                for si in 0..self.store.projects[pi].features[fi].sessions.len() {
                    let Some(obs) = self.watchdog_observe(pi, fi, si) else {
                        continue;
                    };
                    let feature = &self.store.projects[pi].features[fi];
                    let session = &feature.sessions[si];
                    let id = session.id.clone();
                    seen.insert(id.clone());
                    // Bookkeeping the detectors read.
                    if obs.running && !obs.working {
                        self.watchdog.idle_since.entry(id.clone()).or_insert(now);
                    } else {
                        self.watchdog.idle_since.remove(&id);
                    }
                    if obs.running && obs.working && obs.subagents == 0 {
                        capture.push((
                            id.clone(),
                            feature.tmux_session.clone(),
                            session.tmux_window.clone(),
                        ));
                    } else {
                        // A new turn starts with a fresh fingerprint.
                        self.watchdog.panes.remove(&id);
                    }
                    // `idle_since` may have just been set: observe again.
                    let obs = Observation {
                        idle_since: self.watchdog.idle_since.get(&id).copied(),
                        ..obs
                    };
                    detected.push((id, watchdog::detect(&obs, &thresholds, now)));
                }
            }
        }

        let mut changed = false;
        self.watchdog.snoozed.retain(|_, until| *until > now);
        for (id, found) in detected {
            match found {
                Some((kind, detail)) => {
                    if self.watchdog.snoozed.contains_key(&(id.clone(), kind)) {
                        changed |= self
                            .watchdog
                            .flags
                            .remove(&id)
                            .is_some_and(|f| f.kind == kind);
                        continue;
                    }
                    match self.watchdog.flags.get_mut(&id) {
                        Some(flag) if flag.kind == kind => {
                            if flag.detail != detail {
                                flag.detail = detail;
                                changed = true;
                            }
                        }
                        _ => {
                            let flag = WatchFlag {
                                kind,
                                since: now,
                                detail,
                            };
                            self.watchdog_log_flag(&id, &flag);
                            self.watchdog.flags.insert(id.clone(), flag);
                            changed = true;
                        }
                    }
                }
                None => changed |= self.watchdog.flags.remove(&id).is_some(),
            }
        }
        // Forget sessions that are gone.
        let before = self.watchdog.flags.len();
        self.watchdog.flags.retain(|id, _| seen.contains(id));
        changed |= self.watchdog.flags.len() != before;
        self.watchdog
            .last_activity
            .retain(|id, _| seen.contains(id));
        self.watchdog.panes.retain(|id, _| seen.contains(id));
        self.watchdog.idle_since.retain(|id, _| seen.contains(id));

        changed |= self.watchdog_pending_restarts(now);
        changed |= self.watchdog_auto_restart(now);
        self.watchdog_start_capture(capture, &thresholds);
        changed
    }

    fn watchdog_log_flag(&mut self, session_id: &str, flag: &WatchFlag) {
        let where_ = self
            .session_indices_by_id(session_id)
            .map(|(pi, fi, si)| {
                let feature = &self.store.projects[pi].features[fi];
                format!("{} in {}", feature.sessions[si].label, feature.name)
            })
            .unwrap_or_else(|| session_id.to_string());
        self.log_info(
            "watchdog",
            format!("{where_}: {} ({})", flag.kind.label(), flag.detail),
        );
    }

    /// Capture the working panes off the UI thread, when due.
    fn watchdog_start_capture(&mut self, targets: Vec<(String, String, String)>, t: &Thresholds) {
        if !self.watchdog.capture_enabled
            || targets.is_empty()
            || self.watchdog.captures.is_some()
            || self
                .watchdog
                .last_capture
                .is_some_and(|at| at.elapsed() < watchdog::capture_interval(t))
        {
            return;
        }
        self.watchdog.last_capture = Some(Instant::now());
        let (tx, rx) = std::sync::mpsc::channel();
        let capturer = self.watchdog.capturer.clone();
        std::thread::spawn(move || {
            let results = targets
                .into_iter()
                .map(|(id, session, window)| {
                    let fp =
                        capturer(&session, &window).map(|text| watchdog::pane_fingerprint(&text));
                    (id, fp)
                })
                .collect();
            let _ = tx.send(results);
        });
        self.watchdog.captures = Some(rx);
    }

    /// Stuck agents that were stopped for a restart: relaunch each once its
    /// pane is back at the shell; give up at the deadline.
    fn watchdog_pending_restarts(&mut self, now: DateTime<Utc>) -> bool {
        if self.watchdog.pending_restarts.is_empty() {
            return false;
        }
        let mut changed = false;
        let pending: Vec<(String, DateTime<Utc>)> = self
            .watchdog
            .pending_restarts
            .iter()
            .map(|(id, at)| (id.clone(), *at))
            .collect();
        for (id, deadline) in pending {
            let Some((pi, fi, si)) = self.session_indices_by_id(&id) else {
                self.watchdog.pending_restarts.remove(&id);
                continue;
            };
            let (tmux_session, window, label) = {
                let feature = &self.store.projects[pi].features[fi];
                let session = &feature.sessions[si];
                (
                    feature.tmux_session.clone(),
                    session.tmux_window.clone(),
                    session.label.clone(),
                )
            };
            if self.exited_agent_pane(&tmux_session, &window).is_some() {
                self.watchdog.pending_restarts.remove(&id);
                self.exited_sessions.insert(id.clone());
                match self.watchdog_restart_exited(pi, fi, si) {
                    Ok(message) => self.push_toast_success(message),
                    Err(e) => self
                        .report_logged_error("watchdog", format!("could not restart {label}: {e}")),
                }
                changed = true;
            } else if now > deadline {
                self.watchdog.pending_restarts.remove(&id);
                self.report_logged_error(
                    "watchdog",
                    format!("{label} did not stop within {RESTART_DEADLINE_SECS}s; not restarted"),
                );
                changed = true;
            }
        }
        changed
    }

    /// `watchdog_auto_restart`: relaunch exited agents by themselves.
    fn watchdog_auto_restart(&mut self, now: DateTime<Utc>) -> bool {
        if !self.config.aiw.watchdog_auto_restart {
            return false;
        }
        let exited: Vec<String> = self
            .watchdog
            .flags
            .iter()
            .filter(|(_, f)| f.kind == WatchKind::Exited)
            .map(|(id, _)| id.clone())
            .collect();
        let mut changed = false;
        for id in exited {
            if !watchdog::auto_restart_allowed(self.watchdog.auto_restarts.get(&id).copied(), now) {
                continue;
            }
            let Some((pi, fi, si)) = self.session_indices_by_id(&id) else {
                continue;
            };
            let label = self.store.projects[pi].features[fi].sessions[si]
                .label
                .clone();
            if !matches!(
                self.check_start_preconditions(),
                crate::app::resource_gate::StartPreconditions::Ok
            ) {
                self.log_warn(
                    "watchdog",
                    format!("not restarting {label} by itself: the resource limits are reached"),
                );
                self.watchdog.auto_restarts.insert(id, now);
                continue;
            }
            self.watchdog.auto_restarts.insert(id.clone(), now);
            match self.restart_stopped_session_window_resuming(
                pi,
                fi,
                si,
                Some(self.watchdog_resume_id(pi, fi, si)),
            ) {
                Ok(_) => {
                    self.log_info(
                        "watchdog",
                        format!("restarted {label} by itself (watchdog_auto_restart), resuming"),
                    );
                    self.push_toast_success(format!("Watchdog restarted {label}"));
                    self.watchdog.flags.remove(&id);
                    changed = true;
                }
                Err(e) => self.report_logged_error(
                    "watchdog",
                    format!("could not restart {label} by itself: {e}"),
                ),
            }
        }
        changed
    }

    fn watchdog_resume_id(&self, pi: usize, fi: usize, si: usize) -> Option<String> {
        self.store
            .projects
            .get(pi)
            .and_then(|p| p.features.get(fi))
            .and_then(|f| f.sessions.get(si))
            .and_then(super::session_ops::persisted_resume_id)
    }

    /// Relaunch the exited agent at `(pi, fi, si)` resuming its conversation,
    /// through the resource gate.
    pub(crate) fn watchdog_restart_exited(
        &mut self,
        pi: usize,
        fi: usize,
        si: usize,
    ) -> Result<String> {
        let (id, label) = {
            let session = &self.store.projects[pi].features[fi].sessions[si];
            (session.id.clone(), session.label.clone())
        };
        if self.restart_stopped_session_window_would_start_agent(pi, fi, si)
            && self.gate_start(PendingStart::WatchdogRestart {
                session_id: id.clone(),
            })
        {
            return Ok(format!("{label}: confirm the restart first"));
        }
        self.watchdog_restart_exited_unchecked(&id)
    }

    /// The gated restart, replayed after the resource confirmation too.
    pub(crate) fn watchdog_restart_exited_unchecked(&mut self, session_id: &str) -> Result<String> {
        let (pi, fi, si) = self
            .session_indices_by_id(session_id)
            .ok_or_else(|| anyhow!("the session is gone"))?;
        let label = self.store.projects[pi].features[fi].sessions[si]
            .label
            .clone();
        let resume = self.watchdog_resume_id(pi, fi, si);
        let resumed = resume.is_some();
        self.clear_session_working_markers(session_id);
        let restarted = self.restart_stopped_session_window_resuming(pi, fi, si, Some(resume))?;
        if !restarted {
            return Err(anyhow!(
                "its window is not at the shell (or the feature is stopped: press c)"
            ));
        }
        self.watchdog.flags.remove(session_id);
        Ok(if resumed {
            format!("Restarted {label}, resuming its conversation")
        } else {
            format!("Restarted {label} (no saved conversation to resume)")
        })
    }

    /// The actions offered for `session_id`.
    pub fn watchdog_actions(&self, session_id: &str) -> Vec<WatchAction> {
        let Some((pi, fi, si)) = self.session_indices_by_id(session_id) else {
            return Vec::new();
        };
        let Some(obs) = self.watchdog_observe(pi, fi, si) else {
            return Vec::new();
        };
        let session = &self.store.projects[pi].features[fi].sessions[si];
        let feature = &self.store.projects[pi].features[fi];
        let restartable = session.is_background()
            || (feature.status != ProjectStatus::Stopped && session.kind.is_tmux_backed());
        let has_prompt = self.pending_prompt_for(session_id).is_some()
            || self.watchdog_latest_prompt(pi, fi, si).is_some();
        watchdog::actions_for(
            self.watchdog_flag(session_id).map(|f| f.kind),
            &obs,
            restartable,
            has_prompt,
        )
    }

    /// The latest prompt sent to the session: from the harness's transcripts,
    /// else `.claude/latest-prompt.txt`.
    fn watchdog_latest_prompt(&self, pi: usize, fi: usize, si: usize) -> Option<String> {
        let feature = self.store.projects.get(pi)?.features.get(fi)?;
        let session = feature.sessions.get(si)?;
        let resume = super::session_ops::persisted_resume_id(session);
        super::util::read_latest_prompt_for_session(
            &feature.workdir,
            Some(&session.kind),
            resume.as_deref(),
        )
        .or_else(|| std::fs::read_to_string(super::util::latest_prompt_path(&feature.workdir)).ok())
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
    }

    /// `!` on a session row (or `Enter` on a watchdog row of the needs-you
    /// list): the actions for that session.
    pub fn open_watchdog_dialog(&mut self, session_id: &str) {
        let Some((pi, fi, si)) = self.session_indices_by_id(session_id) else {
            return;
        };
        let feature = &self.store.projects[pi].features[fi];
        let session = &feature.sessions[si];
        if !session.kind.is_agent_harness() {
            self.message = Some("! works on agent sessions".to_string());
            return;
        }
        let actions = self.watchdog_actions(session_id);
        if actions.is_empty() {
            self.message = Some(format!("Nothing to do for {} right now", session.label));
            return;
        }
        self.mode = AppMode::Watchdog(WatchdogDialogState {
            session_id: session_id.to_string(),
            label: session.label.clone(),
            feature: feature.name.clone(),
            reason: self.watchdog_flag(session_id).map(WatchFlag::reason),
            actions,
            selected: 0,
        });
    }

    /// `!` on the dashboard: the selected session, or the feature's flagged one.
    pub fn open_watchdog_for_selection(&mut self) {
        let session_id = match self.selection {
            Selection::Session(pi, fi, si) => self
                .store
                .projects
                .get(pi)
                .and_then(|p| p.features.get(fi))
                .and_then(|f| f.sessions.get(si))
                .map(|s| s.id.clone()),
            Selection::Feature(pi, fi) => self
                .store
                .projects
                .get(pi)
                .and_then(|p| p.features.get(fi))
                .and_then(|f| {
                    f.sessions
                        .iter()
                        .find(|s| self.watchdog.flags.contains_key(&s.id))
                        .or_else(|| f.sessions.iter().find(|s| s.kind.is_agent_harness()))
                })
                .map(|s| s.id.clone()),
            _ => None,
        };
        match session_id {
            Some(id) => self.open_watchdog_dialog(&id),
            None => self.message = Some("Select an agent session for !".to_string()),
        }
    }

    /// Run the dialog's selected action and close it.
    pub fn confirm_watchdog_dialog(&mut self) {
        let AppMode::Watchdog(state) = &self.mode else {
            return;
        };
        let Some(action) = state.actions.get(state.selected).copied() else {
            return;
        };
        let session_id = state.session_id.clone();
        self.mode = AppMode::Normal;
        match self.watchdog_perform(&session_id, action) {
            Ok(message) => {
                self.log_info("watchdog", message.clone());
                self.push_toast_success(message);
            }
            Err(e) => self.report_logged_error("watchdog", format!("{}: {e}", action.label())),
        }
    }

    /// Do `action` for `session_id`. Returns what happened, for a toast.
    pub fn watchdog_perform(&mut self, session_id: &str, action: WatchAction) -> Result<String> {
        let (pi, fi, si) = self
            .session_indices_by_id(session_id)
            .ok_or_else(|| anyhow!("the session is gone"))?;
        let (tmux_session, window, label, background) = {
            let feature = &self.store.projects[pi].features[fi];
            let session = &feature.sessions[si];
            (
                feature.tmux_session.clone(),
                session.tmux_window.clone(),
                session.label.clone(),
                session.is_background(),
            )
        };
        let obs = self
            .watchdog_observe(pi, fi, si)
            .ok_or_else(|| anyhow!("{label} is not an agent session"))?;
        let need_idle = |what: &str| -> Result<()> {
            if obs.exited || !obs.running {
                return Err(anyhow!("{label} is not running; restart it first"));
            }
            if obs.working {
                return Err(anyhow!(
                    "{label} is working; {what} only goes into an agent at its prompt"
                ));
            }
            if obs.waiting_on_user {
                return Err(anyhow!(
                    "{label} is waiting on you (a question or permission); answer it first"
                ));
            }
            Ok(())
        };
        match action {
            WatchAction::Dismiss => {
                let snooze = self.watchdog_thresholds().snooze();
                let Some(flag) = self.watchdog.flags.remove(session_id) else {
                    return Ok(format!("{label} has nothing flagged"));
                };
                self.watchdog
                    .snoozed
                    .insert((session_id.to_string(), flag.kind), Utc::now() + snooze);
                Ok(format!(
                    "{label}: {} dismissed for {}",
                    flag.kind.label(),
                    watchdog::short_duration(snooze)
                ))
            }
            WatchAction::Interrupt => {
                if !obs.running {
                    return Err(anyhow!("{label} is not running"));
                }
                self.tmux
                    .send_key_name(&tmux_session, &window, "Escape")
                    .with_context(|| format!("could not press Esc in {label}"))?;
                Ok(format!("Pressed Esc in {label}"))
            }
            WatchAction::Compact => {
                need_idle("/compact")?;
                self.tmux
                    .send_literal(&tmux_session, &window, "/compact")
                    .with_context(|| format!("could not type /compact into {label}"))?;
                self.schedule_prompt_enter(session_id);
                self.watchdog.flags.remove(session_id);
                Ok(format!("Sent /compact to {label}"))
            }
            WatchAction::ReplayPrompt => {
                need_idle("a prompt")?;
                if self.pending_prompt_for(session_id).is_some() {
                    if self.deliver_pending_prompt(session_id) {
                        return Ok(format!("Delivered the waiting prompt to {label}"));
                    }
                    return Err(anyhow!("could not deliver the waiting prompt"));
                }
                let text = self
                    .watchdog_latest_prompt(pi, fi, si)
                    .ok_or_else(|| anyhow!("no earlier prompt found for {label}"))?;
                self.paste_prompt_unsent(pi, fi, si, &text)?;
                Ok(format!(
                    "Pasted the latest prompt into {label} — press Enter in its pane to send"
                ))
            }
            WatchAction::DeliverQueued => {
                need_idle("a queued item")?;
                let first = self
                    .followups_for(session_id)
                    .first()
                    .map(|f| f.text.clone())
                    .ok_or_else(|| anyhow!("nothing is queued for {label}"))?;
                // Pasted first, taken off the queue after: a failed paste
                // leaves it queued.
                self.tmux
                    .paste_text(&tmux_session, &window, &first)
                    .with_context(|| format!("could not paste into {label}"))?;
                self.take_followup(session_id)?;
                self.schedule_prompt_enter(session_id);
                self.watchdog.flags.remove(session_id);
                self.watchdog.idle_since.remove(session_id);
                let left = self.queued_followup_count(session_id);
                Ok(if left == 0 {
                    format!("Delivered the queued item to {label}")
                } else {
                    format!(
                        "Delivered the first queued item to {label}; {left} more go at its next turn end"
                    )
                })
            }
            WatchAction::RestartResume => {
                if background {
                    self.restart_background_session(pi, fi, si);
                    self.watchdog.flags.remove(session_id);
                    return Ok(format!("Restarting {label} (claude respawn)"));
                }
                if obs.exited || self.exited_agent_pane(&tmux_session, &window).is_some() {
                    self.exited_sessions.insert(session_id.to_string());
                    return self.watchdog_restart_exited(pi, fi, si);
                }
                if !obs.running {
                    return Err(anyhow!("{label} is stopped; start it with c"));
                }
                // A running (stuck) agent: end it, leave its shell, relaunch
                // resuming once the pane is back at the prompt.
                let panes = crate::fork::session_reconcile::agent_panes(self.tmux.pane_commands());
                let pid = panes
                    .get(&(tmux_session.clone(), window.clone()))
                    .and_then(|pane| pane.pane.pid)
                    .ok_or_else(|| anyhow!("could not find {label}'s pane process"))?;
                (self.watchdog.killer)(pid);
                self.clear_session_working_markers(session_id);
                self.watchdog.pending_restarts.insert(
                    session_id.to_string(),
                    Utc::now() + chrono::Duration::seconds(RESTART_DEADLINE_SECS),
                );
                self.watchdog.flags.remove(session_id);
                Ok(format!(
                    "Stopping {label}'s agent; it restarts resuming its conversation"
                ))
            }
        }
    }

    /// Forget that `session_id` is working: its agent died mid-turn (ended by
    /// the watchdog, crashed, exited) and no `Stop` will come to say so.
    /// Without this a relaunched agent would read as working — and soon as
    /// stuck — forever.
    ///
    /// The thinking hook also marks the feature's tmux session, which sync
    /// turns into the feature-level marker every agent in the feature falls
    /// back to; that goes too when no other session of the feature still has
    /// a marker of its own, or every sibling would read as working.
    pub(crate) fn clear_session_working_markers(&mut self, session_id: &str) {
        self.ipc_thinking_feature_sessions.remove(session_id);
        self.ipc_tool_feature_sessions.remove(session_id);
        self.clear_subagents(session_id);
        self.watchdog.panes.remove(session_id);
        let Some((pi, fi, si)) = self.session_indices_by_id(session_id) else {
            return;
        };
        let feature = &self.store.projects[pi].features[fi];
        let conversation = super::session_ops::persisted_resume_id(&feature.sessions[si]);
        let tmux_session = feature.tmux_session.clone();
        let sibling_marked = feature.sessions.iter().any(|s| {
            s.id != session_id
                && (self.ipc_thinking_feature_sessions.contains(&s.id)
                    || self.ipc_tool_feature_sessions.contains(&s.id))
        });
        if let Some(conversation) = conversation {
            self.ipc_thinking_sessions.remove(&conversation);
            self.ipc_tool_sessions.remove(&conversation);
        }
        if !sibling_marked {
            self.ipc_thinking_sessions.remove(&tmux_session);
            self.ipc_tool_sessions.remove(&tmux_session);
            self.thinking_features.remove(&tmux_session);
        }
    }

    /// Whether a restart of `session_id` is waiting for its agent to stop.
    pub fn watchdog_restart_pending(&self, session_id: &str) -> bool {
        self.watchdog.pending_restarts.contains_key(session_id)
    }
}
