//! FORK: background Claude sessions (DESIGN.md §19.7) — the app side of
//! `fork::background`.
//!
//! - **Start.** A background session is created by the `s` picker's
//!   "Claude (background)" entry (then named, then given a first prompt) or
//!   by the New Feature dialog's "Run in the background" row. Either records
//!   a pending launch; [`App::background_tick`] runs `claude --bg` off the UI
//!   thread with AIW's usual Claude flags (`--settings` with the hooks,
//!   `--name`, the mode, the context paragraph) and stores the short id it
//!   prints.
//! - **State.** From `claude agents --json` (read every few seconds while a
//!   background session exists, §19.2's roster), overlaid with the session's
//!   own hooks (they reach AIW through the settings file's `env` block, since
//!   Claude's supervisor, not AIW's shell, starts the process).
//! - **Attach.** `Enter` opens a window in the feature's tmux session that
//!   runs `claude attach <id>` and closes itself on detach; the cockpit's
//!   agent pane shows it like any other session window.
//! - **Manage.** `x` → `claude stop`, `c` → `claude respawn` (or
//!   `claude --resume <conversation> --bg` when Claude no longer lists it),
//!   `d` → `claude rm`, with Claude's refusal shown and an explicit second
//!   confirmation for the flags it prints.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::time::{Duration, Instant};

use super::{App, AppMode, Selection};
use crate::db::background::BackgroundRow;
use crate::fork::background::{self as bg, BgState, RmRefusal, RunOutput};
use crate::fork::claude_roster::RosterEntry;
use crate::project::{Feature, LaunchOpts, SessionKind};

/// How often the roster is read while a background session exists.
pub const ROSTER_INTERVAL: Duration = Duration::from_secs(3);
/// How often attached windows are checked for a detach.
const WINDOW_CHECK: Duration = Duration::from_secs(2);
/// How old the selected session's log tail may get.
const LOG_REFRESH: Duration = Duration::from_secs(5);

/// What AIW keeps for one background session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BgRecord {
    /// The short id `claude --bg` printed (`attach`, `logs`, `stop`, …).
    pub bg_id: Option<String>,
    /// The conversation id, once the roster or a hook reported it.
    pub conversation_id: Option<String>,
}

/// A `claude --bg` waiting for the next tick.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PendingLaunch {
    pub prompt: Option<String>,
    /// Continue this conversation (`--resume <id> --bg`) instead.
    pub resume: Option<String>,
}

/// One `claude` command run for a background session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Op {
    Launch,
    Stop,
    Respawn,
    Remove,
    Logs,
}

struct OpDone {
    session_id: String,
    op: Op,
    result: Result<RunOutput, String>,
}

/// Runtime state on `App`.
#[derive(Default)]
pub struct BackgroundRuntime {
    pub records: HashMap<String, BgRecord>,
    pub pending: HashMap<String, PendingLaunch>,
    /// Features whose first Claude session the New Feature dialog asked to
    /// run in the background, by (project, feature name); claimed at start.
    pub pending_features: HashSet<(String, String)>,
    inflight: HashSet<(String, Op)>,
    channel: Option<(Sender<OpDone>, Receiver<OpDone>)>,
    /// The roster entry of each background session Claude lists.
    pub entries: HashMap<String, RosterEntry>,
    /// A roster read has been applied since AIW started.
    pub roster_seen: bool,
    /// Sessions whose attach window is open.
    pub attached: HashSet<String>,
    /// The last lines of `claude logs`, by session, with when they were read.
    pub logs: HashMap<String, (Instant, Vec<String>)>,
    /// The last failure of a session's `claude` command.
    pub errors: HashMap<String, String>,
    /// The `claude` binary to run (tests); resolved like every launch else.
    pub claude_bin: Option<String>,
    last_window_check: Option<Instant>,
}

impl BackgroundRuntime {
    /// Whether a command of `op` is running for `session_id`.
    pub fn is_running(&self, session_id: &str, op: Op) -> bool {
        self.inflight.contains(&(session_id.to_string(), op))
    }
}

/// The prompt dialog after naming a background session in the `s` picker.
#[derive(Debug, Clone)]
pub struct BackgroundPromptState {
    pub pi: usize,
    pub fi: usize,
    pub label: String,
    pub text: String,
}

/// The `d` confirmation on a background session.
#[derive(Debug, Clone)]
pub struct BackgroundRemoveState {
    pub session_id: String,
    pub label: String,
    /// Claude refused the first `rm`; `y` reruns it with these flags.
    pub refusal: Option<RmRefusal>,
    /// The `rm` is running.
    pub running: bool,
}

/// Hand the New Feature dialog's "run in the background" to the feature's
/// first Claude session that has no conversation yet (feature start). A
/// free function so the start loop can call it while it holds the feature.
pub(crate) fn claim_feature_background(
    runtime: &mut BackgroundRuntime,
    project_name: &str,
    feature: &mut Feature,
) {
    let key = (project_name.to_string(), feature.name.clone());
    if !runtime.pending_features.contains(&key) {
        return;
    }
    let taken: Vec<String> = feature
        .sessions
        .iter()
        .map(|s| s.tmux_window.clone())
        .collect();
    let Some(session) = feature.sessions.iter_mut().find(|s| {
        s.kind == SessionKind::Claude
            && !s.is_background()
            && super::session_ops::persisted_resume_id(s).is_none()
    }) else {
        return;
    };
    runtime.pending_features.remove(&key);
    session.tmux_window = bg::window_name(taken.iter().map(String::as_str));
    runtime.pending.entry(session.id.clone()).or_default();
}

impl App {
    // ── store ───────────────────────────────────────────────────────

    /// Load stored background sessions (startup).
    pub(crate) fn load_background_sessions(&mut self) {
        let Some(db) = self.db.as_ref() else {
            return;
        };
        match db.all_background_sessions() {
            Ok(rows) => {
                self.background.records = rows
                    .into_iter()
                    .map(|(id, row)| {
                        (
                            id,
                            BgRecord {
                                bg_id: row.bg_id,
                                conversation_id: row.conversation_id,
                            },
                        )
                    })
                    .collect();
            }
            Err(e) => self.log_warn("background", format!("could not load sessions: {e}")),
        }
    }

    fn persist_background(&mut self, session_id: &str) {
        let Some(record) = self.background.records.get(session_id).cloned() else {
            return;
        };
        if let Some(db) = self.db.as_ref()
            && let Err(e) = db.save_background_session(
                session_id,
                &BackgroundRow {
                    bg_id: record.bg_id,
                    conversation_id: record.conversation_id,
                },
            )
        {
            self.log_warn("background", format!("could not save a session: {e}"));
        }
    }

    /// A session is gone from the dashboard (removed, or its feature
    /// deleted). A session Claude still runs is stopped — its conversation
    /// stays, and `claude agents` keeps listing it — rather than left running
    /// with nothing in AIW pointing at it.
    pub(crate) fn forget_background(&mut self, session_id: &str) {
        self.background.pending.remove(session_id);
        self.background.entries.remove(session_id);
        self.background.attached.remove(session_id);
        self.background.logs.remove(session_id);
        self.background.errors.remove(session_id);
        let Some(record) = self.background.records.remove(session_id) else {
            return;
        };
        if let Some(db) = self.db.as_ref()
            && let Err(e) = db.delete_background_session(session_id)
        {
            self.log_warn("background", format!("could not delete a session: {e}"));
        }
        if let Some(id) = record.bg_id {
            let bin = self.background.claude_bin.clone();
            let _ = std::thread::Builder::new()
                .name("aiw-bg-stop".into())
                .spawn(move || {
                    let bin = bin.unwrap_or_else(crate::claude::ClaudeLauncher::resolve_binary);
                    let _ = bg::run(
                        &bin,
                        &bg::verb_args("stop", &id),
                        &std::env::temp_dir(),
                        &[],
                        bg::OP_TIMEOUT,
                    );
                });
        }
    }

    /// Whether any session in the store is a background session.
    pub fn has_background_sessions(&self) -> bool {
        self.store.projects.iter().any(|p| {
            p.features
                .iter()
                .any(|f| f.sessions.iter().any(|s| s.is_background()))
        })
    }

    /// The background session selected on the dashboard, if one is.
    pub fn selected_background_session(&self) -> Option<(usize, usize, usize)> {
        let Selection::Session(pi, fi, si) = self.selection else {
            return None;
        };
        let session = self
            .store
            .projects
            .get(pi)?
            .features
            .get(fi)?
            .sessions
            .get(si)?;
        session.is_background().then_some((pi, fi, si))
    }

    // ── state ───────────────────────────────────────────────────────

    /// What the session is doing.
    pub fn background_state(&self, session_id: &str) -> BgState {
        let rt = &self.background;
        if rt.pending.contains_key(session_id) || rt.is_running(session_id, Op::Launch) {
            return BgState::Starting;
        }
        if let Some(entry) = rt.entries.get(session_id) {
            return bg::state_of(entry);
        }
        if let Some(error) = rt.errors.get(session_id) {
            return BgState::Failed(error.clone());
        }
        let has_id = rt
            .records
            .get(session_id)
            .is_some_and(|r| r.bg_id.is_some());
        if has_id && !rt.roster_seen {
            // Not read yet since AIW started: say so rather than "gone".
            return BgState::Starting;
        }
        BgState::Gone
    }

    /// The short id of a background session, once `claude --bg` printed it.
    pub fn background_id(&self, session_id: &str) -> Option<&str> {
        self.background
            .records
            .get(session_id)
            .and_then(|r| r.bg_id.as_deref())
    }

    /// Whether the session's attach window is open.
    pub fn background_attached(&self, session_id: &str) -> bool {
        self.background.attached.contains(session_id)
    }

    /// The info block's line for a background session:
    /// `bg 7c5dcf5d · working · <last log line>`.
    pub fn background_info_line(&self, session_id: &str) -> String {
        let mut parts = vec![match self.background_id(session_id) {
            Some(id) => format!("bg {id}"),
            None => "bg".to_string(),
        }];
        parts.push(self.background_state(session_id).word());
        if self.background_attached(session_id) {
            parts.push("attached".into());
        }
        if let Some((_, lines)) = self.background.logs.get(session_id)
            && let Some(last) = lines.last()
        {
            parts.push(last.clone());
        }
        parts.join(" · ")
    }

    /// Fold a roster read into the background sessions: their entries, and
    /// conversation ids learned from it. Returns whether anything changed.
    pub(crate) fn background_apply_roster(&mut self, roster: &[RosterEntry]) -> bool {
        let sessions: Vec<(String, Option<String>)> = self
            .store
            .projects
            .iter()
            .flat_map(|p| p.features.iter())
            .flat_map(|f| f.sessions.iter())
            .filter(|s| s.is_background())
            .map(|s| (s.id.clone(), super::session_ops::persisted_resume_id(s)))
            .collect();
        let mut entries = HashMap::new();
        let mut learned = Vec::new();
        for (session_id, hook_conversation) in sessions {
            let record = self
                .background
                .records
                .get(&session_id)
                .cloned()
                .unwrap_or_default();
            let conversation = record.conversation_id.clone().or(hook_conversation);
            let name = self.session_identity.names.get(&session_id);
            let found = roster.iter().filter(|e| e.is_background()).find(|e| {
                (record.bg_id.is_some() && e.id == record.bg_id)
                    || (conversation.is_some() && e.session_id == conversation)
                    || (name.is_some() && e.name.as_deref() == name)
            });
            if let Some(entry) = found {
                if record.conversation_id.is_none()
                    && let Some(conv) = entry.session_id.clone()
                {
                    learned.push((session_id.clone(), conv));
                }
                if record.bg_id.is_none()
                    && let Some(id) = entry.id.clone()
                {
                    learned.push((session_id.clone(), format!("id:{id}")));
                }
                entries.insert(session_id, entry.clone());
            }
        }
        for (session_id, value) in learned {
            let record = self
                .background
                .records
                .entry(session_id.clone())
                .or_default();
            match value.strip_prefix("id:") {
                Some(id) => record.bg_id = Some(id.to_string()),
                None => record.conversation_id = Some(value),
            }
            self.persist_background(&session_id);
        }
        let changed = entries != self.background.entries || !self.background.roster_seen;
        self.background.entries = entries;
        self.background.roster_seen = true;
        changed
    }

    // ── create ──────────────────────────────────────────────────────

    /// The `s` picker's "Claude (background)" after its name and prompt:
    /// add the session and launch it on the next tick.
    pub fn create_background_session(
        &mut self,
        pi: usize,
        fi: usize,
        label: &str,
        prompt: &str,
    ) -> anyhow::Result<String> {
        let feature = self
            .store
            .projects
            .get_mut(pi)
            .and_then(|p| p.features.get_mut(fi))
            .ok_or_else(|| anyhow::anyhow!("that feature is gone"))?;
        let taken: Vec<String> = feature
            .sessions
            .iter()
            .map(|s| s.tmux_window.clone())
            .collect();
        let window = bg::window_name(taken.iter().map(String::as_str));
        let session = feature.add_session_named(SessionKind::Claude, label.to_string());
        session.tmux_window = window;
        let session_id = session.id.clone();
        self.background.pending.insert(
            session_id.clone(),
            PendingLaunch {
                prompt: Some(prompt.to_string()).filter(|p| !p.trim().is_empty()),
                resume: None,
            },
        );
        self.save()?;
        self.log_info(
            "background",
            format!("added background session '{label}' ({session_id})"),
        );
        Ok(session_id)
    }

    /// A feature start (`finish_feature_launch`) has a task prompt for a
    /// feature whose agent runs in the background: it becomes the
    /// `claude --bg` prompt. Returns whether it was taken.
    pub(crate) fn background_take_startup_prompt(
        &mut self,
        pi: usize,
        fi: usize,
        prompt: &str,
    ) -> bool {
        let Some(feature) = self.store.projects.get(pi).and_then(|p| p.features.get(fi)) else {
            return false;
        };
        let Some(session) = feature.sessions.iter().find(|s| s.is_background()) else {
            return false;
        };
        let Some(pending) = self.background.pending.get_mut(&session.id) else {
            return false;
        };
        pending.prompt = Some(prompt.to_string()).filter(|p| !p.trim().is_empty());
        true
    }

    /// Whether every tmux-backed session of the feature runs in the
    /// background (a start creates no tmux session for it).
    pub(crate) fn feature_is_background_only(feature: &Feature) -> bool {
        let mut any = false;
        for session in feature.sessions.iter().filter(|s| s.kind.is_tmux_backed()) {
            if !session.is_background() {
                return false;
            }
            any = true;
        }
        any
    }

    // ── tick ────────────────────────────────────────────────────────

    /// Main-loop step: launch what is pending, apply finished commands,
    /// read the roster, notice detaches, keep the selected log tail fresh.
    /// Returns whether anything visible changed.
    pub fn background_tick(&mut self) -> bool {
        let mut changed = self.poll_background_ops();
        if self.background.pending.is_empty()
            && self.background.inflight.is_empty()
            && !self.has_background_sessions()
        {
            return changed;
        }
        let pending: Vec<String> = self.background.pending.keys().cloned().collect();
        for session_id in pending {
            if !self.background.is_running(&session_id, Op::Launch) {
                changed |= self.start_background_launch(&session_id);
            }
        }
        changed |= self.poll_claude_roster_bg();
        self.claude_roster_tick();
        changed |= self.check_attach_windows();
        self.refresh_selected_background_logs();
        changed
    }

    fn channel(&mut self) -> Sender<OpDone> {
        self.background
            .channel
            .get_or_insert_with(std::sync::mpsc::channel)
            .0
            .clone()
    }

    /// Run `claude <args>` for `session_id` off the UI thread. `false` when
    /// a command of the same kind is already running for it.
    fn spawn_background_op(
        &mut self,
        session_id: &str,
        op: Op,
        args: Vec<String>,
        cwd: PathBuf,
        env: Vec<(String, String)>,
        timeout: Duration,
    ) -> bool {
        let key = (session_id.to_string(), op);
        if self.background.inflight.contains(&key) {
            return false;
        }
        let tx = self.channel();
        let bin = self.background.claude_bin.clone();
        let id = session_id.to_string();
        let spawned = std::thread::Builder::new()
            .name("aiw-bg-claude".into())
            .spawn(move || {
                let bin = bin.unwrap_or_else(crate::claude::ClaudeLauncher::resolve_binary);
                let result = bg::run(&bin, &args, &cwd, &env, timeout);
                let _ = tx.send(OpDone {
                    session_id: id,
                    op,
                    result,
                });
            });
        if spawned.is_err() {
            return false;
        }
        self.background.inflight.insert(key);
        true
    }

    /// The environment AIW's hook scripts need, for a session of `feature`.
    fn background_env(feature: &Feature, session_id: &str, window: &str) -> Vec<(String, String)> {
        let mut env = vec![
            ("AIW_ACTIVE".to_string(), "1".to_string()),
            ("AIW_SESSION".to_string(), feature.tmux_session.clone()),
            ("AIW_TMUX_SESSION".to_string(), feature.tmux_session.clone()),
            ("AIW_TMUX_WINDOW".to_string(), window.to_string()),
            ("AIW_FEATURE_SESSION_ID".to_string(), session_id.to_string()),
        ];
        let (tmux_bin, socket) = crate::tmux::TmuxManager::runtime_binary_and_socket();
        env.push((
            "AIW_TMUX_BIN".to_string(),
            tmux_bin.to_string_lossy().into_owned(),
        ));
        if let Some(socket) = socket {
            env.push((
                "AIW_TMUX_SOCKET".to_string(),
                socket.to_string_lossy().into_owned(),
            ));
        }
        if let Some(aiw) = crate::tmux::TmuxManager::cli_binary_path() {
            env.push(("AIW_BIN".to_string(), aiw.to_string_lossy().into_owned()));
        }
        // The hooks find AIW's socket under its state dir; a supervisor
        // started from another terminal may not share these.
        for key in ["XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_DATA_HOME"] {
            if let Ok(value) = std::env::var(key)
                && !value.is_empty()
            {
                env.push((key.to_string(), value));
            }
        }
        env.extend(crate::fork::ports::env_for_tmux_session(
            &feature.tmux_session,
        ));
        env
    }

    /// Start `claude --bg` (or `--resume <id> --bg`) for a pending session.
    fn start_background_launch(&mut self, session_id: &str) -> bool {
        let Some((pi, fi, si)) = self.session_indices_by_id(session_id) else {
            self.background.pending.remove(session_id);
            return false;
        };
        let Some(pending) = self.background.pending.get(session_id).cloned() else {
            return false;
        };
        // A task prompt the fan-out path queued for this session goes on the
        // command line instead of being pasted into a pane it never has.
        let queued = self.pending_prompt_for(session_id).map(|p| p.text.clone());
        if queued.is_some() {
            self.forget_fan_out_session(session_id);
        }
        let project_name = self.store.projects[pi].name.clone();
        let repo = self.store.projects[pi].repo.clone();
        let feature = self.store.projects[pi].features[fi].clone();
        let session = feature.sessions[si].clone();
        crate::fork::ports::prepare_feature_session(&feature.id, &feature.tmux_session);
        let env = Self::background_env(&feature, session_id, &session.tmux_window);
        let args = match &pending.resume {
            Some(conversation) => bg::resume_args(conversation),
            None => {
                let name =
                    self.session_identity
                        .names
                        .assign(&session.id, &feature.name, &session.label);
                let ctx = crate::fork::launch::LaunchContext::for_feature(
                    &project_name,
                    &repo,
                    &feature,
                    &session.id,
                )
                .with_session_name(Some(name));
                let flags = feature.mode.cli_flags(LaunchOpts {
                    enable_chrome: feature.enable_chrome,
                });
                let extra = crate::fork::launch::claude_launch_args(&ctx, &self.config.aiw, flags);
                if let Some(path) = bg::settings_path(&extra)
                    && let Err(e) =
                        bg::amend_settings_file(std::path::Path::new(path), &bg::settings_env(&env))
                {
                    self.log_warn(
                        "background",
                        format!("could not add the hook environment to {path}: {e}"),
                    );
                }
                let prompt = pending.prompt.clone().or(queued);
                bg::launch_args(extra, prompt.as_deref())
            }
        };
        self.log_info(
            "background",
            format!("{}: claude {}", session.label, args.join(" ")),
        );
        self.background.errors.remove(session_id);
        if self.spawn_background_op(
            session_id,
            Op::Launch,
            args,
            feature.workdir.clone(),
            env,
            bg::LAUNCH_TIMEOUT,
        ) {
            self.background.pending.remove(session_id);
            true
        } else {
            false
        }
    }

    fn poll_background_ops(&mut self) -> bool {
        let mut done = Vec::new();
        if let Some((_, rx)) = self.background.channel.as_ref() {
            while let Ok(item) = rx.try_recv() {
                done.push(item);
            }
        }
        let changed = !done.is_empty();
        for item in done {
            self.background
                .inflight
                .remove(&(item.session_id.clone(), item.op));
            self.apply_background_op(item);
        }
        changed
    }

    fn session_label(&self, session_id: &str) -> String {
        self.session_indices_by_id(session_id)
            .map(|(pi, fi, si)| {
                self.store.projects[pi].features[fi].sessions[si]
                    .label
                    .clone()
            })
            .unwrap_or_else(|| "background session".into())
    }

    fn apply_background_op(&mut self, item: OpDone) {
        let label = self.session_label(&item.session_id);
        match (item.op, item.result) {
            (Op::Launch, Ok(out)) => match bg::parse_bg_output(&out.combined()) {
                Some(id) if out.success => {
                    let record = self
                        .background
                        .records
                        .entry(item.session_id.clone())
                        .or_default();
                    record.bg_id = Some(id.clone());
                    self.persist_background(&item.session_id);
                    self.push_toast_success(format!("'{label}' runs in the background ({id})"));
                    self.log_info("background", format!("{label}: backgrounded {id}"));
                }
                _ => {
                    let why = match out.last_line() {
                        line if line.is_empty() => "claude --bg printed no session id".to_string(),
                        line => line,
                    };
                    self.background_failed(&item.session_id, &label, "start", why);
                }
            },
            (Op::Launch, Err(e)) => self.background_failed(&item.session_id, &label, "start", e),
            (Op::Stop, Ok(out)) if out.success => {
                self.push_toast_info(format!("Stopped '{label}' (its conversation is kept)"));
            }
            (Op::Respawn, Ok(out)) if out.success => {
                self.push_toast_success(format!("Restarted '{label}' in the background"));
            }
            (op @ (Op::Stop | Op::Respawn), Ok(out)) => {
                let verb = if op == Op::Stop { "stop" } else { "restart" };
                self.background_failed(&item.session_id, &label, verb, out.last_line());
            }
            (op @ (Op::Stop | Op::Respawn), Err(e)) => {
                let verb = if op == Op::Stop { "stop" } else { "restart" };
                self.background_failed(&item.session_id, &label, verb, e);
            }
            (Op::Logs, Ok(out)) => {
                let lines = bg::log_tail(&out.combined(), bg::LOG_LINES);
                self.background
                    .logs
                    .insert(item.session_id, (Instant::now(), lines));
            }
            (Op::Logs, Err(_)) => {
                // Keep the old tail; try again at the next refresh.
                self.background
                    .logs
                    .entry(item.session_id)
                    .and_modify(|(at, _)| *at = Instant::now());
            }
            (Op::Remove, result) => self.background_remove_done(&item.session_id, &label, result),
        }
    }

    fn background_failed(&mut self, session_id: &str, label: &str, verb: &str, why: String) {
        self.background
            .errors
            .insert(session_id.to_string(), why.clone());
        self.report_logged_error(
            "background",
            format!("Could not {verb} '{label}' in the background: {why}"),
        );
    }

    /// Close the attach state of sessions whose window has gone (detached).
    fn check_attach_windows(&mut self) -> bool {
        if self.background.attached.is_empty()
            || self
                .background
                .last_window_check
                .is_some_and(|at| at.elapsed() < WINDOW_CHECK)
        {
            return false;
        }
        self.background.last_window_check = Some(Instant::now());
        let attached: Vec<String> = self.background.attached.iter().cloned().collect();
        let mut changed = false;
        for session_id in attached {
            let window = self.session_indices_by_id(&session_id).map(|(pi, fi, si)| {
                let feature = &self.store.projects[pi].features[fi];
                (
                    feature.tmux_session.clone(),
                    feature.sessions[si].tmux_window.clone(),
                )
            });
            let open =
                window.is_some_and(|(session, window)| self.tmux.window_exists(&session, &window));
            let just_opened = self
                .recent_launches
                .get(&session_id)
                .is_some_and(|at| at.elapsed() < Duration::from_secs(3));
            if !open && !just_opened {
                self.background.attached.remove(&session_id);
                changed = true;
            }
        }
        changed
    }

    fn refresh_selected_background_logs(&mut self) {
        let Some((pi, fi, si)) = self.selected_background_session() else {
            return;
        };
        let session_id = self.store.projects[pi].features[fi].sessions[si].id.clone();
        let Some(id) = self.background_id(&session_id).map(str::to_string) else {
            return;
        };
        if self
            .background
            .logs
            .get(&session_id)
            .is_some_and(|(at, _)| at.elapsed() < LOG_REFRESH)
        {
            return;
        }
        let cwd = self.store.projects[pi].features[fi].workdir.clone();
        if self.spawn_background_op(
            &session_id,
            Op::Logs,
            bg::verb_args("logs", &id),
            cwd,
            Vec::new(),
            bg::OP_TIMEOUT,
        ) {
            // Mark the attempt so a failing `logs` is not retried every turn.
            self.background
                .logs
                .entry(session_id)
                .or_insert_with(|| (Instant::now(), Vec::new()))
                .0 = Instant::now();
        }
    }

    // ── actions ─────────────────────────────────────────────────────

    /// `Enter` on a background row: open `claude attach <id>` in a window of
    /// the feature's tmux session and focus it in the agent pane.
    pub fn attach_background_session(
        &mut self,
        pi: usize,
        fi: usize,
        si: usize,
    ) -> anyhow::Result<()> {
        let feature = &self.store.projects[pi].features[fi];
        let session = &feature.sessions[si];
        let session_id = session.id.clone();
        let label = session.label.clone();
        if self.background_attached(&session_id) {
            self.workspace_commit_now();
            self.cockpit_focus_agent();
            return Ok(());
        }
        let Some(id) = self.background_id(&session_id).map(str::to_string) else {
            self.push_toast_info(match self.background_state(&session_id) {
                BgState::Starting => format!("'{label}' is still starting"),
                _ => format!("'{label}' has not started — c starts it"),
            });
            return Ok(());
        };
        if self.background_state(&session_id) == BgState::Gone {
            self.push_toast_info(format!(
                "Claude no longer lists '{label}' — c resumes its conversation in the background"
            ));
            return Ok(());
        }
        let tmux_session = feature.tmux_session.clone();
        let window = session.tmux_window.clone();
        let workdir = feature.workdir.clone();
        let feature_id = feature.id.clone();
        if !self.tmux.session_exists(&tmux_session) {
            crate::fork::ports::prepare_feature_session(&feature_id, &tmux_session);
            self.tmux
                .create_session_with_window(&tmux_session, &window, &workdir)?;
            self.tmux
                .set_session_env(&tmux_session, "AIW_SESSION", &tmux_session)?;
        } else if !self.tmux.window_exists(&tmux_session, &window) {
            self.tmux.create_window(&tmux_session, &window, &workdir)?;
        }
        let (tmux_bin, socket) = crate::tmux::TmuxManager::runtime_binary_and_socket();
        let claude = self
            .background
            .claude_bin
            .clone()
            .unwrap_or_else(crate::claude::ClaudeLauncher::resolve_binary);
        let command = bg::attach_shell_command(
            &claude,
            &id,
            &tmux_bin.to_string_lossy(),
            socket.as_deref(),
            &tmux_session,
            &window,
        );
        self.tmux
            .run_shell_command(&tmux_session, &window, &command)?;
        self.recent_launches
            .insert(session_id.clone(), Instant::now());
        self.background.attached.insert(session_id);
        if let Some(feature) = self
            .store
            .projects
            .get_mut(pi)
            .and_then(|p| p.features.get_mut(fi))
            && feature.status == crate::project::ProjectStatus::Stopped
        {
            feature.status = crate::project::ProjectStatus::Idle;
        }
        self.workspace_commit_now();
        if self.cockpit_active() {
            self.cockpit_focus_agent();
        }
        self.message = Some(format!(
            "Attached to '{label}' ({id}) — detach in Claude to send it back to the background"
        ));
        Ok(())
    }

    /// `x` on a background row: `claude stop <id>`.
    pub fn stop_background_session(&mut self, pi: usize, fi: usize, si: usize) {
        let feature = &self.store.projects[pi].features[fi];
        let session = &feature.sessions[si];
        let (session_id, label) = (session.id.clone(), session.label.clone());
        let Some(id) = self.background_id(&session_id).map(str::to_string) else {
            self.background.pending.remove(&session_id);
            self.push_toast_info(format!("'{label}' was not running"));
            return;
        };
        let cwd = feature.workdir.clone();
        if self.spawn_background_op(
            &session_id,
            Op::Stop,
            bg::verb_args("stop", &id),
            cwd,
            Vec::new(),
            bg::OP_TIMEOUT,
        ) {
            self.message = Some(format!("Stopping '{label}' …"));
        }
    }

    /// `c` on a background row: `claude respawn <id>` (running or stopped,
    /// conversation intact); `--resume <conversation> --bg` when Claude no
    /// longer lists it; a fresh `--bg` when it never started.
    pub fn restart_background_session(&mut self, pi: usize, fi: usize, si: usize) {
        let feature = &self.store.projects[pi].features[fi];
        let session = &feature.sessions[si];
        let session_id = session.id.clone();
        let label = session.label.clone();
        let cwd = feature.workdir.clone();
        let conversation = self
            .background
            .records
            .get(&session_id)
            .and_then(|r| r.conversation_id.clone())
            .or_else(|| super::session_ops::persisted_resume_id(session));
        let state = self.background_state(&session_id);
        if state == BgState::Starting {
            self.push_toast_info(format!("'{label}' is starting"));
            return;
        }
        self.background.errors.remove(&session_id);
        match (self.background_id(&session_id).map(str::to_string), state) {
            (Some(id), state) if state != BgState::Gone => {
                if self.spawn_background_op(
                    &session_id,
                    Op::Respawn,
                    bg::verb_args("respawn", &id),
                    cwd,
                    Vec::new(),
                    bg::OP_TIMEOUT,
                ) {
                    self.message = Some(format!("Restarting '{label}' …"));
                }
            }
            _ => {
                self.background.pending.insert(
                    session_id,
                    PendingLaunch {
                        prompt: None,
                        resume: conversation,
                    },
                );
                self.message = Some(format!("Starting '{label}' in the background …"));
            }
        }
    }

    /// `d` on a background row: ask before `claude rm`.
    pub fn open_background_remove(&mut self, pi: usize, fi: usize, si: usize) {
        let session = &self.store.projects[pi].features[fi].sessions[si];
        self.mode = AppMode::BackgroundRemove(BackgroundRemoveState {
            session_id: session.id.clone(),
            label: session.label.clone(),
            refusal: None,
            running: false,
        });
    }

    /// `y` in the remove dialog: `claude rm <id>` (with the refusal's flags
    /// on the second confirmation), or drop a session Claude never ran.
    pub fn confirm_background_remove(&mut self) -> anyhow::Result<()> {
        let AppMode::BackgroundRemove(state) = &self.mode else {
            return Ok(());
        };
        if state.running {
            return Ok(());
        }
        let state = state.clone();
        let Some((pi, fi, _)) = self.session_indices_by_id(&state.session_id) else {
            self.mode = AppMode::Normal;
            return Ok(());
        };
        let Some(id) = self.background_id(&state.session_id).map(str::to_string) else {
            self.mode = AppMode::Normal;
            return self.remove_background_row(&state.session_id, &state.label);
        };
        let flags = state
            .refusal
            .as_ref()
            .map(|r| r.flags.clone())
            .unwrap_or_default();
        let cwd = self.store.projects[pi].features[fi].workdir.clone();
        if self.spawn_background_op(
            &state.session_id,
            Op::Remove,
            bg::rm_args(&id, &flags),
            cwd,
            Vec::new(),
            bg::OP_TIMEOUT,
        ) && let AppMode::BackgroundRemove(state) = &mut self.mode
        {
            state.running = true;
        }
        Ok(())
    }

    fn background_remove_done(
        &mut self,
        session_id: &str,
        label: &str,
        result: Result<RunOutput, String>,
    ) {
        let out = match result {
            Ok(out) => out,
            Err(e) => {
                self.close_background_remove(session_id);
                self.report_logged_error("background", format!("Could not remove '{label}': {e}"));
                return;
            }
        };
        if out.success {
            self.close_background_remove(session_id);
            if let Err(e) = self.remove_background_row(session_id, label) {
                self.report_logged_error(
                    "background",
                    format!("Removed '{label}' from Claude but not from AIW: {e}"),
                );
            }
            return;
        }
        match bg::parse_rm_refusal(&out.combined()) {
            Some(refusal) if !self.background_remove_already_forced(session_id) => {
                if let AppMode::BackgroundRemove(state) = &mut self.mode
                    && state.session_id == session_id
                {
                    state.refusal = Some(refusal);
                    state.running = false;
                } else {
                    self.report_logged_error(
                        "background",
                        format!("Claude refused to remove '{label}': {}", refusal.text),
                    );
                }
            }
            _ => {
                self.close_background_remove(session_id);
                self.report_logged_error(
                    "background",
                    format!("Could not remove '{label}': {}", out.last_line()),
                );
            }
        }
    }

    fn background_remove_already_forced(&self, session_id: &str) -> bool {
        matches!(&self.mode, AppMode::BackgroundRemove(state)
            if state.session_id == session_id && state.refusal.is_some())
    }

    fn close_background_remove(&mut self, session_id: &str) {
        if matches!(&self.mode, AppMode::BackgroundRemove(state) if state.session_id == session_id)
        {
            self.mode = AppMode::Normal;
        }
    }

    /// Drop the row (Claude has already let go of the session).
    fn remove_background_row(&mut self, session_id: &str, label: &str) -> anyhow::Result<()> {
        // Nothing left to stop.
        if let Some(record) = self.background.records.get_mut(session_id) {
            record.bg_id = None;
        }
        let Some((pi, fi, si)) = self.session_indices_by_id(session_id) else {
            return Ok(());
        };
        self.selection = Selection::Session(pi, fi, si);
        self.remove_session()?;
        self.push_toast_success(format!("Removed '{label}'"));
        Ok(())
    }

    // ── picker ──────────────────────────────────────────────────────

    /// The prompt step after naming a background session.
    pub fn open_background_prompt(&mut self, pi: usize, fi: usize, label: String) {
        self.mode = AppMode::BackgroundPrompt(BackgroundPromptState {
            pi,
            fi,
            label,
            text: String::new(),
        });
    }

    /// `Enter` in the prompt step.
    pub fn submit_background_prompt(&mut self) {
        let AppMode::BackgroundPrompt(state) = &self.mode else {
            return;
        };
        let state = state.clone();
        self.mode = AppMode::Normal;
        match self.create_background_session(state.pi, state.fi, &state.label, &state.text) {
            Ok(session_id) => {
                if let Some((pi, fi, si)) = self.session_indices_by_id(&session_id) {
                    self.selection = Selection::Session(pi, fi, si);
                }
                self.message = Some(format!("Starting '{}' in the background …", state.label));
            }
            Err(e) => self.push_toast_error(format!("Error: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, state: &str) -> RosterEntry {
        RosterEntry {
            cwd: "/w".into(),
            kind: "background".into(),
            id: Some(id.into()),
            state: Some(state.into()),
            session_id: Some(format!("conv-{id}")),
            ..Default::default()
        }
    }

    #[test]
    fn a_dialog_choice_claims_the_first_claude_session() {
        let mut feature = Feature::new_for_project(
            "RS",
            "feat".into(),
            "feat".into(),
            "/tmp/feat".into(),
            false,
            crate::project::VibeMode::Vibe,
            crate::project::AgentKind::Claude,
            false,
        );
        feature.add_session(SessionKind::Terminal);
        feature.add_session(SessionKind::Claude);
        let mut rt = BackgroundRuntime::default();
        claim_feature_background(&mut rt, "RS", &mut feature);
        assert!(
            feature.sessions.iter().all(|s| !s.is_background()),
            "no choice, no claim"
        );

        rt.pending_features.insert(("RS".into(), "feat".into()));
        claim_feature_background(&mut rt, "RS", &mut feature);
        let claude = feature
            .sessions
            .iter()
            .find(|s| s.kind == SessionKind::Claude)
            .unwrap();
        assert!(claude.is_background());
        assert_eq!(claude.tmux_window, "aiw-bg-claude");
        assert!(!claude.runs_with_feature());
        assert!(rt.pending.contains_key(&claude.id));
        assert!(rt.pending_features.is_empty());
        assert!(
            !App::feature_is_background_only(&feature),
            "the terminal still runs"
        );
        feature.sessions.retain(|s| s.kind == SessionKind::Claude);
        assert!(App::feature_is_background_only(&feature));
    }

    #[test]
    fn states_come_from_pending_roster_and_errors() {
        let mut app = crate::app::fork_workspace::workspace_app(&[SessionKind::Claude]);
        app.background
            .pending
            .insert("s".into(), PendingLaunch::default());
        assert_eq!(app.background_state("s"), BgState::Starting);
        app.background.pending.clear();
        app.background.records.insert(
            "s".into(),
            BgRecord {
                bg_id: Some("7c5d".into()),
                conversation_id: None,
            },
        );
        assert_eq!(
            app.background_state("s"),
            BgState::Starting,
            "roster not read yet"
        );
        app.background.roster_seen = true;
        assert_eq!(app.background_state("s"), BgState::Gone);
        app.background
            .entries
            .insert("s".into(), entry("7c5d", "working"));
        assert_eq!(app.background_state("s"), BgState::Working);
        assert_eq!(app.background_info_line("s"), "bg 7c5d · working");
        app.background.entries.clear();
        app.background
            .errors
            .insert("s".into(), "not trusted".into());
        assert_eq!(
            app.background_state("s"),
            BgState::Failed("not trusted".into())
        );
    }
}
