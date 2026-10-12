//! FORK: §19.9 — scheduled and recurring prompts and commands.
//!
//! A schedule belongs to a feature. A *prompt* schedule delivers its text to
//! an agent session (pinned, by label, or the feature's first agent) the way
//! a follow-up goes: pasted when the agent is idle at its prompt with nothing
//! else waiting, queued for its turn end when it works, deferred while it
//! waits on you. A *shell* schedule types a command into a terminal of the
//! feature (created when missing), only when that terminal is at its prompt.
//!
//! The timer rides on the app's loop (`schedules_tick`, every
//! [`TICK_INTERVAL`]); next fire times are computed from the wall clock each
//! time (`fork::schedules`), so clock changes and DST need no bookkeeping.
//! Fires missed while AIW was not running run once on start (`catch_up:
//! once`, the default) or are recorded as missed. A fire never stacks on a
//! previous one still queued.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Instant;

use anyhow::{Result, anyhow};
use chrono::{DateTime, Local, TimeZone, Utc};
use serde_json::Value;

use super::App;
use crate::app::{AppMode, Selection};
use crate::fork::messaging::Delivery;
use crate::fork::schedules::{
    self, CONFIG_SYNC_INTERVAL, CatchUp, DEFAULT_TERMINAL, DEFER_RETRY_SECS, Due,
    ONE_SHOT_DEFER_LIMIT_SECS, Outcome, RunRecord, SCHEDULE_ACTION, Schedule, ScheduleKind,
    TICK_INTERVAL,
};
use crate::project::{ProjectStatus, SessionKind};

/// In-memory history kept per schedule (the database keeps [`schedules::HISTORY_CAP`]).
const MEMORY_HISTORY: usize = 20;

/// Why a fire happened, for the history line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    Timer,
    CatchUp,
    RunNow,
    Retry,
    Automation,
}

impl Trigger {
    fn label(self) -> &'static str {
        match self {
            Self::Timer => "timer",
            Self::CatchUp => "catch-up",
            Self::RunNow => "run now",
            Self::Retry => "after waiting on you",
            Self::Automation => "automation",
        }
    }
}

/// A fire put off because the agent waits on you.
#[derive(Debug, Clone, Copy)]
struct Deferred {
    since: DateTime<Utc>,
    last_try: DateTime<Utc>,
    /// The slot that came due.
    slot: DateTime<Utc>,
}

/// The schedules' state, on `App::schedules`.
pub(crate) struct ScheduleRuntime {
    pub(crate) items: Vec<Schedule>,
    history: HashMap<String, VecDeque<RunRecord>>,
    deferred: HashMap<String, Deferred>,
    /// When AIW started: fires due before it were missed.
    pub(crate) started: DateTime<Utc>,
    last_tick: Option<Instant>,
    last_config_sync: Option<Instant>,
    /// `aiw.json` problems already logged.
    config_warnings: HashSet<String>,
}

impl Default for ScheduleRuntime {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            history: HashMap::new(),
            deferred: HashMap::new(),
            started: Utc::now(),
            last_tick: None,
            last_config_sync: None,
            config_warnings: HashSet::new(),
        }
    }
}

/// What the schedules list shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleScope {
    Session {
        feature_id: String,
        session_id: String,
    },
    Feature {
        feature_id: String,
    },
    Project {
        project_id: String,
    },
}

/// `t`: the schedules of a session, a feature or a project.
#[derive(Debug, Clone)]
pub struct ScheduleListState {
    pub scope: ScheduleScope,
    pub title: String,
    pub selected: usize,
    pub show_history: bool,
    pub confirm_delete: bool,
}

/// One row of the list, ready to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct ScheduleRow {
    pub id: String,
    pub spec: String,
    pub target: String,
    pub feature: String,
    pub text: String,
    pub next: Option<String>,
    pub last: Option<String>,
    pub paused: bool,
    pub finished: bool,
    pub from_config: bool,
    pub deferred: bool,
}

/// The fields of the add / edit dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorField {
    Kind,
    Target,
    When,
    Text,
    StartIfStopped,
    CatchUp,
    MaxRuns,
    End,
}

impl EditorField {
    pub const ALL: [Self; 8] = [
        Self::Kind,
        Self::Target,
        Self::When,
        Self::Text,
        Self::StartIfStopped,
        Self::CatchUp,
        Self::MaxRuns,
        Self::End,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Kind => "Does",
            Self::Target => "Target",
            Self::When => "When",
            Self::Text => "Text",
            Self::StartIfStopped => "If stopped",
            Self::CatchUp => "Missed runs",
            Self::MaxRuns => "Max runs",
            Self::End => "Ends",
        }
    }
}

/// The add / edit dialog.
#[derive(Debug, Clone)]
pub struct ScheduleEditorState {
    /// The schedule being edited (`None`: a new one).
    pub editing: Option<String>,
    pub feature_id: String,
    pub feature: String,
    pub kind: ScheduleKind,
    /// Prompt targets: `(pinned session id, label)`; index 0 is "the
    /// feature's first agent" (`None`).
    pub targets: Vec<(Option<String>, String)>,
    pub target_index: usize,
    /// Shell: the terminal's name.
    pub terminal: String,
    pub spec: String,
    pub text: String,
    pub start_if_stopped: bool,
    pub catch_up: CatchUp,
    pub max_runs: String,
    pub end: String,
    pub focus: EditorField,
    pub error: Option<String>,
    /// The list to go back to.
    pub list: ScheduleListState,
}

impl ScheduleEditorState {
    pub fn next_field(&mut self) {
        let i = EditorField::ALL
            .iter()
            .position(|f| *f == self.focus)
            .unwrap_or(0);
        self.focus = EditorField::ALL[(i + 1) % EditorField::ALL.len()];
    }

    pub fn prev_field(&mut self) {
        let i = EditorField::ALL
            .iter()
            .position(|f| *f == self.focus)
            .unwrap_or(0);
        self.focus = EditorField::ALL[(i + EditorField::ALL.len() - 1) % EditorField::ALL.len()];
    }

    /// Left / right / space on a choice field.
    pub fn cycle(&mut self, forward: bool) {
        match self.focus {
            EditorField::Kind => {
                self.kind = match self.kind {
                    ScheduleKind::Prompt => ScheduleKind::Shell,
                    ScheduleKind::Shell => ScheduleKind::Prompt,
                }
            }
            EditorField::Target
                if self.kind == ScheduleKind::Prompt && !self.targets.is_empty() =>
            {
                let n = self.targets.len();
                self.target_index = if forward {
                    (self.target_index + 1) % n
                } else {
                    (self.target_index + n - 1) % n
                };
            }
            EditorField::StartIfStopped => self.start_if_stopped = !self.start_if_stopped,
            EditorField::CatchUp => {
                self.catch_up = match self.catch_up {
                    CatchUp::Once => CatchUp::None,
                    CatchUp::None => CatchUp::Once,
                }
            }
            _ => {}
        }
    }

    /// The text field under focus, when it is one.
    pub fn field_text_mut(&mut self) -> Option<&mut String> {
        match self.focus {
            EditorField::Target if self.kind == ScheduleKind::Shell => Some(&mut self.terminal),
            EditorField::When => Some(&mut self.spec),
            EditorField::Text => Some(&mut self.text),
            EditorField::MaxRuns => Some(&mut self.max_runs),
            EditorField::End => Some(&mut self.end),
            _ => None,
        }
    }
}

/// Short id for messages: the first 8 characters.
fn short(id: &str) -> &str {
    let end = id.char_indices().nth(8).map(|(i, _)| i).unwrap_or(id.len());
    &id[..end]
}

impl App {
    fn schedule_min_interval(&self) -> u64 {
        self.config.aiw.schedule_min_interval_secs.max(1)
    }

    // ── load / persist ───────────────────────────────────────────

    /// Load the schedules (start-up).
    pub(crate) fn load_schedules(&mut self) {
        let Some(db) = self.db.as_ref() else {
            return;
        };
        match db.all_schedules() {
            Ok(items) => self.schedules.items = items,
            Err(e) => self.log_warn("schedule", format!("could not load schedules: {e}")),
        }
    }

    fn persist_schedule(&mut self, id: &str) {
        let Some(schedule) = self.schedules.items.iter().find(|s| s.id == id).cloned() else {
            return;
        };
        if let Some(db) = self.db.as_ref()
            && let Err(e) = db.save_schedule(&schedule)
        {
            self.log_warn(
                "schedule",
                format!("could not save schedule {}: {e}", short(id)),
            );
        }
    }

    fn schedule_mut(&mut self, id: &str) -> Option<&mut Schedule> {
        self.schedules.items.iter_mut().find(|s| s.id == id)
    }

    pub fn schedule(&self, id: &str) -> Option<&Schedule> {
        self.schedules.items.iter().find(|s| s.id == id)
    }

    /// The schedule's runs, newest first (database, else memory).
    pub fn schedule_history(&self, id: &str) -> Vec<RunRecord> {
        if let Some(db) = self.db.as_ref()
            && let Ok(runs) = db.schedule_runs(id)
        {
            return runs;
        }
        self.schedules
            .history
            .get(id)
            .map(|h| h.iter().cloned().collect())
            .unwrap_or_default()
    }

    fn last_run(&self, id: &str) -> Option<RunRecord> {
        if let Some(run) = self.schedules.history.get(id).and_then(|h| h.front()) {
            return Some(run.clone());
        }
        self.db
            .as_ref()
            .and_then(|db| db.schedule_runs(id).ok())
            .and_then(|runs| runs.into_iter().next())
    }

    fn record_schedule_run(
        &mut self,
        id: &str,
        outcome: &Outcome,
        detail: String,
        at: DateTime<Utc>,
    ) {
        let run = RunRecord {
            at,
            outcome: outcome.label(),
            detail,
        };
        let history = self.schedules.history.entry(id.to_string()).or_default();
        history.push_front(run.clone());
        history.truncate(MEMORY_HISTORY);
        if let Some(db) = self.db.as_ref()
            && let Err(e) = db.push_schedule_run(id, &run)
        {
            self.log_warn(
                "schedule",
                format!("could not save a run of {}: {e}", short(id)),
            );
        }
        let line = format!(
            "{} {}: {}{}",
            short(id),
            self.schedule(id)
                .map(|s| s.spec.clone())
                .unwrap_or_default(),
            run.outcome,
            if run.detail.is_empty() {
                String::new()
            } else {
                format!(" ({})", run.detail)
            }
        );
        match outcome {
            Outcome::Failed(_) => {
                self.log_warn("schedule", line.clone());
                self.push_toast_error(format!("Schedule {line}"));
            }
            _ => self.log_info("schedule", line),
        }
    }

    /// Forget the schedules of a deleted session.
    pub(crate) fn forget_schedules_for_session(&mut self, session_id: &str) {
        let ids: Vec<String> = self
            .schedules
            .items
            .iter()
            .filter(|s| s.session_id.as_deref() == Some(session_id))
            .map(|s| s.id.clone())
            .collect();
        for id in ids {
            self.drop_schedule(&id);
        }
    }

    /// Forget the schedules of a deleted feature.
    pub(crate) fn forget_schedules_for_feature(&mut self, feature_id: &str) {
        let ids: Vec<String> = self
            .schedules
            .items
            .iter()
            .filter(|s| s.feature_id == feature_id)
            .map(|s| s.id.clone())
            .collect();
        for id in ids {
            self.drop_schedule(&id);
        }
    }

    fn drop_schedule(&mut self, id: &str) {
        self.schedules.items.retain(|s| s.id != id);
        self.schedules.history.remove(id);
        self.schedules.deferred.remove(id);
        if let Some(db) = self.db.as_ref()
            && let Err(e) = db.delete_schedule(id)
        {
            self.log_warn(
                "schedule",
                format!("could not delete schedule {}: {e}", short(id)),
            );
        }
    }

    // ── queries ──────────────────────────────────────────────────

    /// The agent session a prompt schedule goes to right now.
    fn schedule_target_session(&self, s: &Schedule) -> Option<(usize, usize, usize)> {
        if let Some(id) = &s.session_id {
            return self.session_indices_by_id(id);
        }
        let (pi, fi) = self.feature_indices_by_id(&s.feature_id)?;
        let feature = &self.store.projects[pi].features[fi];
        let si = match &s.target {
            Some(label) => feature
                .sessions
                .iter()
                .position(|x| x.label == *label && x.kind.is_agent_harness())?,
            None => feature
                .sessions
                .iter()
                .position(|x| x.kind.is_agent_harness())?,
        };
        Some((pi, fi, si))
    }

    fn schedule_target_text(&self, s: &Schedule) -> String {
        let label = match s.kind {
            ScheduleKind::Prompt => s
                .session_id
                .as_ref()
                .and_then(|id| self.session_indices_by_id(id))
                .map(|(pi, fi, si)| {
                    self.store.projects[pi].features[fi].sessions[si]
                        .label
                        .clone()
                }),
            ScheduleKind::Shell => None,
        };
        s.target_label(label.as_deref())
    }

    /// Whether `session_id` is what this schedule acts on.
    fn schedule_hits_session(&self, s: &Schedule, session_id: &str) -> bool {
        match s.kind {
            ScheduleKind::Prompt => self
                .schedule_target_session(s)
                .map(|(pi, fi, si)| {
                    self.store.projects[pi].features[fi].sessions[si]
                        .id
                        .as_str()
                        == session_id
                })
                .unwrap_or(false),
            ScheduleKind::Shell => self
                .session_indices_by_id(session_id)
                .map(|(pi, fi, si)| {
                    let feature = &self.store.projects[pi].features[fi];
                    feature.id == s.feature_id
                        && feature.sessions[si].label
                            == s.target.as_deref().unwrap_or(DEFAULT_TERMINAL)
                })
                .unwrap_or(false),
        }
    }

    /// Whether the session has an active schedule (the row's ⏲).
    pub fn session_has_active_schedule(&self, session_id: &str) -> bool {
        let now = Utc::now();
        self.schedules
            .items
            .iter()
            .any(|s| !s.inactive(now) && self.schedule_hits_session(s, session_id))
    }

    /// Whether the feature has an active schedule (the row's ⏲).
    pub fn feature_has_active_schedule(&self, feature_id: &str) -> bool {
        let now = Utc::now();
        self.schedules
            .items
            .iter()
            .any(|s| s.feature_id == feature_id && !s.inactive(now))
    }

    /// The info block's `⏲ next 14:30 · every 30 min` for a session.
    pub fn schedule_info_line(&self, session_id: &str) -> Option<String> {
        let now = Utc::now();
        let mut active: Vec<(DateTime<Utc>, &Schedule)> = self
            .schedules
            .items
            .iter()
            .filter(|s| self.schedule_hits_session(s, session_id))
            .filter_map(|s| s.next_fire(now, &Local).map(|next| (next, s)))
            .collect();
        if active.is_empty() {
            return None;
        }
        active.sort_by_key(|(next, _)| *next);
        let (next, first) = active[0];
        let when = schedules::short_when(next, now, &Local);
        Some(if active.len() == 1 {
            let spec = first
                .parsed()
                .map(|s| s.describe())
                .unwrap_or_else(|_| first.spec.clone());
            format!("⏲ next {when} · {spec}")
        } else {
            format!("⏲ {} schedules · next {when}", active.len())
        })
    }

    /// Schedules in the list's scope, oldest first.
    pub fn schedules_in_scope(&self, scope: &ScheduleScope) -> Vec<&Schedule> {
        self.schedules
            .items
            .iter()
            .filter(|s| match scope {
                ScheduleScope::Session {
                    feature_id,
                    session_id,
                } => {
                    s.feature_id == *feature_id
                        && (s.session_id.is_none() || s.session_id.as_deref() == Some(session_id))
                }
                ScheduleScope::Feature { feature_id } => s.feature_id == *feature_id,
                ScheduleScope::Project { project_id } => self
                    .feature_indices_by_id(&s.feature_id)
                    .is_some_and(|(pi, _)| self.store.projects[pi].id == *project_id),
            })
            .collect()
    }

    /// The list's rows.
    pub fn schedule_rows(&self, state: &ScheduleListState) -> Vec<ScheduleRow> {
        let now = Utc::now();
        self.schedules_in_scope(&state.scope)
            .into_iter()
            .map(|s| {
                let feature = self
                    .feature_indices_by_id(&s.feature_id)
                    .map(|(pi, fi)| self.store.projects[pi].features[fi].name.clone())
                    .unwrap_or_default();
                ScheduleRow {
                    id: s.id.clone(),
                    spec: s
                        .parsed()
                        .map(|p| p.describe())
                        .unwrap_or_else(|_| s.spec.clone()),
                    target: self.schedule_target_text(s),
                    feature,
                    text: s.text.lines().next().unwrap_or_default().to_string(),
                    next: s
                        .next_fire(now, &Local)
                        .map(|t| schedules::short_when(t, now, &Local)),
                    last: self.last_run(&s.id).map(|r| {
                        format!("{} {}", schedules::short_when(r.at, now, &Local), r.outcome)
                    }),
                    paused: s.paused,
                    finished: s.finished(now),
                    from_config: s.from_config(),
                    deferred: self.schedules.deferred.contains_key(&s.id),
                }
            })
            .collect()
    }

    // ── the timer ────────────────────────────────────────────────

    /// Fire what is due (rate-limited). Called every loop turn; returns
    /// whether anything visible changed.
    pub fn schedules_tick(&mut self) -> bool {
        if !self.config.aiw.schedules {
            return false;
        }
        if self
            .schedules
            .last_tick
            .is_some_and(|at| at.elapsed() < TICK_INTERVAL)
        {
            return false;
        }
        self.schedules.last_tick = Some(Instant::now());
        if self
            .schedules
            .last_config_sync
            .is_none_or(|at| at.elapsed() >= CONFIG_SYNC_INTERVAL)
        {
            self.schedules.last_config_sync = Some(Instant::now());
            self.sync_config_schedules();
        }
        self.schedules_tick_at(Utc::now(), &Local)
    }

    /// One timer pass at `now`, reading wall-clock rules in `tz`.
    pub(crate) fn schedules_tick_at<Tz: TimeZone>(&mut self, now: DateTime<Utc>, tz: &Tz) -> bool {
        // Schedules of features that are gone go too.
        let orphans: Vec<String> = self
            .schedules
            .items
            .iter()
            .filter(|s| {
                self.feature_indices_by_id(&s.feature_id).is_none()
                    || s.session_id
                        .as_ref()
                        .is_some_and(|id| self.session_indices_by_id(id).is_none())
            })
            .map(|s| s.id.clone())
            .collect();
        let mut changed = !orphans.is_empty();
        for id in orphans {
            self.drop_schedule(&id);
        }
        let ids: Vec<String> = self.schedules.items.iter().map(|s| s.id.clone()).collect();
        for id in ids {
            changed |= self.schedule_step(&id, now, tz);
        }
        changed
    }

    fn schedule_step<Tz: TimeZone>(&mut self, id: &str, now: DateTime<Utc>, tz: &Tz) -> bool {
        let Some(s) = self.schedule(id).cloned() else {
            return false;
        };
        if let Some(deferred) = self.schedules.deferred.get(id).copied() {
            if s.paused {
                self.schedules.deferred.remove(id);
                return true;
            }
            if (now - deferred.last_try).num_seconds() < DEFER_RETRY_SECS {
                return false;
            }
            let outcome = self.fire_schedule_now(id, Trigger::Retry);
            if outcome == Outcome::SkippedWaiting {
                let next_slot_passed = s
                    .parsed()
                    .ok()
                    .and_then(|spec| spec.next_after(deferred.slot, tz))
                    .is_some_and(|next| next <= now);
                let one_shot_expired = s.parsed().is_ok_and(|spec| spec.is_one_shot())
                    && (now - deferred.since).num_seconds() >= ONE_SHOT_DEFER_LIMIT_SECS;
                if next_slot_passed || one_shot_expired {
                    self.schedules.deferred.remove(id);
                    self.record_schedule_run(id, &outcome, "gave up".into(), now);
                    self.finish_fire(id, &outcome, now);
                } else if let Some(d) = self.schedules.deferred.get_mut(id) {
                    d.last_try = now;
                }
                return false;
            }
            self.schedules.deferred.remove(id);
            self.record_schedule_run(id, &outcome, Trigger::Retry.label().into(), now);
            self.finish_fire(id, &outcome, now);
            return true;
        }
        let Some(due) = schedules::due(&s, now, self.schedules.started, tz) else {
            return false;
        };
        let (slot, trigger) = match due {
            Due::Missed(_) if s.catch_up == CatchUp::None => {
                self.record_schedule_run(id, &Outcome::Missed, String::new(), now);
                self.finish_fire(id, &Outcome::Missed, now);
                return true;
            }
            Due::Missed(slot) => (slot, Trigger::CatchUp),
            Due::Now(slot) => (slot, Trigger::Timer),
        };
        let outcome = self.fire_schedule_now(id, trigger);
        self.record_schedule_run(id, &outcome, trigger.label().into(), now);
        if outcome == Outcome::SkippedWaiting {
            self.schedules.deferred.insert(
                id.to_string(),
                Deferred {
                    since: now,
                    last_try: now,
                    slot,
                },
            );
        } else {
            self.finish_fire(id, &outcome, now);
        }
        true
    }

    /// After a fire: advance, count, save.
    fn finish_fire(&mut self, id: &str, outcome: &Outcome, now: DateTime<Utc>) {
        if let Some(s) = self.schedule_mut(id) {
            s.last_fire = Some(now);
            if outcome.counts_as_run() {
                s.runs = s.runs.saturating_add(1);
            }
        }
        self.persist_schedule(id);
    }

    /// Fire `id` now and return what happened (nothing is recorded).
    fn fire_schedule_now(&mut self, id: &str, _trigger: Trigger) -> Outcome {
        let Some(s) = self.schedule(id).cloned() else {
            return Outcome::Failed("the schedule is gone".into());
        };
        let Some((pi, fi)) = self.feature_indices_by_id(&s.feature_id) else {
            return Outcome::Failed("its feature is gone".into());
        };
        match s.kind {
            ScheduleKind::Prompt => self.fire_prompt_schedule(&s),
            ScheduleKind::Shell => self.fire_shell_schedule(&s, pi, fi),
        }
    }

    fn resource_gate_open(&self) -> bool {
        matches!(
            self.check_start_preconditions(),
            crate::app::resource_gate::StartPreconditions::Ok
        )
    }

    fn fire_prompt_schedule(&mut self, s: &Schedule) -> Outcome {
        let Some((pi, fi, si)) = self.schedule_target_session(s) else {
            return Outcome::Failed(match &s.target {
                Some(label) => format!("no agent session `{label}`"),
                None => "the feature has no agent session".into(),
            });
        };
        let (session_id, tmux_session, window, background, feature_stopped) = {
            let feature = &self.store.projects[pi].features[fi];
            let session = &feature.sessions[si];
            (
                session.id.clone(),
                feature.tmux_session.clone(),
                session.tmux_window.clone(),
                session.is_background(),
                feature.status == ProjectStatus::Stopped,
            )
        };
        // Never stack on a fire still waiting in the queue.
        if let Some(followup) = s.queued_followup
            && self
                .followups_for(&session_id)
                .iter()
                .any(|f| f.id == followup)
        {
            return Outcome::SkippedStillQueued;
        }
        if background {
            // No pane: Claude's inbox, else the follow-up queue (§19.5).
            let before = self.followups_for(&session_id).last().map(|f| f.id);
            return match self.send_session_message(&session_id, &s.text, "aiw schedule") {
                Delivery::Inbox => Outcome::DeliveredInbox,
                Delivery::Queued { .. } => {
                    self.remember_queued_followup(&s.id, &session_id, before);
                    Outcome::Queued
                }
                Delivery::Failed(why) => Outcome::Failed(why),
            };
        }
        let Some(obs) = self.watchdog_observe(pi, fi, si) else {
            return Outcome::Failed("not an agent session".into());
        };
        if obs.exited || !obs.running {
            if !s.start_if_stopped {
                return Outcome::SkippedNotRunning;
            }
            if !self.resource_gate_open() {
                return Outcome::SkippedLimits;
            }
            let started = if feature_stopped {
                self.ensure_feature_running_with_target(
                    pi,
                    fi,
                    Some(&session_id),
                    crate::app::resource_gate::StartIntent::Approved,
                )
                .map(|_| ())
            } else {
                let resume = super::session_ops::persisted_resume_id(
                    &self.store.projects[pi].features[fi].sessions[si],
                );
                self.restart_stopped_session_window_resuming(pi, fi, si, Some(resume))
                    .map(|_| ())
            };
            return match started {
                Ok(()) => {
                    self.queue_pending_prompt(&session_id, &s.text, true, false);
                    Outcome::Started
                }
                Err(e) => Outcome::Failed(format!("could not start it: {e}")),
            };
        }
        if self.schedule_blocked_on_user(pi, fi, si) {
            return Outcome::SkippedWaiting;
        }
        let idle = !obs.working
            && self.queued_followup_count(&session_id) == 0
            && self.pending_prompt_for(&session_id).is_none();
        if idle {
            if let Err(e) = self.tmux.paste_text(&tmux_session, &window, &s.text) {
                return Outcome::Failed(format!("could not paste: {e}"));
            }
            self.schedule_prompt_enter(&session_id);
            return Outcome::Delivered;
        }
        let before = self.followups_for(&session_id).last().map(|f| f.id);
        match self.queue_followup(&session_id, &s.text) {
            Ok(_) => {
                self.remember_queued_followup(&s.id, &session_id, before);
                Outcome::Queued
            }
            Err(e) => Outcome::Failed(format!("could not queue: {e}")),
        }
    }

    /// Whether the agent is blocked on you: a permission request, a question
    /// its hooks raised, a wait Claude's roster saw, or a held edit (a
    /// Vibeless diff review). Narrower than the watchdog's check on purpose:
    /// the generic "finished, waiting for input" marker a turn end leaves, and
    /// a harness's undifferentiated wait, mean the agent is at its prompt —
    /// exactly when a scheduled prompt should go in.
    fn schedule_blocked_on_user(&self, pi: usize, fi: usize, si: usize) -> bool {
        use crate::app::attention::AttentionState;
        let feature = &self.store.projects[pi].features[fi];
        let session = &feature.sessions[si];
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
                    record.state == AttentionState::Question
                        && crate::fork::badges::signal_applies(
                            feature,
                            Some(&record.agent),
                            session,
                        )
                })
            || self.pending_inputs.iter().any(|input| {
                input.feature_name.as_deref() == Some(feature.name.as_str())
                    && !input.is_session_wait()
            })
    }

    fn remember_queued_followup(
        &mut self,
        schedule_id: &str,
        session_id: &str,
        before: Option<i64>,
    ) {
        let queued = self
            .followups_for(session_id)
            .last()
            .map(|f| f.id)
            .filter(|id| Some(*id) != before);
        if let Some(s) = self.schedule_mut(schedule_id) {
            s.queued_followup = queued;
        }
    }

    fn fire_shell_schedule(&mut self, s: &Schedule, pi: usize, fi: usize) -> Outcome {
        let terminal = s
            .target
            .clone()
            .unwrap_or_else(|| DEFAULT_TERMINAL.to_string());
        let (tmux_session, running) = {
            let feature = &self.store.projects[pi].features[fi];
            (
                feature.tmux_session.clone(),
                feature.status != ProjectStatus::Stopped
                    && self.tmux.session_exists(&feature.tmux_session),
            )
        };
        let mut pi = pi;
        let mut fi = fi;
        if !running {
            if !s.start_if_stopped {
                return Outcome::SkippedNotRunning;
            }
            if !self.resource_gate_open() {
                return Outcome::SkippedLimits;
            }
            if let Err(e) = self.ensure_feature_running(
                pi,
                fi,
                crate::app::resource_gate::StartIntent::Approved,
            ) {
                return Outcome::Failed(format!("could not start the feature: {e}"));
            }
            match self.feature_indices_by_id(&s.feature_id) {
                Some((p, f)) => (pi, fi) = (p, f),
                None => return Outcome::Failed("its feature is gone".into()),
            }
        }
        let existing = self.store.projects[pi].features[fi]
            .sessions
            .iter()
            .find(|x| {
                x.label == terminal && matches!(x.kind, SessionKind::Terminal | SessionKind::Custom)
            })
            .map(|x| (x.id.clone(), x.tmux_window.clone()));
        let mut just_opened = false;
        let window = match existing {
            Some((_, window)) => {
                if !self.tmux.window_exists(&tmux_session, &window) {
                    let workdir = self.store.projects[pi].features[fi].workdir.clone();
                    if let Err(e) = self.tmux.create_window(&tmux_session, &window, &workdir) {
                        return Outcome::Failed(format!("could not open `{terminal}`: {e}"));
                    }
                    just_opened = true;
                }
                window
            }
            None => {
                // Opening it must not move the user's selection.
                let selection = self.selection.clone();
                let message = self.message.clone();
                let created = self.add_terminal_session_for_picker(pi, fi, Some(terminal.clone()));
                self.selection = selection;
                self.message = message;
                just_opened = true;
                match created {
                    Ok(session_id) => match self.session_indices_by_id(&session_id) {
                        Some((p, f, si)) => self.store.projects[p].features[f].sessions[si]
                            .tmux_window
                            .clone(),
                        None => return Outcome::Failed("the new terminal is gone".into()),
                    },
                    Err(e) => return Outcome::Failed(format!("could not open `{terminal}`: {e}")),
                }
            }
        };
        // Only into a terminal at its prompt. One opened just now is (its
        // shell may still be starting: tmux reports itself until it execs).
        let panes = if just_opened {
            Default::default()
        } else {
            crate::fork::session_reconcile::agent_panes(self.tmux.pane_commands())
        };
        if let Some(pane) = panes.get(&(tmux_session.clone(), window.clone())) {
            let at_prompt = crate::fork::session_reconcile::is_plain_shell(&pane.pane.command)
                && pane.pane.shell_has_children != Some(true);
            if !at_prompt {
                let what = if pane.pane.command.is_empty() {
                    "something".to_string()
                } else {
                    pane.pane.command.clone()
                };
                return Outcome::SkippedBusy(what);
            }
        }
        match self.tmux.run_shell_command(&tmux_session, &window, &s.text) {
            Ok(()) => Outcome::Ran,
            Err(e) => Outcome::Failed(format!("could not run it: {e}")),
        }
    }

    // ── actions ──────────────────────────────────────────────────

    /// Run `id` now, whatever its timer says. Returns what happened.
    pub fn run_schedule_now(&mut self, id: &str, trigger: Trigger) -> Result<Outcome> {
        if self.schedule(id).is_none() {
            return Err(anyhow!("no schedule {id}"));
        }
        let now = Utc::now();
        self.schedules.deferred.remove(id);
        let outcome = self.fire_schedule_now(id, trigger);
        self.record_schedule_run(id, &outcome, trigger.label().into(), now);
        self.finish_fire(id, &outcome, now);
        Ok(outcome)
    }

    /// Pause or resume. Returns the new paused state.
    pub fn set_schedule_paused(&mut self, id: &str, paused: bool) -> Result<bool> {
        let s = self
            .schedule_mut(id)
            .ok_or_else(|| anyhow!("no schedule {id}"))?;
        s.paused = paused;
        if !paused {
            // A resumed schedule counts from now, not from its last fire
            // before the pause (no burst of catch-up).
            let now = Utc::now();
            if s.last_fire.is_none_or(|t| t < now) && !s.parsed().is_ok_and(|p| p.is_one_shot()) {
                s.last_fire = Some(now);
            }
        }
        self.schedules.deferred.remove(id);
        self.persist_schedule(id);
        Ok(paused)
    }

    /// Delete a dashboard-made schedule (`aiw.json` ones are refused).
    pub fn delete_schedule(&mut self, id: &str) -> Result<()> {
        let s = self
            .schedule(id)
            .ok_or_else(|| anyhow!("no schedule {id}"))?;
        if let Some(name) = &s.config_name {
            return Err(anyhow!(
                "`{name}` is defined in aiw.json: remove it there, or pause it"
            ));
        }
        self.drop_schedule(id);
        Ok(())
    }

    /// Add or replace a schedule after checking it. Returns its id.
    pub fn save_schedule(&mut self, schedule: Schedule) -> Result<String> {
        let mut schedule = schedule;
        let now = Utc::now();
        schedule.spec =
            schedules::normalize_spec(&schedule.spec, self.schedule_min_interval(), now, &Local)
                .map_err(|e| anyhow!(e))?;
        if schedule.text.trim().is_empty() {
            return Err(anyhow!(match schedule.kind {
                ScheduleKind::Prompt => "the prompt is empty",
                ScheduleKind::Shell => "the command is empty",
            }));
        }
        if self.feature_indices_by_id(&schedule.feature_id).is_none() {
            return Err(anyhow!("the feature is gone"));
        }
        if schedule.kind == ScheduleKind::Prompt {
            if let Some(id) = &schedule.session_id {
                let (pi, fi, si) = self
                    .session_indices_by_id(id)
                    .ok_or_else(|| anyhow!("the session is gone"))?;
                if !self.store.projects[pi].features[fi].sessions[si]
                    .kind
                    .is_agent_harness()
                {
                    return Err(anyhow!("a prompt goes to an agent session"));
                }
            }
        } else {
            schedule.session_id = None;
            if schedule
                .target
                .as_deref()
                .is_some_and(|t| t.trim().is_empty())
            {
                schedule.target = None;
            }
        }
        let id = schedule.id.clone();
        match self.schedules.items.iter_mut().find(|s| s.id == id) {
            Some(existing) => {
                if existing.from_config() {
                    return Err(anyhow!(
                        "this schedule is defined in aiw.json; edit it there"
                    ));
                }
                // An edit keeps its history and count; a changed rule counts
                // from now.
                if existing.spec != schedule.spec {
                    schedule.last_fire = Some(now);
                } else {
                    schedule.last_fire = existing.last_fire;
                }
                schedule.runs = existing.runs;
                schedule.created_at = existing.created_at;
                schedule.queued_followup = existing.queued_followup;
                *existing = schedule;
            }
            None => self.schedules.items.push(schedule),
        }
        self.schedules.deferred.remove(&id);
        self.persist_schedule(&id);
        Ok(id)
    }

    /// A new schedule with defaults, for `feature_id`.
    pub fn new_schedule(feature_id: &str) -> Schedule {
        Schedule {
            id: uuid::Uuid::new_v4().to_string(),
            feature_id: feature_id.to_string(),
            session_id: None,
            target: None,
            kind: ScheduleKind::Prompt,
            spec: String::new(),
            text: String::new(),
            paused: false,
            start_if_stopped: false,
            catch_up: CatchUp::Once,
            end_at: None,
            max_runs: None,
            runs: 0,
            last_fire: None,
            created_at: Utc::now(),
            config_name: None,
            queued_followup: None,
        }
    }

    // ── aiw.json ─────────────────────────────────────────────────

    /// Bring `aiw.json` schedules in line with the config: one per matching
    /// feature, keeping run state; config entries that went away go too.
    pub(crate) fn sync_config_schedules(&mut self) {
        let min = self.schedule_min_interval();
        let now = Utc::now();
        let mut wanted: HashSet<String> = HashSet::new();
        let mut desired: Vec<Schedule> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();
        for project in &self.store.projects {
            let extension = self.extension_for_repo(&project.repo);
            for entry in &extension.schedules {
                let (kind, text) = match entry.validate(min) {
                    Ok(ok) => ok,
                    Err(e) => {
                        warnings.push(format!("{}: {e}", project.name));
                        continue;
                    }
                };
                let spec = match schedules::parse_spec(&entry.spec, min) {
                    Ok(spec) => spec.to_string(),
                    Err(_) => continue,
                };
                let end_at = entry
                    .end
                    .as_deref()
                    .and_then(|e| schedules::parse_end(e).ok())
                    .map(|e| e.in_zone(&Local));
                for feature in &project.features {
                    if !entry.applies_to(&feature.name, &feature.branch) {
                        continue;
                    }
                    let id = schedules::config_schedule_id(&feature.id, &entry.name);
                    wanted.insert(id.clone());
                    desired.push(Schedule {
                        id,
                        feature_id: feature.id.clone(),
                        session_id: None,
                        target: match kind {
                            ScheduleKind::Prompt => entry.session.clone(),
                            ScheduleKind::Shell => entry.terminal.clone(),
                        },
                        kind,
                        spec: spec.clone(),
                        text: text.clone(),
                        paused: false,
                        start_if_stopped: entry.start_if_stopped,
                        catch_up: entry.catch_up.unwrap_or_default(),
                        end_at,
                        max_runs: entry.max_runs,
                        runs: 0,
                        last_fire: None,
                        created_at: now,
                        config_name: Some(entry.name.clone()),
                        queued_followup: None,
                    });
                }
            }
        }
        for warning in warnings {
            if self.schedules.config_warnings.insert(warning.clone()) {
                self.log_warn("schedule", format!("aiw.json: {warning}"));
            }
        }
        let stale: Vec<String> = self
            .schedules
            .items
            .iter()
            .filter(|s| s.from_config() && !wanted.contains(&s.id))
            .map(|s| s.id.clone())
            .collect();
        for id in stale {
            self.drop_schedule(&id);
        }
        for want in desired {
            let id = want.id.clone();
            match self.schedules.items.iter_mut().find(|s| s.id == id) {
                Some(have) => {
                    let same = have.kind == want.kind
                        && have.target == want.target
                        && have.spec == want.spec
                        && have.text == want.text
                        && have.start_if_stopped == want.start_if_stopped
                        && have.catch_up == want.catch_up
                        && have.end_at == want.end_at
                        && have.max_runs == want.max_runs;
                    if same {
                        continue;
                    }
                    if have.spec != want.spec {
                        have.last_fire = Some(now);
                    }
                    have.kind = want.kind;
                    have.target = want.target;
                    have.spec = want.spec;
                    have.text = want.text;
                    have.start_if_stopped = want.start_if_stopped;
                    have.catch_up = want.catch_up;
                    have.end_at = want.end_at;
                    have.max_runs = want.max_runs;
                }
                None => self.schedules.items.push(want),
            }
            self.persist_schedule(&id);
        }
    }

    // ── the dialogs ──────────────────────────────────────────────

    /// `t` on the dashboard: the selected session's, feature's or project's
    /// schedules.
    pub fn open_schedules_for_selection(&mut self) {
        let (scope, title) = match self.selection.clone() {
            Selection::Session(pi, fi, si) => {
                let Some(feature) = self.store.projects.get(pi).and_then(|p| p.features.get(fi))
                else {
                    return;
                };
                let Some(session) = feature.sessions.get(si) else {
                    return;
                };
                (
                    ScheduleScope::Session {
                        feature_id: feature.id.clone(),
                        session_id: session.id.clone(),
                    },
                    format!("{} · {}", session.label, feature.name),
                )
            }
            Selection::Feature(pi, fi) => {
                let Some(feature) = self.store.projects.get(pi).and_then(|p| p.features.get(fi))
                else {
                    return;
                };
                (
                    ScheduleScope::Feature {
                        feature_id: feature.id.clone(),
                    },
                    feature.name.clone(),
                )
            }
            Selection::Project(pi) => {
                let Some(project) = self.store.projects.get(pi) else {
                    return;
                };
                (
                    ScheduleScope::Project {
                        project_id: project.id.clone(),
                    },
                    format!("{} (all features)", project.name),
                )
            }
        };
        self.mode = AppMode::Schedules(ScheduleListState {
            scope,
            title,
            selected: 0,
            show_history: false,
            confirm_delete: false,
        });
    }

    /// The selected row's schedule id in the open list.
    pub fn schedule_list_selected_id(&self) -> Option<String> {
        let AppMode::Schedules(state) = &self.mode else {
            return None;
        };
        self.schedule_rows(state)
            .get(state.selected)
            .map(|row| row.id.clone())
    }

    /// `n` / `e` in the list: the add / edit dialog.
    pub fn open_schedule_editor(&mut self, editing: Option<String>) {
        let AppMode::Schedules(list) = &self.mode else {
            return;
        };
        let list = list.clone();
        let existing = editing.as_ref().and_then(|id| self.schedule(id).cloned());
        if let Some(s) = &existing
            && let Some(name) = &s.config_name
        {
            self.push_toast_warning(format!(
                "`{name}` is defined in aiw.json: edit it there (p pauses it here)"
            ));
            return;
        }
        let (feature_id, pinned) = match (&existing, &list.scope) {
            (Some(s), _) => (s.feature_id.clone(), s.session_id.clone()),
            (
                None,
                ScheduleScope::Session {
                    feature_id,
                    session_id,
                },
            ) => (feature_id.clone(), Some(session_id.clone())),
            (None, ScheduleScope::Feature { feature_id }) => (feature_id.clone(), None),
            (None, ScheduleScope::Project { .. }) => {
                self.push_toast_warning("Select a feature or session row to add a schedule there");
                return;
            }
        };
        let Some((pi, fi)) = self.feature_indices_by_id(&feature_id) else {
            return;
        };
        let feature = &self.store.projects[pi].features[fi];
        let mut targets = vec![(None, "the feature's first agent".to_string())];
        for session in feature
            .sessions
            .iter()
            .filter(|s| s.kind.is_agent_harness())
        {
            targets.push((Some(session.id.clone()), session.label.clone()));
        }
        let pinned_is_agent = pinned
            .as_ref()
            .is_some_and(|id| targets.iter().any(|(t, _)| t.as_ref() == Some(id)));
        let target_index = pinned
            .as_ref()
            .and_then(|id| targets.iter().position(|(t, _)| t.as_ref() == Some(id)))
            .unwrap_or(0);
        let mut state = ScheduleEditorState {
            editing,
            feature_id: feature_id.clone(),
            feature: feature.name.clone(),
            kind: if pinned.is_some() && !pinned_is_agent {
                ScheduleKind::Shell
            } else {
                ScheduleKind::Prompt
            },
            targets,
            target_index,
            terminal: DEFAULT_TERMINAL.to_string(),
            spec: String::new(),
            text: String::new(),
            start_if_stopped: false,
            catch_up: CatchUp::Once,
            max_runs: String::new(),
            end: String::new(),
            focus: EditorField::When,
            error: None,
            list,
        };
        // From a terminal row: a shell schedule in that terminal.
        if let (None, Some(id)) = (&existing, &pinned)
            && !pinned_is_agent
            && let Some((p, f, si)) = self.session_indices_by_id(id)
        {
            state.terminal = self.store.projects[p].features[f].sessions[si]
                .label
                .clone();
        }
        if let Some(s) = existing {
            state.kind = s.kind;
            state.spec = s.spec.clone();
            state.text = s.text.clone();
            state.start_if_stopped = s.start_if_stopped;
            state.catch_up = s.catch_up;
            state.max_runs = s.max_runs.map(|n| n.to_string()).unwrap_or_default();
            state.end = s
                .end_at
                .map(|t| t.with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_default();
            match s.kind {
                ScheduleKind::Prompt => {
                    if let Some(label) = &s.target
                        && let Some(i) = state.targets.iter().position(|(_, l)| l == label)
                    {
                        state.target_index = i;
                    }
                }
                ScheduleKind::Shell => {
                    state.terminal = s
                        .target
                        .clone()
                        .unwrap_or_else(|| DEFAULT_TERMINAL.to_string());
                }
            }
        }
        self.mode = AppMode::ScheduleEditor(Box::new(state));
    }

    /// The editor's preview: `next: Tue 14:30` or the problem.
    pub fn schedule_editor_preview(&self, state: &ScheduleEditorState) -> Result<String, String> {
        let now = Utc::now();
        let spec =
            schedules::normalize_spec(&state.spec, self.schedule_min_interval(), now, &Local)?;
        let parsed = schedules::parse_spec(&spec, 1)?;
        let anchor = now;
        let next = parsed
            .next_after(anchor, &Local)
            .ok_or_else(|| "it never fires".to_string())?;
        Ok(format!(
            "{} · first {}",
            parsed.describe(),
            schedules::short_when(next, now, &Local)
        ))
    }

    /// Enter in the editor: save and go back to the list.
    pub fn submit_schedule_editor(&mut self) {
        let AppMode::ScheduleEditor(state) = &self.mode else {
            return;
        };
        let state = (**state).clone();
        let result = (|| -> Result<String> {
            let mut schedule = match &state.editing {
                Some(id) => self
                    .schedule(id)
                    .cloned()
                    .ok_or_else(|| anyhow!("the schedule is gone"))?,
                None => Self::new_schedule(&state.feature_id),
            };
            schedule.kind = state.kind;
            schedule.spec = state.spec.clone();
            schedule.text = state.text.trim().to_string();
            schedule.start_if_stopped = state.start_if_stopped;
            schedule.catch_up = state.catch_up;
            schedule.max_runs = match state.max_runs.trim() {
                "" => None,
                n => Some(
                    n.parse::<u32>()
                        .ok()
                        .filter(|n| *n > 0)
                        .ok_or_else(|| anyhow!("max runs is a positive number or empty"))?,
                ),
            };
            schedule.end_at = match state.end.trim() {
                "" => None,
                end => Some(
                    schedules::parse_end(end)
                        .map_err(|e| anyhow!(e))?
                        .in_zone(&Local),
                ),
            };
            if schedule.end_at.is_some_and(|end| end <= Utc::now()) {
                return Err(anyhow!("the end time has passed"));
            }
            match state.kind {
                ScheduleKind::Prompt => {
                    let (pinned, _) = state
                        .targets
                        .get(state.target_index)
                        .cloned()
                        .unwrap_or((None, String::new()));
                    schedule.session_id = pinned;
                    schedule.target = None;
                }
                ScheduleKind::Shell => {
                    let name = state.terminal.trim();
                    schedule.session_id = None;
                    schedule.target =
                        (!name.is_empty() && name != DEFAULT_TERMINAL).then(|| name.to_string());
                }
            }
            self.save_schedule(schedule)
        })();
        match result {
            Ok(id) => {
                let list = state.list.clone();
                self.mode = AppMode::Schedules(list);
                // Select the saved row.
                if let AppMode::Schedules(list) = &self.mode {
                    let index = self
                        .schedule_rows(list)
                        .iter()
                        .position(|r| r.id == id)
                        .unwrap_or(0);
                    if let AppMode::Schedules(list) = &mut self.mode {
                        list.selected = index;
                    }
                }
                self.push_toast_success("Schedule saved");
            }
            Err(e) => {
                if let AppMode::ScheduleEditor(state) = &mut self.mode {
                    state.error = Some(e.to_string());
                }
            }
        }
    }

    /// Esc in the editor: back to the list.
    pub fn cancel_schedule_editor(&mut self) {
        if let AppMode::ScheduleEditor(state) = &self.mode {
            self.mode = AppMode::Schedules(state.list.clone());
        }
    }

    // ── `aiw automation schedule …` ─────────────────────────────

    fn schedule_json(&self, s: &Schedule) -> Value {
        let now = Utc::now();
        let (project, feature) = self
            .feature_indices_by_id(&s.feature_id)
            .map(|(pi, fi)| {
                (
                    self.store.projects[pi].name.clone(),
                    self.store.projects[pi].features[fi].name.clone(),
                )
            })
            .unwrap_or_default();
        let last = self.last_run(&s.id).map(|r| {
            serde_json::json!({"at": r.at.to_rfc3339(), "outcome": r.outcome, "detail": r.detail})
        });
        serde_json::json!({
            "id": s.id,
            "project": project,
            "feature": feature,
            "session_id": s.session_id,
            "target": self.schedule_target_text(s),
            "kind": s.kind.as_str(),
            "spec": s.spec,
            "describe": s.parsed().map(|p| p.describe()).unwrap_or_default(),
            "text": s.text,
            "paused": s.paused,
            "finished": s.finished(now),
            "next_fire": s.next_fire(now, &Local).map(|t| t.to_rfc3339()),
            "runs": s.runs,
            "max_runs": s.max_runs,
            "end_at": s.end_at.map(|t| t.to_rfc3339()),
            "catch_up": s.catch_up.as_str(),
            "start_if_stopped": s.start_if_stopped,
            "source": if s.from_config() { "aiw.json" } else { "user" },
            "config_name": s.config_name,
            "last_run": last,
        })
    }

    /// A feature by id, name, or `project/feature`.
    fn resolve_schedule_feature(
        &self,
        reference: &str,
        project: Option<&str>,
    ) -> Result<String, String> {
        let reference = reference.trim();
        let (project, name) = match reference.split_once('/') {
            Some((p, f)) if project.is_none() => (Some(p), f),
            _ => (project, reference),
        };
        let mut hits = Vec::new();
        for p in &self.store.projects {
            if project.is_some_and(|want| p.name != want) {
                continue;
            }
            for f in &p.features {
                if f.id == name || f.name == name || f.branch == name {
                    hits.push(f.id.clone());
                }
            }
        }
        match hits.len() {
            1 => Ok(hits.remove(0)),
            0 => Err(format!("no feature `{reference}`")),
            n => Err(format!(
                "`{reference}` names {n} features; add --project or use project/feature"
            )),
        }
    }

    pub(crate) fn automation_schedule(&mut self, raw: &Value) -> Value {
        let op = raw
            .get("op")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let result = self.automation_schedule_inner(&op, raw);
        let mut out = match result {
            Ok(value) => value,
            Err(e) => serde_json::json!({"ok": false, "error": e}),
        };
        out["type"] = Value::String(crate::automation::AUTOMATION_RESULT_TYPE.into());
        out["action"] = Value::String(SCHEDULE_ACTION.into());
        out["op"] = Value::String(op);
        if out.get("ok").is_none() {
            out["ok"] = Value::Bool(true);
        }
        out
    }

    fn automation_schedule_inner(&mut self, op: &str, raw: &Value) -> Result<Value, String> {
        let text = |key: &str| raw.get(key).and_then(Value::as_str).map(str::to_string);
        let id = || -> Result<String, String> {
            let id = text("id").ok_or("--id is required")?;
            // A unique prefix is enough.
            let hits: Vec<String> = self
                .schedules
                .items
                .iter()
                .filter(|s| s.id == id || s.id.starts_with(&id))
                .map(|s| s.id.clone())
                .collect();
            match hits.len() {
                1 => Ok(hits[0].clone()),
                0 => Err(format!("no schedule `{id}`")),
                _ => Err(format!(
                    "`{id}` matches several schedules; give more of the id"
                )),
            }
        };
        match op {
            "add" => {
                let (feature_id, session_id) = match (text("session"), text("feature")) {
                    (Some(session), _) => {
                        let session_id = self.resolve_message_target(&session)?;
                        let (pi, fi, _) = self
                            .session_indices_by_id(&session_id)
                            .ok_or("the session is gone")?;
                        (
                            self.store.projects[pi].features[fi].id.clone(),
                            Some(session_id),
                        )
                    }
                    (None, Some(feature)) => (
                        self.resolve_schedule_feature(&feature, text("project").as_deref())?,
                        None,
                    ),
                    (None, None) => return Err("give --session or --feature".into()),
                };
                let mut schedule = Self::new_schedule(&feature_id);
                schedule.spec = text("spec").ok_or("--when is required")?;
                match (text("prompt"), text("command")) {
                    (Some(p), None) => {
                        schedule.kind = ScheduleKind::Prompt;
                        schedule.text = p;
                        schedule.session_id = session_id;
                        schedule.target = text("target");
                    }
                    (None, Some(c)) => {
                        schedule.kind = ScheduleKind::Shell;
                        schedule.text = c;
                        schedule.target = text("target").or_else(|| {
                            // --session on a terminal row: that terminal.
                            session_id
                                .as_ref()
                                .and_then(|id| self.session_indices_by_id(id))
                                .map(|(pi, fi, si)| {
                                    self.store.projects[pi].features[fi].sessions[si]
                                        .label
                                        .clone()
                                })
                        });
                    }
                    (Some(_), Some(_)) => return Err("give --prompt or --command, not both".into()),
                    (None, None) => return Err("give --prompt or --command".into()),
                }
                schedule.start_if_stopped = raw
                    .get("start_if_stopped")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                schedule.paused = raw.get("paused").and_then(Value::as_bool).unwrap_or(false);
                if let Some(c) = text("catch_up") {
                    schedule.catch_up = CatchUp::parse(&c).ok_or("--catch-up is once or none")?;
                }
                schedule.max_runs = raw
                    .get("max_runs")
                    .and_then(Value::as_u64)
                    .map(|n| n.min(u64::from(u32::MAX)) as u32);
                if let Some(end) = text("end") {
                    schedule.end_at = Some(schedules::parse_end(&end)?.in_zone(&Local));
                }
                let id = self.save_schedule(schedule).map_err(|e| e.to_string())?;
                let s = self.schedule(&id).cloned().ok_or("the schedule is gone")?;
                Ok(serde_json::json!({"schedule": self.schedule_json(&s)}))
            }
            "list" => {
                let scope = match (text("session"), text("feature"), text("project")) {
                    (Some(session), _, _) => {
                        let session_id = self.resolve_message_target(&session)?;
                        let (pi, fi, _) = self
                            .session_indices_by_id(&session_id)
                            .ok_or("the session is gone")?;
                        Some(ScheduleScope::Session {
                            feature_id: self.store.projects[pi].features[fi].id.clone(),
                            session_id,
                        })
                    }
                    (None, Some(feature), project) => Some(ScheduleScope::Feature {
                        feature_id: self.resolve_schedule_feature(&feature, project.as_deref())?,
                    }),
                    (None, None, Some(project)) => Some(ScheduleScope::Project {
                        project_id: self
                            .store
                            .projects
                            .iter()
                            .find(|p| p.name == project)
                            .map(|p| p.id.clone())
                            .ok_or(format!("no project `{project}`"))?,
                    }),
                    (None, None, None) => None,
                };
                let items: Vec<Value> = match &scope {
                    Some(scope) => self
                        .schedules_in_scope(scope)
                        .into_iter()
                        .map(|s| self.schedule_json(s))
                        .collect(),
                    None => self
                        .schedules
                        .items
                        .iter()
                        .map(|s| self.schedule_json(s))
                        .collect(),
                };
                Ok(serde_json::json!({"schedules": items}))
            }
            "pause" | "resume" => {
                let id = id()?;
                let paused = self
                    .set_schedule_paused(&id, op == "pause")
                    .map_err(|e| e.to_string())?;
                Ok(serde_json::json!({"id": id, "paused": paused}))
            }
            "delete" => {
                let id = id()?;
                self.delete_schedule(&id).map_err(|e| e.to_string())?;
                Ok(serde_json::json!({"id": id, "deleted": true}))
            }
            "run" => {
                let id = id()?;
                let outcome = self
                    .run_schedule_now(&id, Trigger::Automation)
                    .map_err(|e| e.to_string())?;
                Ok(serde_json::json!({
                    "id": id,
                    "outcome": outcome.label(),
                    "ok": !matches!(outcome, Outcome::Failed(_)),
                }))
            }
            "history" => {
                let id = id()?;
                let runs: Vec<Value> = self
                    .schedule_history(&id)
                    .into_iter()
                    .map(|r| serde_json::json!({"at": r.at.to_rfc3339(), "outcome": r.outcome, "detail": r.detail}))
                    .collect();
                Ok(serde_json::json!({"id": id, "runs": runs}))
            }
            other => Err(format!(
                "unknown schedule op `{other}` (add, list, pause, resume, delete, run, history)"
            )),
        }
    }
}

/// The deferral state of `id`, for tests.
#[cfg(test)]
impl App {
    pub(crate) fn schedule_is_deferred(&self, id: &str) -> bool {
        self.schedules.deferred.contains_key(id)
    }

    pub(crate) fn schedule_mut_for_test(&mut self, id: &str) -> &mut Schedule {
        self.schedule_mut(id).expect("schedule")
    }
}
