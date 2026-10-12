//! FORK: one prompt, several agents on `App` (DESIGN.md §17, §19.3). The
//! pure pieces live in `fork::fan_out`.
//!
//! - **Dialog launch.** [`App::create_feature_from_dialog`] runs the ordinary
//!   `create_feature` for the primary agent. With a shared layout the other
//!   agents get windows in the same feature once it is running; with one
//!   worktree each, the next sibling's dialog state is created when the
//!   previous feature's launch finishes ([`App::fan_out_feature_launched`]),
//!   so worktree hooks and their prompts work for every sibling exactly as
//!   they do for one feature.
//! - **Pending prompts.** Each agent's task prompt waits in
//!   `aiw_pending_prompts` until its pane is ready, then is pasted unsent
//!   (and, with "send when ready", followed by one Enter). It is removed
//!   before the paste, so it is delivered at most once, and it survives a
//!   restart of AIW until then.
//! - **Groups.** Worktree-each siblings share a group id
//!   (`aiw_fanout_members`): the sidebar draws a header above the first one
//!   with a rolled-up needs-you count, and `v` on any of them tiles the
//!   group.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::Result;

use super::{App, AppMode, CreateFeatureState, CreateFeatureStep, Selection, StartIntent};
use crate::automation::{CreateFeatureRequest, CreateFeatureResponse, FanOutFeature};
use crate::db::fan_out::FanOutMember;
use crate::fork::fan_out::{
    Due, FanOutLayout, PendingPrompt, fan_out_mode, normalize_prompt, shared_labels,
    sibling_branches,
};
use crate::project::{AgentKind, VibeMode, normalized_feature_name};

/// Once the fallback is due, the pane must have run the agent this long
/// before the paste (a launch script still `exec`ing reads as a shell).
const AGENT_SETTLE: Duration = Duration::from_secs(3);

/// How often the fallback asks tmux what the panes run.
const PANE_CHECK_INTERVAL: Duration = Duration::from_secs(1);

/// A feature's place in a fan-out group.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupMember {
    pub group_id: String,
    pub base_branch: String,
    pub agent: AgentKind,
    pub position: i64,
}

/// A worktree-each sibling still to be created.
#[derive(Debug, Clone, PartialEq)]
pub struct SiblingSpec {
    pub branch: String,
    pub agent: AgentKind,
}

/// What every sibling of a dialog launch shares.
#[derive(Debug, Clone)]
pub struct SiblingTemplate {
    pub project_repo: PathBuf,
    pub mode: VibeMode,
    pub enable_chrome: bool,
    pub allowed_agents: Vec<AgentKind>,
}

/// A dialog launch in progress (also a single-agent one with a prompt, which
/// only needs `send`).
#[derive(Debug, Clone)]
pub struct FanOutRun {
    pub project_name: String,
    pub base_branch: String,
    pub layout: FanOutLayout,
    pub agents: Vec<AgentKind>,
    pub prompt: Option<String>,
    pub send: bool,
    pub group_id: Option<String>,
    pub template: SiblingTemplate,
    pub remaining: VecDeque<SiblingSpec>,
    /// Shared layout: `(agent, label)` windows to add once the feature runs.
    pub extra_shared: Vec<(AgentKind, String)>,
    /// Ids of the features created so far, in order.
    pub created: Vec<String>,
    pub mode_note: Option<String>,
}

impl FanOutRun {
    fn sibling_state(&self, spec: &SiblingSpec, prompt: &str) -> CreateFeatureState {
        let mut state = CreateFeatureState::new(
            self.project_name.clone(),
            self.template.project_repo.clone(),
            Vec::new(),
            false,
        );
        state.branch = spec.branch.clone();
        state.use_worktree = true;
        state.agent = spec.agent.clone();
        state.agent_index = self
            .template
            .allowed_agents
            .iter()
            .position(|a| *a == spec.agent)
            .unwrap_or(0);
        state.allowed_agents = self.template.allowed_agents.clone();
        state.mode = self.template.mode.clone();
        state.mode_index = VibeMode::ALL
            .iter()
            .position(|m| *m == self.template.mode)
            .unwrap_or(0);
        state.enable_chrome = self.template.enable_chrome && spec.agent == AgentKind::Claude;
        state.session_name = App::default_session_name_for_agent(&spec.agent);
        state.task_prompt = prompt.to_string();
        state.send_prompt = self.send;
        state.step = CreateFeatureStep::Prompt;
        state
    }
}

/// Runtime state on `App`.
#[derive(Default)]
pub struct FanOutRuntime {
    pub run: Option<FanOutRun>,
    /// Prompts waiting for their agent, by feature session id.
    pub pending: HashMap<String, PendingPrompt>,
    /// Enters still to press after a "send when ready" paste.
    enters: HashMap<String, Instant>,
    /// When the fallback first saw the agent running in the pane.
    agent_seen: HashMap<String, Instant>,
    last_pane_check: Option<Instant>,
    /// Fan-out group membership, by feature id.
    pub members: HashMap<String, GroupMember>,
}

impl App {
    // ── start-up and cleanup ─────────────────────────────────────

    /// Load pending prompts and group membership (start-up). A loaded prompt
    /// starts its fallback clock now: the agent may already be running.
    pub(crate) fn load_fan_out_state(&mut self) {
        let Some(db) = self.db.as_ref() else {
            return;
        };
        let now = Instant::now();
        let prompts = db.all_pending_prompts();
        let members = db.all_fanout_members();
        match prompts {
            Ok(prompts) => {
                for stored in prompts {
                    self.fan_out.pending.insert(
                        stored.session_id,
                        PendingPrompt::new(stored.text, stored.send, false, now),
                    );
                }
            }
            Err(e) => self.log_warn("fan-out", format!("could not load pending prompts: {e}")),
        }
        match members {
            Ok(members) => {
                for m in members {
                    self.fan_out.members.insert(
                        m.feature_id,
                        GroupMember {
                            group_id: m.group_id,
                            base_branch: m.base_branch,
                            agent: m.agent,
                            position: m.position,
                        },
                    );
                }
            }
            Err(e) => self.log_warn("fan-out", format!("could not load fan-out groups: {e}")),
        }
    }

    /// A session was deleted: drop its pending prompt.
    pub(crate) fn forget_fan_out_session(&mut self, session_id: &str) {
        self.fan_out.enters.remove(session_id);
        self.fan_out.agent_seen.remove(session_id);
        if self.fan_out.pending.remove(session_id).is_some()
            && let Some(db) = self.db.as_ref()
            && let Err(e) = db.delete_pending_prompt(session_id)
        {
            self.log_warn("fan-out", format!("could not delete a pending prompt: {e}"));
        }
    }

    /// A feature was deleted: drop its sessions' prompts and its group seat.
    pub(crate) fn forget_fan_out_feature(&mut self, feature: &crate::project::Feature) {
        for session in &feature.sessions {
            self.forget_fan_out_session(&session.id);
        }
        if self.fan_out.members.remove(&feature.id).is_some()
            && let Some(db) = self.db.as_ref()
            && let Err(e) = db.delete_fanout_member(&feature.id)
        {
            self.log_warn("fan-out", format!("could not delete a group seat: {e}"));
        }
    }

    /// The dialog was cancelled: a half-done fan-out stops where it is.
    pub(crate) fn fan_out_cancelled(&mut self) {
        if let Some(run) = self.fan_out.run.take()
            && !run.created.is_empty()
            && !run.remaining.is_empty()
        {
            let left: Vec<String> = run.remaining.iter().map(|s| s.branch.clone()).collect();
            self.push_toast_warning(format!(
                "Fan-out stopped: {} not created ({})",
                left.len(),
                left.join(", ")
            ));
        }
    }

    // ── the dialog launch ────────────────────────────────────────

    /// `Enter` on the Prompt step: create the feature(s) the dialog
    /// describes.
    pub fn create_feature_from_dialog(&mut self) -> Result<()> {
        // A sibling whose name was refused is being corrected: its launch
        // continues the run that is already under way.
        if self
            .fan_out
            .run
            .as_ref()
            .is_some_and(|run| !run.created.is_empty())
        {
            if let AppMode::CreatingFeature(state) = &mut self.mode {
                state.task_prompt = normalize_prompt(&state.task_prompt);
            }
            return self.create_feature();
        }
        let (run, check_branches) = {
            let AppMode::CreatingFeature(state) = &mut self.mode else {
                return Ok(());
            };
            let prompt = normalize_prompt(&state.task_prompt);
            state.task_prompt = prompt.clone();
            let prompt = (!prompt.is_empty()).then_some(prompt);
            let agents = state.selected_agents();
            let base_branch = state.branch.trim().to_string();
            let mut run = FanOutRun {
                project_name: state.project_name.clone(),
                base_branch: base_branch.clone(),
                layout: state.effective_fan_out(),
                agents: agents.clone(),
                prompt,
                send: state.send_prompt,
                group_id: None,
                template: SiblingTemplate {
                    project_repo: state.project_repo.clone(),
                    mode: state.mode.clone(),
                    enable_chrome: state.enable_chrome,
                    allowed_agents: state.allowed_agents.clone(),
                },
                remaining: VecDeque::new(),
                extra_shared: Vec::new(),
                created: Vec::new(),
                mode_note: None,
            };
            let mut check_branches = None;
            if agents.len() > 1 {
                let (mode, note) = fan_out_mode(&agents, &state.mode);
                state.mode = mode.clone();
                run.template.mode = mode;
                run.mode_note = note;
                state.agent = agents[0].clone();
                match run.layout {
                    FanOutLayout::Shared => {
                        let labels = shared_labels(&agents);
                        state.session_name = labels[0].clone();
                        run.extra_shared = agents[1..]
                            .iter()
                            .cloned()
                            .zip(labels[1..].iter().cloned())
                            .collect();
                    }
                    FanOutLayout::WorktreeEach => {
                        let branches = sibling_branches(&base_branch, &agents);
                        state.branch = branches[0].clone();
                        state.feature_name = None;
                        state.session_name = App::default_session_name_for_agent(&agents[0]);
                        run.remaining = branches[1..]
                            .iter()
                            .zip(agents[1..].iter())
                            .map(|(branch, agent)| SiblingSpec {
                                branch: branch.clone(),
                                agent: agent.clone(),
                            })
                            .collect();
                        run.group_id = Some(uuid::Uuid::new_v4().to_string());
                        check_branches = Some(branches);
                    }
                }
            }
            (run, check_branches)
        };
        // Every sibling's name is checked before the first worktree exists.
        if let Some(branches) = check_branches
            && let Some(message) = self.fan_out_branch_error(&run.project_name, &branches)
        {
            if let AppMode::CreatingFeature(state) = &mut self.mode {
                state.branch = run.base_branch.clone();
                state.branch_error = Some(message.clone());
            }
            self.push_toast_warning(message);
            return Ok(());
        }
        let worktree_each = run.layout == FanOutLayout::WorktreeEach && run.agents.len() > 1;
        let base = run.base_branch.clone();
        self.fan_out.run = Some(run);
        let result = self.create_feature();
        // Refused before anything was launched (validation, a worktree that
        // could not be created): the dialog stays as the user left it.
        if let AppMode::CreatingFeature(state) = &mut self.mode
            && self
                .fan_out
                .run
                .as_ref()
                .is_some_and(|run| run.created.is_empty())
        {
            if worktree_each {
                state.branch = base;
            }
            self.fan_out.run = None;
        }
        if result.is_err() {
            self.fan_out.run = None;
        }
        result
    }

    /// Why a set of sibling branches cannot be created, if one cannot.
    fn fan_out_branch_error(&self, project_name: &str, branches: &[String]) -> Option<String> {
        let project = self.store.find_project(project_name)?;
        for branch in branches {
            if let Err(error) = crate::fork::branch_name::validate_branch_name(branch) {
                return Some(format!("'{branch}': {error}"));
            }
            let normalized = normalized_feature_name(branch);
            if project
                .features
                .iter()
                .any(|f| normalized_feature_name(&f.name) == normalized)
            {
                return Some(format!(
                    "Feature '{branch}' already exists in '{project_name}' (fan-out needs it free)"
                ));
            }
        }
        None
    }

    /// The task prompt of a launch that just started its feature: queued for
    /// the feature's first agent instead of pasted at once, whenever the
    /// pane can be watched (the cockpit) or a dialog launch is under way.
    /// Returns `false` when the caller should deliver it the old way (an
    /// embedding GUI collects it).
    pub(crate) fn queue_startup_prompt(&mut self, pi: usize, fi: usize, prompt: &str) -> bool {
        if !(self.cockpit_active() || self.fan_out.run.is_some()) {
            return false;
        }
        let Some(session_id) = self
            .store
            .projects
            .get(pi)
            .and_then(|p| p.features.get(fi))
            .and_then(|f| f.sessions.iter().find(|s| s.kind.is_agent_harness()))
            .map(|s| s.id.clone())
        else {
            return false;
        };
        let (send, focus) = match &self.fan_out.run {
            Some(run) => (run.send, run.created.is_empty()),
            None => (false, true),
        };
        self.queue_pending_prompt(&session_id, prompt, send, focus);
        true
    }

    /// `finish_feature_launch` created and (normally) started a feature of a
    /// dialog launch: add the shared windows, record the group seat, and
    /// create the next sibling.
    pub(crate) fn fan_out_feature_launched(&mut self, project_name: &str, feature_name: &str) {
        let Some(mut run) = self.fan_out.run.take() else {
            return;
        };
        let Some((pi, fi)) = self.store.projects.iter().enumerate().find_map(|(pi, p)| {
            (p.name == project_name)
                .then(|| p.features.iter().position(|f| f.name == feature_name))
                .flatten()
                .map(|fi| (pi, fi))
        }) else {
            return;
        };
        let feature_id = self.store.projects[pi].features[fi].id.clone();
        let feature_agent = self.store.projects[pi].features[fi].agent.clone();
        run.created.push(feature_id.clone());

        if let Some(group_id) = run.group_id.clone() {
            let member = GroupMember {
                group_id,
                base_branch: run.base_branch.clone(),
                agent: feature_agent,
                position: run.created.len() as i64 - 1,
            };
            self.record_fan_out_member(&feature_id, member);
        }

        if !run.extra_shared.is_empty() {
            let extras = std::mem::take(&mut run.extra_shared);
            let running = {
                let feature = &self.store.projects[pi].features[fi];
                self.tmux.session_exists(&feature.tmux_session)
            };
            if running {
                // The extras launch inside this run's context: their prompts
                // are not the primary's, so they never take the focus.
                self.fan_out.run = Some(run.clone());
                for (agent, label) in extras {
                    match self.create_agent_session_labeled_identified(
                        pi,
                        fi,
                        &label,
                        Some(agent),
                        StartIntent::Approved,
                    ) {
                        Ok((_, session_id, _)) => {
                            if let Some(prompt) = run.prompt.clone() {
                                self.queue_pending_prompt(&session_id, &prompt, run.send, false);
                            }
                        }
                        Err(e) => {
                            self.report_logged_error(
                                "fan-out",
                                format!("{label} did not start in '{feature_name}': {e}"),
                            );
                        }
                    }
                }
                self.fan_out.run = None;
            } else {
                let names: Vec<String> = extras.into_iter().map(|(_, l)| l).collect();
                self.push_toast_warning(format!(
                    "'{feature_name}' did not start, so {} were not added — start it (c), then add them with s",
                    names.join(", ")
                ));
            }
        }

        if let Some(next) = run.remaining.pop_front() {
            let prompt = run.prompt.clone().unwrap_or_default();
            let state = run.sibling_state(&next, &prompt);
            self.fan_out.run = Some(run);
            self.mode = AppMode::CreatingFeature(state);
            if let Err(e) = self.create_feature() {
                self.fan_out.run = None;
                self.report_logged_error(
                    "fan-out",
                    format!("fan-out stopped at '{}': {e}", next.branch),
                );
                return;
            }
            if matches!(self.mode, AppMode::CreatingFeature(_)) {
                // Refused (a name taken since the check, a worktree that
                // could not be created): the sibling's own dialog shows why;
                // Enter retries it and continues, Esc stops the fan-out.
                self.push_toast_warning(format!(
                    "Fan-out paused at '{}' — fix it and press Enter, or Esc to stop",
                    next.branch
                ));
            }
            return;
        }

        // Done: land on the first feature's primary agent.
        if run.agents.len() > 1 {
            if let Some(first) = run.created.first()
                && let Some((pi, fi)) = self.feature_indices_by_id(first)
            {
                let si = self.store.projects[pi].features[fi]
                    .sessions
                    .iter()
                    .position(|s| s.kind.is_agent_harness());
                self.selection = match si {
                    Some(si) => Selection::Session(pi, fi, si),
                    None => Selection::Feature(pi, fi),
                };
            }
            let what = match run.layout {
                FanOutLayout::Shared => {
                    format!("{} agents in '{}'", run.agents.len(), run.base_branch)
                }
                FanOutLayout::WorktreeEach => format!(
                    "{} sibling features for '{}'",
                    run.created.len(),
                    run.base_branch
                ),
            };
            let prompt_note = match (&run.prompt, run.send) {
                (None, _) => String::new(),
                (Some(_), false) => " — the prompt is pasted into each when it is ready".into(),
                (Some(_), true) => " — the prompt is sent to each when it is ready".into(),
            };
            let note = run
                .mode_note
                .as_deref()
                .map(|n| format!(" ({n})"))
                .unwrap_or_default();
            self.push_toast_success(format!("Started {what}{prompt_note}{note}"));
        }
    }

    fn record_fan_out_member(&mut self, feature_id: &str, member: GroupMember) {
        if let Some(db) = self.db.as_ref()
            && let Err(e) = db.save_fanout_member(&FanOutMember {
                feature_id: feature_id.to_string(),
                group_id: member.group_id.clone(),
                base_branch: member.base_branch.clone(),
                agent: member.agent.clone(),
                position: member.position,
            })
        {
            self.log_warn("fan-out", format!("could not save a group seat: {e}"));
        }
        self.fan_out.members.insert(feature_id.to_string(), member);
    }

    // ── pending prompts ──────────────────────────────────────────

    /// Queue `text` for session `session_id`, replacing what was waiting.
    pub(crate) fn queue_pending_prompt(
        &mut self,
        session_id: &str,
        text: &str,
        send: bool,
        focus: bool,
    ) {
        let text = normalize_prompt(text);
        if text.is_empty() {
            return;
        }
        if let Some(db) = self.db.as_ref()
            && let Err(e) = db.save_pending_prompt(session_id, &text, send)
        {
            self.log_warn(
                "fan-out",
                format!("could not save a pending prompt (kept in memory): {e}"),
            );
        }
        self.fan_out.agent_seen.remove(session_id);
        self.fan_out.pending.insert(
            session_id.to_string(),
            PendingPrompt::new(text, send, focus, Instant::now()),
        );
    }

    /// The prompt still waiting for `session_id`, if any.
    pub fn pending_prompt_for(&self, session_id: &str) -> Option<&PendingPrompt> {
        self.fan_out.pending.get(session_id)
    }

    /// The harness's `SessionStart` hook reported in for `session_id`.
    pub(crate) fn fan_out_session_ready(&mut self, session_id: &str) {
        if let Some(prompt) = self.fan_out.pending.get_mut(session_id) {
            prompt.mark_ready(Instant::now());
            self.log_debug(
                "fan-out",
                format!("{session_id}: ready, prompt due shortly"),
            );
        }
    }

    fn session_window_of(&self, session_id: &str) -> Option<(String, String, String)> {
        let (pi, fi, si) = self.session_indices_by_id(session_id)?;
        let feature = &self.store.projects[pi].features[fi];
        let session = &feature.sessions[si];
        Some((
            feature.tmux_session.clone(),
            session.tmux_window.clone(),
            session.label.clone(),
        ))
    }

    /// Deliver what is due. Called every loop turn; returns whether anything
    /// visible changed.
    pub fn pending_prompts_tick(&mut self) -> bool {
        let mut changed = false;
        let now = Instant::now();
        if !self.fan_out.enters.is_empty() {
            let due: Vec<String> = self
                .fan_out
                .enters
                .iter()
                .filter(|(_, at)| now >= **at)
                .map(|(id, _)| id.clone())
                .collect();
            for id in due {
                self.fan_out.enters.remove(&id);
                if let Some((session, window, label)) = self.session_window_of(&id) {
                    match self.tmux.send_key_name(&session, &window, "Enter") {
                        Ok(()) => {
                            self.log_info("fan-out", format!("sent the prompt to {label}"));
                            changed = true;
                        }
                        Err(e) => self.report_logged_error(
                            "fan-out",
                            format!(
                                "pasted the prompt into {label} but could not press Enter: {e}"
                            ),
                        ),
                    }
                }
            }
        }
        if self.fan_out.pending.is_empty() {
            return changed;
        }
        let fallback = Duration::from_secs(self.config.aiw.fan_out_ready_fallback_secs.max(1));
        let mut ready = Vec::new();
        let mut check = Vec::new();
        for (id, prompt) in &self.fan_out.pending {
            match prompt.due(now, fallback) {
                Due::Ready => ready.push(id.clone()),
                Due::IfAgentRuns => check.push(id.clone()),
                Due::Wait => {}
            }
        }
        for id in ready {
            changed |= self.deliver_pending_prompt(&id);
        }
        if check.is_empty()
            || self
                .fan_out
                .last_pane_check
                .is_some_and(|at| now.duration_since(at) < PANE_CHECK_INTERVAL)
        {
            return changed;
        }
        self.fan_out.last_pane_check = Some(now);
        let panes = crate::fork::session_reconcile::agent_panes(self.tmux.pane_commands());
        for id in check {
            let Some((session, window, _)) = self.session_window_of(&id) else {
                // The session is gone; so is its prompt.
                self.forget_fan_out_session(&id);
                continue;
            };
            let runs_agent = panes.get(&(session, window)).map(|pane| {
                !crate::fork::session_reconcile::is_plain_shell(&pane.pane.command)
                    || pane.pane.shell_has_children == Some(true)
            });
            if runs_agent == Some(true) {
                let seen = *self.fan_out.agent_seen.entry(id.clone()).or_insert(now);
                if now.duration_since(seen) >= AGENT_SETTLE {
                    self.log_debug(
                        "fan-out",
                        format!("{id}: no SessionStart hook came; the pane runs the agent"),
                    );
                    changed |= self.deliver_pending_prompt(&id);
                }
            } else {
                self.fan_out.agent_seen.remove(&id);
            }
        }
        changed
    }

    /// Paste the prompt waiting for `session_id` (and schedule its Enter).
    /// Returns whether it was delivered.
    fn deliver_pending_prompt(&mut self, session_id: &str) -> bool {
        let Some((pi, fi, si)) = self.session_indices_by_id(session_id) else {
            self.forget_fan_out_session(session_id);
            return false;
        };
        let (tmux_session, window, label, workdir) = {
            let feature = &self.store.projects[pi].features[fi];
            let session = &feature.sessions[si];
            (
                feature.tmux_session.clone(),
                session.tmux_window.clone(),
                session.label.clone(),
                feature.workdir.clone(),
            )
        };
        if !self.tmux.window_exists(&tmux_session, &window) {
            // Not running (yet): wait for the next start.
            if let Some(prompt) = self.fan_out.pending.get_mut(session_id) {
                prompt.ready_at = None;
                prompt.queued_at = Instant::now();
            }
            self.fan_out.agent_seen.remove(session_id);
            return false;
        }
        // Removed first: whatever happens below, it is never pasted twice.
        let Some(prompt) = self.fan_out.pending.remove(session_id) else {
            return false;
        };
        self.fan_out.agent_seen.remove(session_id);
        if let Some(db) = self.db.as_ref()
            && let Err(e) = db.delete_pending_prompt(session_id)
        {
            self.log_warn(
                "fan-out",
                format!("could not delete a delivered prompt: {e}"),
            );
        }
        if let Err(e) = self.tmux.paste_text(&tmux_session, &window, &prompt.text) {
            self.persist_startup_prompt(&workdir, &prompt.text);
            self.report_logged_error(
                "fan-out",
                format!(
                    "could not paste the prompt into {label}; it is in .claude/latest-prompt.txt: {e}"
                ),
            );
            return false;
        }
        if prompt.send {
            self.fan_out.enters.insert(
                session_id.to_string(),
                Instant::now() + crate::fork::fan_out::ENTER_DELAY,
            );
        }
        self.log_info(
            "fan-out",
            format!(
                "pasted the prompt into {label} ({})",
                if prompt.send { "sending" } else { "unsent" }
            ),
        );
        if prompt.focus && matches!(self.mode, AppMode::Normal) {
            self.selection = Selection::Session(pi, fi, si);
            self.mark_finished_seen(session_id);
            if self.cockpit_active() {
                self.cockpit_focus_agent();
            }
        }
        if prompt.send {
            self.push_toast_success(format!("Prompt sent to {label}"));
        } else {
            self.push_toast_success(format!(
                "Prompt pasted into {label} — read it, then press Enter to send"
            ));
        }
        true
    }

    // ── groups ───────────────────────────────────────────────────

    /// The feature indices of `(pi, fi)`'s fan-out group in this project,
    /// in group order; empty when it is not in one.
    pub fn fan_out_siblings(&self, pi: usize, fi: usize) -> Vec<usize> {
        let Some(project) = self.store.projects.get(pi) else {
            return Vec::new();
        };
        let Some(group) = project
            .features
            .get(fi)
            .and_then(|f| self.fan_out.members.get(&f.id))
            .map(|m| m.group_id.clone())
        else {
            return Vec::new();
        };
        let mut siblings: Vec<(i64, usize)> = project
            .features
            .iter()
            .enumerate()
            .filter_map(|(i, f)| {
                self.fan_out
                    .members
                    .get(&f.id)
                    .filter(|m| m.group_id == group)
                    .map(|m| (m.position, i))
            })
            .collect();
        siblings.sort();
        siblings.into_iter().map(|(_, i)| i).collect()
    }

    /// Whether `(pi, fi)` heads a fan-out group of two or more features in
    /// list order (the sidebar draws the group header above it).
    pub fn fan_out_heads_group(&self, pi: usize, fi: usize) -> bool {
        let siblings = self.fan_out_siblings(pi, fi);
        siblings.len() >= 2 && siblings.iter().min() == Some(&fi)
    }

    /// The header drawn above `(pi, fi)` when it is the first feature of its
    /// group (in list order) and the group still has two or more features:
    /// the header text and how many siblings need the user.
    pub fn fan_out_header(
        &self,
        pi: usize,
        fi: usize,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Option<(String, usize)> {
        if !self.fan_out_heads_group(pi, fi) {
            return None;
        }
        let siblings = self.fan_out_siblings(pi, fi);
        let project = &self.store.projects[pi];
        let member = self.fan_out.members.get(&project.features[fi].id)?;
        let agents: Vec<AgentKind> = siblings
            .iter()
            .map(|&i| project.features[i].agent.clone())
            .collect();
        let needs_you = siblings
            .iter()
            .filter(|&&i| {
                crate::fork::badges::feature_badge(self, pi, i, now).kind
                    == crate::fork::badges::BadgeKind::NeedsYou
            })
            .count();
        Some((
            crate::fork::fan_out::group_header(&member.base_branch, &agents),
            needs_you,
        ))
    }

    /// `v` on a fan-out sibling: the grid of the whole group (the first agent
    /// of each sibling). Returns `false` when the selection is not in a
    /// group, so `v` does what it always did.
    pub(crate) fn cockpit_show_group_grid(&mut self) -> bool {
        let (pi, fi) = match self.selection {
            Selection::Session(pi, fi, _) | Selection::Feature(pi, fi) => (pi, fi),
            Selection::Project(_) => return false,
        };
        let siblings = self.fan_out_siblings(pi, fi);
        if siblings.len() < 2 {
            return false;
        }
        let project = &self.store.projects[pi];
        let names: Vec<String> = siblings
            .iter()
            .map(|&i| format!("{}/{}", project.name, project.features[i].name))
            .collect();
        let label = self
            .fan_out
            .members
            .get(&project.features[fi].id)
            .map(|m| m.base_branch.clone())
            .unwrap_or_else(|| project.features[fi].name.clone());
        self.cockpit_show_grid_for(&label, &names);
        true
    }

    // ── automation ───────────────────────────────────────────────

    /// `aiw automation create-feature` with `agents`, `fan_out`, `prompt`
    /// and `send` (one agent with a prompt comes here too).
    pub fn create_feature_from_request_fan_out(
        &mut self,
        request: &CreateFeatureRequest,
    ) -> Result<CreateFeatureResponse> {
        let agents = request.agent_list();
        let prompt = request
            .prompt
            .as_deref()
            .map(normalize_prompt)
            .filter(|p| !p.is_empty());
        if agents.len() <= 1 {
            let mut single = request.clone();
            single.agent = agents[0].clone();
            let response = self.create_feature_from_request(&single)?;
            if let (Some(prompt), Some(feature_id)) =
                (prompt.as_deref(), response.feature_id.clone())
            {
                // Scripted: never steals the focus.
                self.queue_request_prompts(&feature_id, prompt, request.send);
            }
            return Ok(response);
        }

        let has_features = self
            .store
            .find_project(&request.project_name)
            .map(|p| !p.features.is_empty())
            .ok_or_else(|| anyhow::anyhow!("Project '{}' not found", request.project_name))?;
        let use_worktree = request.use_worktree.unwrap_or(has_features);
        let layout = request.fan_out.unwrap_or(if use_worktree {
            FanOutLayout::WorktreeEach
        } else {
            FanOutLayout::Shared
        });
        if layout == FanOutLayout::WorktreeEach && !use_worktree {
            anyhow::bail!(
                "fan_out \"worktree_each\" needs worktrees; set use_worktree true or use \"shared\""
            );
        }
        for agent in &agents {
            if !self.allows_agent_for_repo(
                &self
                    .store
                    .find_project(&request.project_name)
                    .map(|p| p.repo.clone())
                    .unwrap_or_default(),
                agent,
            ) {
                anyhow::bail!(
                    "Harness '{}' is not allowed for this workspace",
                    agent.display_name()
                );
            }
        }
        let (mode, note) = fan_out_mode(&agents, &request.mode);
        let mut base = request.clone();
        base.agents = Vec::new();
        base.mode = mode;
        base.prompt = None;
        base.use_worktree = Some(use_worktree);

        match layout {
            FanOutLayout::Shared => {
                let labels = shared_labels(&agents);
                let mut primary = base.clone();
                primary.agent = agents[0].clone();
                let mut response = self.create_feature_from_request(&primary)?;
                if request.dry_run {
                    response.message = format!(
                        "Dry run: would create feature '{}' with {} agents ({})",
                        request.branch,
                        agents.len(),
                        labels.join(", ")
                    );
                    return Ok(response);
                }
                let feature_id = response.feature_id.clone().unwrap_or_default();
                let mut sessions = Vec::new();
                if let Some((pi, fi)) = self.feature_indices_by_id(&feature_id) {
                    // The primary has its default label, `labels[0]`.
                    if let Some(s) = self.store.projects[pi].features[fi]
                        .sessions
                        .iter()
                        .find(|s| s.kind.is_agent_harness())
                    {
                        sessions.push((s.label.clone(), s.id.clone()));
                    }
                    if response.started {
                        for (agent, label) in agents[1..].iter().zip(labels[1..].iter()) {
                            let (_, session_id, _) = self.create_agent_session_labeled_identified(
                                pi,
                                fi,
                                label,
                                Some(agent.clone()),
                                StartIntent::Approved,
                            )?;
                            sessions.push((label.clone(), session_id));
                        }
                    }
                    self.save()?;
                }
                if let Some(prompt) = prompt.as_deref() {
                    for (_, session_id) in &sessions {
                        self.queue_pending_prompt(session_id, prompt, request.send, false);
                    }
                }
                response.features = vec![FanOutFeature {
                    branch: response.branch.clone(),
                    agent: agents[0].clone(),
                    workdir: response.workdir.clone(),
                    tmux_session: response.tmux_session.clone(),
                    feature_id: response.feature_id.clone(),
                    sessions,
                }];
                if let Some(note) = note {
                    response.message = format!("{} ({note})", response.message);
                }
                Ok(response)
            }
            FanOutLayout::WorktreeEach => {
                let branches = sibling_branches(&request.branch, &agents);
                if let Some(message) = self.fan_out_branch_error(&request.project_name, &branches) {
                    anyhow::bail!(message);
                }
                let group_id = uuid::Uuid::new_v4().to_string();
                let mut features = Vec::new();
                let mut first: Option<CreateFeatureResponse> = None;
                let mut all_started = true;
                for (i, (branch, agent)) in branches.iter().zip(agents.iter()).enumerate() {
                    let mut sibling = base.clone();
                    sibling.branch = branch.clone();
                    sibling.agent = agent.clone();
                    let response = self.create_feature_from_request(&sibling)?;
                    all_started &= response.started;
                    let mut sessions = Vec::new();
                    if let Some(feature_id) = response.feature_id.clone() {
                        self.record_fan_out_member(
                            &feature_id,
                            GroupMember {
                                group_id: group_id.clone(),
                                base_branch: request.branch.trim().to_string(),
                                agent: agent.clone(),
                                position: i as i64,
                            },
                        );
                        if let Some((pi, fi)) = self.feature_indices_by_id(&feature_id)
                            && let Some(s) = self.store.projects[pi].features[fi]
                                .sessions
                                .iter()
                                .find(|s| s.kind.is_agent_harness())
                        {
                            sessions.push((s.label.clone(), s.id.clone()));
                        }
                        if let Some(prompt) = prompt.as_deref() {
                            self.queue_request_prompts(&feature_id, prompt, request.send);
                        }
                    }
                    features.push(FanOutFeature {
                        branch: response.branch.clone(),
                        agent: agent.clone(),
                        workdir: response.workdir.clone(),
                        tmux_session: response.tmux_session.clone(),
                        feature_id: response.feature_id.clone(),
                        sessions,
                    });
                    if first.is_none() {
                        first = Some(response);
                    }
                }
                let mut response = first.expect("at least two agents");
                response.branch = request.branch.clone();
                response.started = all_started;
                response.message = format!(
                    "{} {} sibling features for '{}': {}{}",
                    if request.dry_run {
                        "Dry run: would create"
                    } else {
                        "Created"
                    },
                    features.len(),
                    request.branch,
                    branches.join(", "),
                    note.map(|n| format!(" ({n})")).unwrap_or_default()
                );
                response.features = features;
                Ok(response)
            }
        }
    }

    /// Queue `prompt` for the first agent session of feature `feature_id`
    /// (scripted: it never takes the focus).
    fn queue_request_prompts(&mut self, feature_id: &str, prompt: &str, send: bool) {
        let Some((pi, fi)) = self.feature_indices_by_id(feature_id) else {
            return;
        };
        let first: Option<String> = self.store.projects[pi].features[fi]
            .sessions
            .iter()
            .find(|s| s.kind.is_agent_harness())
            .map(|s| s.id.clone());
        if let Some(id) = first {
            self.queue_pending_prompt(&id, prompt, send, false);
        }
    }
}
