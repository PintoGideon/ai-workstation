//! FORK: `P` lands a feature (DESIGN.md §19.10): the app side of
//! [`crate::fork::land`].
//!
//! The dialog opens at once and fills when the preflight (git, on a helper
//! thread) answers. `Enter` lands: the repository's `pre_land` / `post_land`
//! hooks go through the usual one-time approval, a `land` terminal in the
//! feature tails the log, and the worker runs on its own thread. The row reads
//! `landing · <stage>` meanwhile and `⛔ land failed: <reason>` after a
//! failure, until the next successful land or `x` in the dialog.
//!
//! After a successful land the app does what git cannot: remove the feature
//! (its sessions, worktree and ports, through the normal delete flow) and the
//! branch, or mark a kept feature ready. That waits until no dialog is open,
//! so the user sees the result first.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::app::{App, AppMode, Selection};
use crate::fork::hook_approval::{HookName, HookResume};
use crate::fork::land::{
    self, Event, Failure, HookRun, InProgress, Job, LAND_ACTION, LAND_TERMINAL, Outcome, Preflight,
    PreflightInput, Stage,
};
use crate::project::{ProjectStatus, SessionKind};

/// How long a removed feature's branch deletion waits for the feature to go.
const BRANCH_DELETE_PATIENCE: Duration = Duration::from_secs(300);

/// A land that is running.
struct Running {
    repo: PathBuf,
    feature: String,
    stage: Stage,
    events: Receiver<Event>,
    log: PathBuf,
    strategy: land::Strategy,
    branch: String,
    started: Instant,
}

/// What happened last, per feature (for the dialog and `status`).
#[derive(Debug, Clone)]
pub struct LastLand {
    pub outcome: Outcome,
    pub finished: chrono::DateTime<chrono::Utc>,
    /// The feature's name and project, so `status` still answers after
    /// the land removed the feature.
    pub feature: String,
    pub project: String,
}

/// App-side work after a successful land.
#[derive(Debug, Clone)]
struct Cleanup {
    feature_id: String,
    project: String,
    feature: String,
    repo: PathBuf,
    branch: String,
    remove_worktree: bool,
    delete_branch: bool,
    mark_done: bool,
}

#[derive(Debug, Clone)]
struct BranchDelete {
    feature_id: String,
    repo: PathBuf,
    branch: String,
    since: Instant,
}

/// The land state on [`App`].
#[derive(Default)]
pub struct LandRuntime {
    preflights: HashMap<String, Receiver<Preflight>>,
    running: HashMap<String, Running>,
    failures: HashMap<String, Failure>,
    last: HashMap<String, LastLand>,
    cleanups: Vec<Cleanup>,
    branch_deletes: Vec<BranchDelete>,
    /// Tests: where locks and logs go, and the `gh` binary.
    pub(crate) state_dir: Option<PathBuf>,
    pub(crate) gh: Option<String>,
}

/// What the `P` dialog shows.
#[derive(Debug, Clone)]
pub struct LandDialogState {
    pub feature_id: String,
    pub feature: String,
    pub project: String,
    pub preflight: Option<Preflight>,
    /// Agents in the feature that are working / waiting on you.
    pub working: Vec<String>,
    pub waiting: Vec<String>,
    /// `Enter` was pressed with agents working: `y` lands anyway.
    pub confirm_working: bool,
    /// A line under the title: what the last key did.
    pub note: Option<String>,
}

impl App {
    fn land_state_dir(&self) -> PathBuf {
        self.land
            .state_dir
            .clone()
            .unwrap_or_else(crate::ipc::state_dir)
    }

    fn land_gh(&self) -> String {
        self.land.gh.clone().unwrap_or_else(|| "gh".to_string())
    }

    // ── queries ──────────────────────────────────────────────────────

    /// The stage of a running land of `feature_id`.
    pub(crate) fn land_stage(&self, feature_id: &str) -> Option<Stage> {
        self.land.running.get(feature_id).map(|r| r.stage)
    }

    /// Why the last land of `feature_id` failed, until dismissed.
    pub(crate) fn land_failure(&self, feature_id: &str) -> Option<&Failure> {
        self.land.failures.get(feature_id)
    }

    /// Features whose last land failed and has not been dismissed, in a
    /// stable order (the needs-you list shows one row each).
    pub(crate) fn land_failed_features(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.land.failures.keys().cloned().collect();
        ids.sort();
        ids
    }

    /// Forget `feature_id`'s last failure (the needs-you list's `x`).
    pub(crate) fn forget_land_failure(&mut self, feature_id: &str) -> bool {
        self.land.failures.remove(feature_id).is_some()
    }

    pub(crate) fn land_last(&self, feature_id: &str) -> Option<&LastLand> {
        self.land.last.get(feature_id)
    }

    /// The line under a feature row while it lands or after a failure.
    pub(crate) fn land_line(&self, feature_id: &str) -> Option<String> {
        if let Some(stage) = self.land_stage(feature_id) {
            return Some(format!("⇡ landing · {}", stage.label()));
        }
        self.land_failure(feature_id)
            .map(|f| format!("⛔ land failed: {} (P)", f.reason))
    }

    /// Another land is running in `repo` (not `except`).
    fn land_running_in_repo(&self, repo: &std::path::Path, except: &str) -> Option<String> {
        self.land
            .running
            .iter()
            .find(|(id, r)| id.as_str() != except && r.repo == repo)
            .map(|(_, r)| r.feature.clone())
    }

    // ── opening ──────────────────────────────────────────────────────

    /// `P` on the dashboard.
    pub fn open_land_for_selection(&mut self) {
        let (pi, fi) = match &self.selection {
            Selection::Feature(pi, fi) | Selection::Session(pi, fi, _) => (*pi, *fi),
            _ => {
                self.message = Some("Select a feature to land".into());
                return;
            }
        };
        let Some(id) = self
            .store
            .projects
            .get(pi)
            .and_then(|p| p.features.get(fi))
            .map(|f| f.id.clone())
        else {
            return;
        };
        self.open_land_dialog(&id);
    }

    /// Agents in the feature at `(pi, fi)`: `(working, waiting on you)` labels.
    fn land_agent_states(&self, pi: usize, fi: usize) -> (Vec<String>, Vec<String>) {
        let mut working = Vec::new();
        let mut waiting = Vec::new();
        let feature = &self.store.projects[pi].features[fi];
        for (si, session) in feature.sessions.iter().enumerate() {
            if !session.kind.is_agent_harness() {
                continue;
            }
            if let Some(obs) = self.watchdog_observe(pi, fi, si) {
                if obs.waiting_on_user {
                    waiting.push(session.label.clone());
                } else if obs.working {
                    working.push(session.label.clone());
                }
            }
        }
        (working, waiting)
    }

    fn land_preflight_input(
        &self,
        feature_id: &str,
    ) -> Option<(PreflightInput, Vec<String>, Vec<String>)> {
        let (pi, fi) = self.feature_indices_by_id(feature_id)?;
        let project = &self.store.projects[pi];
        let feature = &project.features[fi];
        let ext = self.extension_for_repo(&project.repo);
        let (working, waiting) = self.land_agent_states(pi, fi);
        let mut extra_blockers = Vec::new();
        let mut extra_warnings = Vec::new();
        if let Some(other) = self.land_running_in_repo(&project.repo, feature_id) {
            extra_blockers.push(format!(
                "another land is running in this repository ({other}); one at a time"
            ));
        }
        if self.land.running.contains_key(feature_id) {
            extra_blockers.push("this feature is landing now".into());
        }
        if !working.is_empty() {
            extra_warnings.push(format!(
                "working now: {} — landing needs a second confirm (y)",
                working.join(", ")
            ));
        }
        if !waiting.is_empty() {
            extra_warnings.push(format!("waiting on you: {}", waiting.join(", ")));
        }
        for (hook, config) in [
            ("pre_land", ext.lifecycle_hooks.pre_land.as_ref()),
            ("post_land", ext.lifecycle_hooks.post_land.as_ref()),
        ] {
            if let Some(config) = config {
                extra_warnings.push(format!("{hook} hook: {}", config.script()));
            }
        }
        Some((
            PreflightInput {
                feature: feature.name.clone(),
                project: project.name.clone(),
                repo: project.repo.clone(),
                workdir: feature.workdir.clone(),
                is_worktree: feature.is_worktree,
                branch: feature.branch.clone(),
                config: ext.land.clone(),
                probe_gh: true,
                extra_blockers,
                extra_warnings,
            },
            working,
            waiting,
        ))
    }

    /// Open the dialog for `feature_id` and start its preflight.
    pub(crate) fn open_land_dialog(&mut self, feature_id: &str) {
        let Some((input, working, waiting)) = self.land_preflight_input(feature_id) else {
            return;
        };
        let state = LandDialogState {
            feature_id: feature_id.to_string(),
            feature: input.feature.clone(),
            project: input.project.clone(),
            preflight: None,
            working,
            waiting,
            confirm_working: false,
            note: None,
        };
        // A running or just-failed land shows its state; the preflight still
        // runs so `r` / Enter have fresh facts.
        self.spawn_land_preflight(feature_id, input);
        self.mode = AppMode::Land(Box::new(state));
    }

    fn spawn_land_preflight(&mut self, feature_id: &str, input: PreflightInput) {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(land::compute_preflight(&input));
        });
        self.land.preflights.insert(feature_id.to_string(), rx);
    }

    /// `r` in the dialog: compute the preflight again.
    pub(crate) fn refresh_land_preflight(&mut self) {
        let Some(id) = self.land_dialog_feature() else {
            return;
        };
        let Some((input, working, waiting)) = self.land_preflight_input(&id) else {
            return;
        };
        if let AppMode::Land(state) = &mut self.mode {
            state.preflight = None;
            state.working = working;
            state.waiting = waiting;
            state.confirm_working = false;
            state.note = None;
        }
        self.spawn_land_preflight(&id, input);
    }

    fn land_dialog_feature(&self) -> Option<String> {
        match &self.mode {
            AppMode::Land(state) => Some(state.feature_id.clone()),
            _ => None,
        }
    }

    // ── landing ──────────────────────────────────────────────────────

    /// `Enter` (or `y` after the working-agents warning) in the dialog.
    pub(crate) fn confirm_land(&mut self) {
        let AppMode::Land(state) = &self.mode else {
            return;
        };
        let Some(preflight) = state.preflight.clone() else {
            self.set_land_note("the preflight is still running");
            return;
        };
        if !preflight.landable() {
            self.set_land_note("blocked — see the list above");
            return;
        }
        if !state.working.is_empty() && !state.confirm_working {
            if let AppMode::Land(state) = &mut self.mode {
                state.confirm_working = true;
                state.note = Some(format!(
                    "{} still working — y lands anyway, Esc keeps waiting",
                    state.working.join(", ")
                ));
            }
            return;
        }
        let feature_id = state.feature_id.clone();
        let Some((pi, fi)) = self.feature_indices_by_id(&feature_id) else {
            return;
        };
        let repo = self.store.projects[pi].repo.clone();
        let ext = self.extension_for_repo(&repo);
        // The repository's hooks need their one-time approval; the dialog
        // replays this method afterwards (HookResume::Land).
        let Some(pre) = self.gate_feature_hook(
            pi,
            fi,
            HookName::PreLand,
            ext.lifecycle_hooks.pre_land.clone(),
            HookResume::Land,
        ) else {
            return;
        };
        let Some(post) = self.gate_feature_hook(
            pi,
            fi,
            HookName::PostLand,
            ext.lifecycle_hooks.post_land.clone(),
            HookResume::Land,
        ) else {
            return;
        };
        let pre = pre.map(|c| HookRun {
            name: "pre_land",
            script: c.script().to_string(),
        });
        let post = post.map(|c| HookRun {
            name: "post_land",
            script: c.script().to_string(),
        });
        let started = self.start_land(&feature_id, &preflight, pre, post);
        if let AppMode::Land(state) = &mut self.mode {
            state.confirm_working = false;
            state.note = Some(match started {
                Ok(()) => "landing…".to_string(),
                Err(e) => e,
            });
        }
    }

    fn set_land_note(&mut self, note: &str) {
        if let AppMode::Land(state) = &mut self.mode {
            state.note = Some(note.to_string());
        }
    }

    /// Start the worker for `feature_id` with an approved `preflight`.
    pub(crate) fn start_land(
        &mut self,
        feature_id: &str,
        preflight: &Preflight,
        pre_land: Option<HookRun>,
        post_land: Option<HookRun>,
    ) -> Result<(), String> {
        if self.land.running.contains_key(feature_id) {
            return Err("this feature is landing now".into());
        }
        if let Some(other) = self.land_running_in_repo(&preflight.repo, feature_id) {
            return Err(format!(
                "another land is running in this repository ({other})"
            ));
        }
        let (pi, fi) = self
            .feature_indices_by_id(feature_id)
            .ok_or("the feature is gone")?;
        let feature = &self.store.projects[pi].features[fi];
        let tmux_session = feature.tmux_session.clone();
        let summary = std::fs::read_to_string(feature.workdir.join(".claude/latest-prompt.txt"))
            .map(|p| land::summary_line(&p))
            .unwrap_or_default();
        let state_dir = self.land_state_dir();
        let log = land::log_path(&state_dir, feature_id);
        let mut env = crate::fork::ports::env_for_tmux_session(&tmux_session);
        env.push(("AIW_FEATURE".into(), feature.name.clone()));
        let job = Job {
            feature_id: feature_id.to_string(),
            feature: feature.name.clone(),
            repo: preflight.repo.clone(),
            workdir: preflight.workdir.clone(),
            branch: preflight.branch.clone(),
            strategy: preflight.strategy.clone(),
            has_remote: preflight.facts_has_remote,
            state_dir,
            log: log.clone(),
            env,
            pre_land,
            post_land,
            summary,
            gh: self.land_gh(),
            before_finish: None,
        };
        // Start the log so the terminal has something to tail at once.
        if let Some(parent) = log.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&log, "");
        // Best effort: a land runs whether or not the terminal opens.
        let _ = self.open_land_terminal(pi, fi, &log);
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            land::run(job, tx);
        });
        self.land.failures.remove(feature_id);
        self.land.running.insert(
            feature_id.to_string(),
            Running {
                repo: preflight.repo.clone(),
                feature: preflight.feature.clone(),
                stage: Stage::Starting,
                events: rx,
                log,
                strategy: preflight.strategy.clone(),
                branch: preflight.branch.clone(),
                started: Instant::now(),
            },
        );
        self.log_info(
            "land",
            format!(
                "land started: {} ({} → {}, {} then {})",
                preflight.feature,
                preflight.branch,
                preflight.base,
                preflight.strategy.update.as_str(),
                preflight.strategy.finish.as_str()
            ),
        );
        Ok(())
    }

    /// The feature's `land` terminal, tailing `log`, when the feature runs.
    /// Opening it does not move the selection.
    fn open_land_terminal(
        &mut self,
        pi: usize,
        fi: usize,
        log: &std::path::Path,
    ) -> Option<(String, String)> {
        let (tmux_session, running) = {
            let feature = &self.store.projects[pi].features[fi];
            (
                feature.tmux_session.clone(),
                feature.status != ProjectStatus::Stopped
                    && self.tmux.session_exists(&feature.tmux_session),
            )
        };
        if !running {
            return None;
        }
        let existing = self.store.projects[pi].features[fi]
            .sessions
            .iter()
            .find(|s| s.label == LAND_TERMINAL && s.kind == SessionKind::Terminal)
            .map(|s| s.tmux_window.clone());
        let window = match existing {
            Some(window) => {
                if !self.tmux.window_exists(&tmux_session, &window) {
                    let workdir = self.store.projects[pi].features[fi].workdir.clone();
                    self.tmux
                        .create_window(&tmux_session, &window, &workdir)
                        .ok()?;
                } else {
                    // Stop an earlier tail.
                    let _ = self.tmux.send_key_name(&tmux_session, &window, "C-c");
                }
                window
            }
            None => {
                let selection = self.selection.clone();
                let message = self.message.clone();
                let created =
                    self.add_terminal_session_for_picker(pi, fi, Some(LAND_TERMINAL.into()));
                self.selection = selection;
                self.message = message;
                let id = created.ok()?;
                let (p, f, si) = self.session_indices_by_id(&id)?;
                self.store.projects[p].features[f].sessions[si]
                    .tmux_window
                    .clone()
            }
        };
        // A script, not a pipeline typed into the user's shell (which may
        // be fish): it follows the log and returns once the end marker is
        // written, so a fast land is never cut short.
        let script = log.with_extension("tail.sh");
        if std::fs::write(&script, land::tail_script(log)).is_err() {
            return None;
        }
        let command = format!("clear; sh {}", land::shell_quote(&script.to_string_lossy()));
        self.tmux
            .run_shell_command(&tmux_session, &window, &command)
            .ok()?;
        Some((tmux_session, window))
    }

    // ── the tick ─────────────────────────────────────────────────────

    /// Poll preflights and workers; run cleanups. `true` when something
    /// changed on screen.
    pub fn land_tick(&mut self) -> bool {
        let mut changed = false;
        // Preflights.
        let ids: Vec<String> = self.land.preflights.keys().cloned().collect();
        for id in ids {
            let result = self.land.preflights.get(&id).map(|rx| rx.try_recv());
            match result {
                Some(Ok(preflight)) => {
                    self.land.preflights.remove(&id);
                    if let AppMode::Land(state) = &mut self.mode
                        && state.feature_id == id
                    {
                        state.preflight = Some(preflight);
                        changed = true;
                    }
                }
                Some(Err(TryRecvError::Disconnected)) => {
                    self.land.preflights.remove(&id);
                }
                _ => {}
            }
        }
        // Workers.
        let ids: Vec<String> = self.land.running.keys().cloned().collect();
        for id in ids {
            while let Some(event) = self.land.running.get(&id).map(|r| r.events.try_recv()) {
                match event {
                    Ok(Event::Stage(stage)) => {
                        if let Some(r) = self.land.running.get_mut(&id) {
                            r.stage = stage;
                        }
                        changed = true;
                    }
                    Ok(Event::Finished(outcome)) => {
                        self.finish_land(*outcome);
                        changed = true;
                        break;
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        // The worker died without a result (it never panics,
                        // but be safe).
                        if let Some(r) = self.land.running.remove(&id) {
                            self.land.failures.insert(
                                id.clone(),
                                Failure {
                                    kind: land::FailureKind::Other,
                                    reason: "the land stopped unexpectedly".into(),
                                    in_progress: land::in_progress(
                                        &self
                                            .feature_indices_by_id(&id)
                                            .map(|(p, f)| {
                                                self.store.projects[p].features[f].workdir.clone()
                                            })
                                            .unwrap_or_default(),
                                    ),
                                },
                            );
                            self.log_warn(
                                "land",
                                format!("land worker for {} vanished", r.feature),
                            );
                        }
                        changed = true;
                        break;
                    }
                }
            }
        }
        // A deleted feature takes its failure with it.
        if !self.land.failures.is_empty() {
            let live: std::collections::HashSet<String> = self
                .store
                .projects
                .iter()
                .flat_map(|p| p.features.iter().map(|f| f.id.clone()))
                .collect();
            let before = self.land.failures.len();
            self.land.failures.retain(|id, _| live.contains(id));
            changed |= before != self.land.failures.len();
        }
        changed |= self.run_land_cleanups();
        changed |= self.run_land_branch_deletes();
        changed
    }

    fn finish_land(&mut self, outcome: Outcome) {
        let id = outcome.feature_id.clone();
        let Some(running) = self.land.running.remove(&id) else {
            return;
        };
        let took = running.started.elapsed().as_secs();
        let project = self
            .feature_indices_by_id(&id)
            .map(|(pi, _)| self.store.projects[pi].name.clone())
            .unwrap_or_default();
        self.land.last.insert(
            id.clone(),
            LastLand {
                outcome: outcome.clone(),
                finished: chrono::Utc::now(),
                feature: running.feature.clone(),
                project,
            },
        );
        if outcome.ok {
            self.land.failures.remove(&id);
            let what = match (&outcome.pr_url, &outcome.merged) {
                (Some(url), _) if outcome.pr_existed => format!("PR already open: {url}"),
                (Some(url), _) => format!("opened {url}"),
                (None, Some(_)) => format!("{} fast-forwarded", running.strategy.base),
                _ => "updated and checked".into(),
            };
            self.push_toast_success(format!("Landed {}: {what}", running.feature));
            self.log_info(
                "land",
                format!("land of {} succeeded in {took}s: {what}", running.feature),
            );
            if outcome.pr_url.is_some() {
                self.sync_active_prs_background();
            }
            if running.strategy.finish != land::Finish::None || running.strategy.remove_worktree {
                if let Some((pi, fi)) = self.feature_indices_by_id(&id) {
                    self.land.cleanups.push(Cleanup {
                        feature_id: id.clone(),
                        project: self.store.projects[pi].name.clone(),
                        feature: self.store.projects[pi].features[fi].name.clone(),
                        repo: running.repo.clone(),
                        branch: running.branch.clone(),
                        remove_worktree: running.strategy.remove_worktree,
                        delete_branch: running.strategy.delete_branch
                            && running.strategy.finish != land::Finish::None,
                        mark_done: running.strategy.mark_done,
                    });
                }
            } else if running.strategy.mark_done {
                self.land.cleanups.push(Cleanup {
                    feature_id: id.clone(),
                    project: String::new(),
                    feature: running.feature.clone(),
                    repo: running.repo.clone(),
                    branch: running.branch.clone(),
                    remove_worktree: false,
                    delete_branch: false,
                    mark_done: true,
                });
            }
        } else if let Some(failure) = outcome.failure.clone() {
            self.push_toast_warning(format!(
                "Land of {} stopped: {}",
                running.feature, failure.reason
            ));
            self.log_warn(
                "land",
                format!("land of {} failed: {}", running.feature, failure.reason),
            );
            self.land.failures.insert(id.clone(), failure);
        }
    }

    /// After a success: remove the feature, or mark it ready. Waits for the
    /// dashboard (no dialog open).
    fn run_land_cleanups(&mut self) -> bool {
        if self.land.cleanups.is_empty() || !matches!(self.mode, AppMode::Normal) {
            return false;
        }
        let cleanup = self.land.cleanups.remove(0);
        let Some((pi, fi)) = self.feature_indices_by_id(&cleanup.feature_id) else {
            return true;
        };
        if cleanup.remove_worktree && self.store.projects[pi].features[fi].is_worktree {
            if cleanup.delete_branch {
                self.land.branch_deletes.push(BranchDelete {
                    feature_id: cleanup.feature_id.clone(),
                    repo: cleanup.repo.clone(),
                    branch: cleanup.branch.clone(),
                    since: Instant::now(),
                });
            }
            self.log_info(
                "land",
                format!("removing {} after its land", cleanup.feature),
            );
            self.mode = AppMode::DeletingFeature(cleanup.project.clone(), cleanup.feature.clone());
            if let Err(e) = self.delete_feature() {
                self.push_toast_warning(format!("Could not remove {}: {e}", cleanup.feature));
            }
            return true;
        }
        if cleanup.mark_done {
            let feature = &mut self.store.projects[pi].features[fi];
            if !feature.ready {
                feature.ready = true;
                if let Err(e) = self.save() {
                    self.log_warn("land", format!("could not save the ready mark: {e}"));
                }
            }
        }
        if cleanup.delete_branch {
            self.push_toast_warning(format!(
                "Kept branch {}: it is still checked out in the feature's worktree",
                cleanup.branch
            ));
        }
        true
    }

    /// Delete a landed branch once its feature (and worktree) is gone.
    fn run_land_branch_deletes(&mut self) -> bool {
        if self.land.branch_deletes.is_empty() {
            return false;
        }
        let mut changed = false;
        let pending = std::mem::take(&mut self.land.branch_deletes);
        for delete in pending {
            let gone = self.feature_indices_by_id(&delete.feature_id).is_none();
            if gone {
                match land::git(&delete.repo, &["branch", "-d", &delete.branch]) {
                    Ok(_) => self.log_info("land", format!("deleted branch {}", delete.branch)),
                    Err(e) => {
                        self.log_warn("land", format!("kept branch {}: {e}", delete.branch));
                        self.push_toast_warning(format!("Kept branch {}: {e}", delete.branch));
                    }
                }
                changed = true;
            } else if delete.since.elapsed() < BRANCH_DELETE_PATIENCE {
                self.land.branch_deletes.push(delete);
            } else {
                self.log_warn(
                    "land",
                    format!("kept branch {}: its feature was not removed", delete.branch),
                );
            }
        }
        changed
    }

    // ── dialog actions ───────────────────────────────────────────────

    /// `a`: abort a rebase or merge the land (or the user) left in progress.
    pub(crate) fn abort_land_in_progress(&mut self) {
        let Some(id) = self.land_dialog_feature() else {
            return;
        };
        let Some((pi, fi)) = self.feature_indices_by_id(&id) else {
            return;
        };
        let workdir = self.store.projects[pi].features[fi].workdir.clone();
        match land::in_progress(&workdir) {
            Some(op) => match land::git(&workdir, &op.abort_args()) {
                Ok(_) => {
                    self.log_info(
                        "land",
                        format!("aborted the {} in {}", op.as_str(), workdir.display()),
                    );
                    self.refresh_land_preflight();
                    self.set_land_note(&format!("aborted the {}", op.as_str()));
                }
                Err(e) => self.set_land_note(&format!("git {} --abort failed: {e}", op.as_str())),
            },
            None => self.set_land_note("nothing is in progress"),
        }
    }

    /// `x`: forget the last failure.
    pub(crate) fn dismiss_land_failure(&mut self) {
        let Some(id) = self.land_dialog_feature() else {
            return;
        };
        if self.land.failures.remove(&id).is_some() {
            self.set_land_note("dismissed");
        }
    }

    /// `t`: a terminal in the feature's worktree.
    pub(crate) fn open_land_shell(&mut self) {
        let Some(id) = self.land_dialog_feature() else {
            return;
        };
        let Some((pi, fi)) = self.feature_indices_by_id(&id) else {
            return;
        };
        let feature = &self.store.projects[pi].features[fi];
        if feature.status == ProjectStatus::Stopped
            || !self.tmux.session_exists(&feature.tmux_session)
        {
            self.set_land_note("start the feature first (c) to open a terminal in it");
            return;
        }
        match self.add_terminal_session_for_picker(pi, fi, None) {
            Ok(_) => {
                self.mode = AppMode::Normal;
            }
            Err(e) => self.set_land_note(&format!("could not open a terminal: {e}")),
        }
    }

    /// The dialog's lines of log, for a running or finished land.
    pub(crate) fn land_dialog_log(&self, feature_id: &str, n: usize) -> Vec<String> {
        let path = self
            .land
            .running
            .get(feature_id)
            .map(|r| r.log.clone())
            .or_else(|| {
                self.land
                    .last
                    .get(feature_id)
                    .map(|l| l.outcome.log.clone())
            });
        path.map(|p| land::log_tail(&p, n)).unwrap_or_default()
    }

    // ── automation ───────────────────────────────────────────────────

    /// `aiw automation land`: ops `dry_run`, `start`, `status`, `abort`.
    pub(crate) fn automation_land(&mut self, raw: &Value) -> Value {
        let op = raw
            .get("op")
            .and_then(Value::as_str)
            .unwrap_or("start")
            .to_string();
        let mut out = match self.automation_land_inner(&op, raw) {
            Ok(value) => value,
            Err(e) => json!({"ok": false, "error": e}),
        };
        out["type"] = Value::String(crate::automation::AUTOMATION_RESULT_TYPE.into());
        out["action"] = Value::String(LAND_ACTION.into());
        out["op"] = Value::String(op);
        if out.get("ok").is_none() {
            out["ok"] = Value::Bool(true);
        }
        out
    }

    fn land_feature_by_name(&self, name: &str, project: Option<&str>) -> Result<String, String> {
        let mut hits = Vec::new();
        for p in &self.store.projects {
            if project.is_some_and(|wanted| wanted != p.name) {
                continue;
            }
            for f in &p.features {
                if f.id == name
                    || f.name == name
                    || f.nickname.as_deref() == Some(name)
                    || f.branch == name
                {
                    hits.push((p.name.clone(), f.id.clone()));
                }
            }
        }
        match hits.len() {
            0 => Err(format!("no feature named `{name}`")),
            1 => Ok(hits.remove(0).1),
            _ => Err(format!(
                "`{name}` matches features in {}; pass --project",
                hits.iter()
                    .map(|(p, _)| p.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }

    fn automation_land_inner(&mut self, op: &str, raw: &Value) -> Result<Value, String> {
        let name = raw
            .get("feature")
            .and_then(Value::as_str)
            .ok_or("--feature is required")?
            .to_string();
        let project = raw
            .get("project")
            .and_then(Value::as_str)
            .map(str::to_string);
        let found = self.land_feature_by_name(&name, project.as_deref());
        if op == "status" && found.is_err() {
            // A land that removed its feature still answers by name.
            let hit = self
                .land
                .last
                .iter()
                .find(|(id, last)| {
                    (last.feature == name || id.as_str() == name)
                        && project.as_deref().is_none_or(|p| p == last.project)
                })
                .map(|(id, _)| id.clone());
            if let Some(id) = hit {
                return Ok(self.land_status_json(&id));
            }
        }
        let id = found?;
        match op {
            "status" => Ok(self.land_status_json(&id)),
            "abort" => {
                let (pi, fi) = self
                    .feature_indices_by_id(&id)
                    .ok_or("the feature is gone")?;
                let workdir = self.store.projects[pi].features[fi].workdir.clone();
                let op = land::in_progress(&workdir).ok_or("nothing is in progress")?;
                land::git(&workdir, &op.abort_args())?;
                Ok(json!({"aborted": op.as_str()}))
            }
            "dry_run" | "start" => {
                let (input, working, waiting) = self
                    .land_preflight_input(&id)
                    .ok_or("the feature is gone")?;
                let preflight = land::compute_preflight(&input);
                let preflight_json = serde_json::to_value(&preflight).unwrap_or(Value::Null);
                if op == "dry_run" {
                    return Ok(json!({
                        "landable": preflight.landable(),
                        "working": working,
                        "waiting": waiting,
                        "preflight": preflight_json,
                    }));
                }
                if !preflight.landable() {
                    return Ok(json!({
                        "ok": false,
                        "error": "not landable",
                        "blockers": preflight.blockers,
                        "preflight": preflight_json,
                    }));
                }
                let yes = raw.get("yes").and_then(Value::as_bool).unwrap_or(false);
                if !working.is_empty() && !yes {
                    return Ok(json!({
                        "ok": false,
                        "error": format!("agents are working ({}); pass --yes to land anyway", working.join(", ")),
                        "preflight": preflight_json,
                    }));
                }
                let (pi, fi) = self
                    .feature_indices_by_id(&id)
                    .ok_or("the feature is gone")?;
                let repo = self.store.projects[pi].repo.clone();
                let workdir = self.store.projects[pi].features[fi].workdir.clone();
                let ext = self.extension_for_repo(&repo);
                let pre = self
                    .approved_repo_hook_or_refuse(
                        &repo,
                        HookName::PreLand,
                        ext.lifecycle_hooks.pre_land.clone(),
                        &workdir,
                    )?
                    .map(|c| HookRun {
                        name: "pre_land",
                        script: c.script().to_string(),
                    });
                let post = self
                    .approved_repo_hook_or_refuse(
                        &repo,
                        HookName::PostLand,
                        ext.lifecycle_hooks.post_land.clone(),
                        &workdir,
                    )?
                    .map(|c| HookRun {
                        name: "post_land",
                        script: c.script().to_string(),
                    });
                self.start_land(&id, &preflight, pre, post)?;
                Ok(json!({
                    "status": "started",
                    "plan": preflight.plan,
                    "preflight": preflight_json,
                }))
            }
            other => Err(format!(
                "unknown land op `{other}` (dry_run, start, status, abort)"
            )),
        }
    }

    fn land_status_json(&self, id: &str) -> Value {
        if let Some(r) = self.land.running.get(id) {
            return json!({
                "state": "running",
                "stage": r.stage,
                "log_tail": land::log_tail(&r.log, 20),
            });
        }
        match self.land.last.get(id) {
            Some(last) => json!({
                "state": if last.outcome.ok { "succeeded" } else { "failed" },
                "result": last.outcome,
                "finished": last.finished.to_rfc3339(),
                "failure_shown": self.land.failures.contains_key(id),
                "log_tail": land::log_tail(&last.outcome.log, 20),
            }),
            None => json!({"state": "idle"}),
        }
    }
}

/// An operation git left in progress, for the dialog's `a` hint.
pub(crate) fn abort_hint(op: Option<InProgress>) -> Option<String> {
    op.map(|op| format!("a aborts the {}", op.as_str()))
}

/// The dialog's lines (separate from drawing, for tests).
pub(crate) fn land_lines(
    app: &App,
    state: &LandDialogState,
    log_lines: usize,
) -> Vec<(String, LineTone)> {
    let mut lines: Vec<(String, LineTone)> = Vec::new();
    if let Some(stage) = app.land_stage(&state.feature_id) {
        lines.push((format!("⇡ landing · {}", stage.label()), LineTone::Working));
        lines.push((String::new(), LineTone::Plain));
        for line in app.land_dialog_log(&state.feature_id, log_lines) {
            lines.push((line, LineTone::Dim));
        }
        return lines;
    }
    if let Some(failure) = app.land_failure(&state.feature_id) {
        lines.push((
            format!("⛔ last land failed: {}", failure.reason),
            LineTone::Bad,
        ));
        if let Some(hint) = abort_hint(failure.in_progress) {
            lines.push((
                format!("   {hint}, or resolve it in a terminal (t)"),
                LineTone::Dim,
            ));
        }
        for line in app.land_dialog_log(&state.feature_id, 6) {
            lines.push((format!("   {line}"), LineTone::Dim));
        }
        lines.push((String::new(), LineTone::Plain));
    } else if let Some(last) = app.land_last(&state.feature_id)
        && last.outcome.ok
    {
        let what = match (&last.outcome.pr_url, &last.outcome.merged) {
            (Some(url), _) if last.outcome.pr_existed => format!("PR already open: {url}"),
            (Some(url), _) => format!("PR opened: {url}"),
            (None, Some(sha)) => format!("base fast-forwarded to {}", &sha[..sha.len().min(10)]),
            _ => "updated and checked".to_string(),
        };
        lines.push((format!("✓ landed · {what}"), LineTone::Good));
        lines.push((String::new(), LineTone::Plain));
    }
    let Some(pre) = &state.preflight else {
        lines.push((
            "Checking the branch, its base and the worktree…".into(),
            LineTone::Dim,
        ));
        return lines;
    };
    if pre.base.is_empty() {
        lines.push((format!("{} → (no base)", pre.branch), LineTone::Plain));
    } else {
        lines.push((
            format!(
                "{} → {}   (base: {})",
                pre.branch, pre.base, pre.base_source
            ),
            LineTone::Plain,
        ));
    }
    lines.push((
        format!(
            "{} ahead · {} behind {} · {} · finish {} ({})",
            pre.ahead,
            pre.behind,
            if pre.upstream.is_empty() {
                "base"
            } else {
                &pre.upstream
            },
            pre.strategy.update.as_str(),
            pre.strategy.finish.as_str(),
            pre.strategy.finish_reason
        ),
        LineTone::Plain,
    ));
    lines.push((
        match &pre.strategy.check_command {
            Some(check) => format!("check: {check}"),
            None => "check: none (set land.check_command in aiw.json)".into(),
        },
        LineTone::Dim,
    ));
    let mut after = Vec::new();
    if pre.strategy.finish != crate::fork::land::Finish::None || pre.strategy.remove_worktree {
        if pre.strategy.remove_worktree {
            after.push("remove the worktree");
        }
        if pre.strategy.delete_branch && pre.strategy.finish != crate::fork::land::Finish::None {
            after.push("delete the branch");
        }
    }
    if pre.strategy.mark_done && !pre.strategy.remove_worktree {
        after.push("mark ready");
    }
    if !after.is_empty() {
        lines.push((format!("afterwards: {}", after.join(", ")), LineTone::Dim));
    }
    if !pre.blockers.is_empty() {
        lines.push((String::new(), LineTone::Plain));
        for blocker in &pre.blockers {
            lines.push((format!("⛔ {blocker}"), LineTone::Bad));
        }
    }
    if !pre.warnings.is_empty() {
        lines.push((String::new(), LineTone::Plain));
        for warning in &pre.warnings {
            lines.push((format!("⚠ {warning}"), LineTone::Warn));
        }
    }
    lines.push((String::new(), LineTone::Plain));
    lines.push(("Plan:".into(), LineTone::Plain));
    for command in &pre.plan {
        lines.push((format!("  {command}"), LineTone::Dim));
    }
    lines
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LineTone {
    Plain,
    Dim,
    Good,
    Bad,
    Warn,
    Working,
}
