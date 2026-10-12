//! FORK: session names, conversation forks and Claude's own session roster
//! on `App` (DESIGN.md §19.2). The pure pieces live in
//! `fork::session_identity` and `fork::claude_roster`.
//!
//! - Names: assigned on a Claude session's first launch, persisted in
//!   `aiw_session_names`, flushed on the status sweep.
//! - Forks: `f` on a Claude or Codex row opens a two-choice dialog — a new
//!   window in the same feature, or a new worktree through `F`'s fork flow —
//!   and the chosen session's first launch starts from the source
//!   conversation.
//! - Roster: `claude agents --json` read off the UI thread every
//!   [`ROSTER_INTERVAL`]; external sessions in a project's checkouts are drawn
//!   as dimmed lines and listed by `E`; a matched AIW session that waits gets
//!   a needs-you badge even when its hooks said nothing.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use super::{App, AppMode, Selection};
use crate::fork::claude_roster::{self, RosterEntry};
use crate::fork::session_identity::{ForkSource, SessionNames, fork_label};
use crate::project::{AgentKind, SessionKind};

/// How often the roster is read.
pub const ROSTER_INTERVAL: Duration = Duration::from_secs(10);

/// At most this many external lines under one sidebar row; the rest are a
/// `+N more (E)` line.
pub const MAX_EXTERNAL_LINES: usize = 2;

/// How many sidebar lines `count` external sessions take.
pub fn external_line_count(count: usize) -> usize {
    if count > MAX_EXTERNAL_LINES {
        MAX_EXTERNAL_LINES + 1
    } else {
        count
    }
}

/// A Claude session from the roster that is not one of AIW's, in one of a
/// project's checkouts.
#[derive(Debug, Clone, PartialEq)]
pub struct ExternalSession {
    pub project_name: String,
    /// The feature whose checkout it runs in, when it runs in one.
    pub feature_id: Option<String>,
    pub entry: RosterEntry,
}

/// The `f` dialog: where the fork of one session goes.
#[derive(Debug, Clone)]
pub struct ForkSessionState {
    pub pi: usize,
    pub fi: usize,
    pub source: ForkSource,
    /// 0: a new window in the same feature; 1: a new feature worktree.
    pub choice: usize,
}

/// The `E` dialog: external Claude sessions.
#[derive(Debug, Clone, Default)]
pub struct ExternalSessionsState {
    pub selected: usize,
}

/// Hand a pending feature-level fork to the feature's first agent session
/// of the source's harness that has no conversation yet (feature start).
/// A free function so the start loop can call it while it holds the feature.
pub(crate) fn claim_feature_fork(
    identity: &mut IdentityRuntime,
    project_name: &str,
    feature: &crate::project::Feature,
) {
    let key = (project_name.to_string(), feature.name.clone());
    let Some(source) = identity.pending_feature_forks.get(&key) else {
        return;
    };
    let want = match source.agent {
        AgentKind::Claude => SessionKind::Claude,
        AgentKind::Codex => SessionKind::Codex,
        _ => return,
    };
    let Some(session_id) = feature
        .sessions
        .iter()
        .find(|s| s.kind == want && super::session_ops::persisted_resume_id(s).is_none())
        .map(|s| s.id.clone())
    else {
        return;
    };
    if let Some(source) = identity.pending_feature_forks.remove(&key) {
        identity.pending_forks.insert(session_id, source);
    }
}

/// One finished roster read: the binary used and what it printed.
type RosterRead = (String, Result<Vec<RosterEntry>, String>);

/// Runtime state on `App`.
#[derive(Default)]
pub struct IdentityRuntime {
    pub names: SessionNames,
    /// Fork sources for sessions that have not launched yet, by feature
    /// session id.
    pub pending_forks: HashMap<String, ForkSource>,
    /// Fork sources for features that have no session yet (the new-worktree
    /// fork), by (project name, feature name); handed to the feature's first
    /// agent session when it starts.
    pub pending_feature_forks: HashMap<(String, String), ForkSource>,
    /// The last roster read.
    pub roster: Vec<RosterEntry>,
    /// External sessions derived from the roster, recomputed when it is read.
    pub externals: Vec<ExternalSession>,
    /// What an AIW session waits for according to the roster, by feature
    /// session id.
    pub roster_waiting: HashMap<String, String>,
    roster_bg: Option<Receiver<RosterRead>>,
    /// The Claude binary the last successful read used; resolving it runs
    /// `--version` on every installed build, so it is kept between reads.
    roster_bin: Option<String>,
    roster_last: Option<Instant>,
    roster_error_logged: bool,
}

impl App {
    /// Load stored names (startup).
    pub(crate) fn load_session_names(&mut self) {
        if let Some(db) = self.db.as_ref() {
            match db.all_session_names() {
                Ok(names) => self.session_identity.names = SessionNames::from_map(names),
                Err(e) => self.log_warn("identity", format!("could not load session names: {e}")),
            }
        }
    }

    /// Write names assigned since the last flush. A failed write keeps them
    /// for the next flush.
    pub(crate) fn persist_session_names(&mut self) {
        let unsaved = self.session_identity.names.take_unsaved();
        if unsaved.is_empty() {
            return;
        }
        let Some(db) = self.db.as_ref() else {
            return;
        };
        if let Err(e) = db.save_session_names(&unsaved) {
            self.session_identity.names.restore_unsaved(unsaved);
            self.log_warn("identity", format!("could not save session names: {e}"));
        }
    }

    /// Forget a deleted session's name and any pending fork.
    pub(crate) fn forget_session_identity(&mut self, session_id: &str) {
        self.forget_inbox(session_id); // §19.5
        self.session_identity.pending_forks.remove(session_id);
        self.session_identity.roster_waiting.remove(session_id);
        if self.session_identity.names.forget(session_id)
            && let Some(db) = self.db.as_ref()
            && let Err(e) = db.delete_session_name(session_id)
        {
            self.log_warn("identity", format!("could not delete a session name: {e}"));
        }
    }

    /// A feature was deleted: forget its sessions' names and forks.
    pub(crate) fn forget_feature_identities(&mut self, feature: &crate::project::Feature) {
        for session in &feature.sessions {
            self.forget_session_identity(&session.id);
        }
    }

    /// Give the Claude session `session_id` its name if it has none yet.
    pub(crate) fn assign_session_name(&mut self, session_id: &str) -> Option<String> {
        let (feature_name, label) = self.store.projects.iter().find_map(|p| {
            p.features.iter().find_map(|f| {
                f.sessions
                    .iter()
                    .find(|s| s.id == session_id && s.kind == SessionKind::Claude)
                    .map(|s| (f.name.clone(), s.label.clone()))
            })
        })?;
        Some(
            self.session_identity
                .names
                .assign(session_id, &feature_name, &label),
        )
    }

    /// The session's harness name, if it has one.
    pub fn session_harness_name(&self, session_id: &str) -> Option<&str> {
        self.session_identity.names.get(session_id)
    }

    /// The conversation a fork of session (pi, fi, si) would start from.
    pub(crate) fn fork_source_for(&self, pi: usize, fi: usize, si: usize) -> Option<ForkSource> {
        let session = self
            .store
            .projects
            .get(pi)?
            .features
            .get(fi)?
            .sessions
            .get(si)?;
        let agent = match session.kind {
            SessionKind::Claude => AgentKind::Claude,
            SessionKind::Codex => AgentKind::Codex,
            _ => return None,
        };
        let id = super::session_ops::persisted_resume_id(session)?;
        Some(ForkSource {
            agent,
            harness_session_id: id,
            source_label: session.label.clone(),
        })
    }

    /// `f`: open the fork dialog for the selected session.
    pub fn start_fork_session(&mut self) {
        let Selection::Session(pi, fi, si) = self.selection else {
            self.push_toast_info("Select a Claude or Codex session to fork its conversation");
            return;
        };
        let Some(kind) = self
            .store
            .projects
            .get(pi)
            .and_then(|p| p.features.get(fi))
            .and_then(|f| f.sessions.get(si))
            .map(|s| s.kind.clone())
        else {
            return;
        };
        if !matches!(kind, SessionKind::Claude | SessionKind::Codex) {
            self.push_toast_info("Only Claude and Codex conversations can be forked");
            return;
        }
        let Some(source) = self.fork_source_for(pi, fi, si) else {
            self.push_toast_warning("No saved conversation to fork yet — send it a prompt first");
            return;
        };
        self.mode = AppMode::ForkSession(ForkSessionState {
            pi,
            fi,
            source,
            choice: 0,
        });
    }

    /// Enter in the fork dialog.
    pub fn confirm_fork_session(&mut self) -> anyhow::Result<()> {
        let AppMode::ForkSession(state) = std::mem::replace(&mut self.mode, AppMode::Normal) else {
            return Ok(());
        };
        if state.choice == 1 {
            // A new worktree: `F`'s flow, carrying the conversation along.
            self.selection = Selection::Feature(state.pi, state.fi);
            self.start_fork_feature();
            if let AppMode::ForkingFeature(fork) = &mut self.mode {
                fork.agent = state.source.agent.clone();
                if let Some(index) = self
                    .active_extension
                    .allowed_agents()
                    .iter()
                    .position(|a| *a == state.source.agent)
                {
                    fork.agent_index = index;
                }
                // The forked conversation already carries the context.
                fork.include_context = false;
                fork.fork_conversation = Some(state.source);
            }
            return Ok(());
        }
        self.fork_session_here(state.pi, state.fi, state.source)
    }

    /// A new window in the same feature, started from `source`.
    pub(crate) fn fork_session_here(
        &mut self,
        pi: usize,
        fi: usize,
        source: ForkSource,
    ) -> anyhow::Result<()> {
        let Some(feature) = self.store.projects.get(pi).and_then(|p| p.features.get(fi)) else {
            return Ok(());
        };
        let existing: Vec<&str> = feature.sessions.iter().map(|s| s.label.as_str()).collect();
        let label = fork_label(&source.source_label, &existing);
        let agent = source.agent.clone();
        let source_label = source.source_label.clone();
        match self.create_agent_session_with_fork(
            pi,
            fi,
            &label,
            Some(agent),
            super::StartIntent::Approved,
            Some(source),
        ) {
            Ok((si, _, _)) => {
                self.selection = Selection::Session(pi, fi, si);
                self.push_toast_success(format!("Forked '{source_label}' into '{label}'"));
                Ok(())
            }
            Err(e) => {
                self.push_toast_warning(format!("Fork failed: {e}"));
                Err(e)
            }
        }
    }

    // ── roster ──────────────────────────────────────────────────────────

    /// Status-sweep step: start a roster read when one is due.
    pub fn claude_roster_tick(&mut self) {
        self.persist_session_names();
        if !self.config.aiw.claude_agents_roster || self.session_identity.roster_bg.is_some() {
            return;
        }
        if self
            .session_identity
            .roster_last
            .is_some_and(|at| at.elapsed() < ROSTER_INTERVAL)
        {
            return;
        }
        if self.store.projects.is_empty() {
            return;
        }
        self.session_identity.roster_last = Some(Instant::now());
        let (tx, rx) = std::sync::mpsc::channel();
        self.session_identity.roster_bg = Some(rx);
        let cached = self.session_identity.roster_bin.clone();
        let spawned = std::thread::Builder::new()
            .name("aiw-claude-roster".into())
            .spawn(move || {
                let bin = cached.unwrap_or_else(crate::claude::ClaudeLauncher::resolve_binary);
                let result = claude_roster::fetch(&bin, claude_roster::FETCH_TIMEOUT);
                let _ = tx.send((bin, result));
            });
        if spawned.is_err() {
            self.session_identity.roster_bg = None;
        }
    }

    /// Apply a finished roster read. Returns whether anything visible
    /// changed.
    pub fn poll_claude_roster_bg(&mut self) -> bool {
        let Some(rx) = self.session_identity.roster_bg.as_ref() else {
            return false;
        };
        let (bin, result) = match rx.try_recv() {
            Ok(received) => received,
            Err(std::sync::mpsc::TryRecvError::Empty) => return false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.session_identity.roster_bg = None;
                return false;
            }
        };
        self.session_identity.roster_bg = None;
        match result {
            Ok(roster) => {
                self.session_identity.roster_error_logged = false;
                self.session_identity.roster_bin = Some(bin);
                self.apply_claude_roster(roster)
            }
            Err(e) => {
                // Resolve the binary again next time (an update may have
                // replaced it).
                self.session_identity.roster_bin = None;
                if !self.session_identity.roster_error_logged {
                    self.session_identity.roster_error_logged = true;
                    self.log_debug("roster", e);
                }
                false
            }
        }
    }

    /// Fold a roster into external sessions and waiting signals.
    pub(crate) fn apply_claude_roster(&mut self, roster: Vec<RosterEntry>) -> bool {
        // AIW's own Claude sessions: by conversation id and by name.
        let mut by_conversation: HashMap<String, String> = HashMap::new();
        for project in &self.store.projects {
            for feature in &project.features {
                for session in &feature.sessions {
                    if session.kind != SessionKind::Claude {
                        continue;
                    }
                    if let Some(id) = super::session_ops::persisted_resume_id(session) {
                        by_conversation.insert(id, session.id.clone());
                    }
                }
            }
        }
        let names = &self.session_identity.names;
        let checkouts: Vec<(String, Option<String>, PathBuf)> = self
            .store
            .projects
            .iter()
            .flat_map(|p| {
                let features = p
                    .features
                    .iter()
                    .map(|f| (p.name.clone(), Some(f.id.clone()), f.workdir.clone()));
                std::iter::once((p.name.clone(), None, p.repo.clone())).chain(features)
            })
            .collect();

        let mut waiting = HashMap::new();
        let mut externals = Vec::new();
        for entry in &roster {
            let own = entry
                .session_id
                .as_ref()
                .and_then(|id| by_conversation.get(id).cloned())
                .or_else(|| {
                    entry
                        .name
                        .as_deref()
                        .and_then(|n| names.session_for_name(n))
                        .map(str::to_string)
                });
            if let Some(session_id) = own {
                if let Some(detail) = entry.waiting_detail() {
                    waiting.insert(session_id, detail);
                }
                continue;
            }
            let cwd = PathBuf::from(&entry.cwd);
            // The most specific checkout wins: a feature's worktree over the
            // project's repo, and a feature in the repo dir over the repo.
            let best = checkouts
                .iter()
                .filter(|(_, _, root)| claude_roster::path_within(&cwd, root))
                .max_by_key(|(_, feature, root)| (root.components().count(), feature.is_some()));
            if let Some((project_name, feature_id, _)) = best {
                externals.push(ExternalSession {
                    project_name: project_name.clone(),
                    feature_id: feature_id.clone(),
                    entry: entry.clone(),
                });
            }
        }
        let changed = externals != self.session_identity.externals
            || waiting != self.session_identity.roster_waiting;
        self.session_identity.roster = roster;
        self.session_identity.externals = externals;
        self.session_identity.roster_waiting = waiting;
        changed
    }

    /// A hook message arrived for the session. One that shows the agent
    /// running again (a tool, a new turn, a finished turn) clears the
    /// roster's waiting signal at once, instead of leaving "needs you" on
    /// the row until the next roster read.
    pub(crate) fn roster_saw_hook_activity(&mut self, feature_session_id: &str, msg_type: &str) {
        if matches!(
            msg_type,
            "clear" | "tool-start" | "tool-stop" | "thinking-start" | "thinking-stop" | "stop"
        ) {
            self.session_identity
                .roster_waiting
                .remove(feature_session_id);
        }
    }

    /// What the roster says the session waits for, if anything.
    pub fn roster_waiting_for(&self, session_id: &str) -> Option<&str> {
        self.session_identity
            .roster_waiting
            .get(session_id)
            .map(String::as_str)
    }

    /// External sessions in the feature's checkout.
    pub fn external_sessions_for_feature(&self, feature_id: &str) -> Vec<&ExternalSession> {
        self.session_identity
            .externals
            .iter()
            .filter(|e| e.feature_id.as_deref() == Some(feature_id))
            .collect()
    }

    /// External sessions in the project's repo but no feature's checkout.
    pub fn external_sessions_for_project(&self, project_name: &str) -> Vec<&ExternalSession> {
        self.session_identity
            .externals
            .iter()
            .filter(|e| e.project_name == project_name && e.feature_id.is_none())
            .collect()
    }

    /// `E`: list external Claude sessions.
    pub fn open_external_sessions(&mut self) {
        if !self.config.aiw.claude_agents_roster {
            self.push_toast_info("The Claude roster is off (claude_agents_roster in config.json)");
            return;
        }
        if self.session_identity.externals.is_empty() {
            self.push_toast_info("No Claude sessions outside AIW in your projects");
            return;
        }
        self.mode = AppMode::ExternalSessions(ExternalSessionsState::default());
    }

    /// Enter in the `E` list: copy the attach command, when there is one.
    pub fn copy_external_attach_command(&mut self) {
        let AppMode::ExternalSessions(state) = &self.mode else {
            return;
        };
        let Some(external) = self.session_identity.externals.get(state.selected) else {
            return;
        };
        let name = external.entry.display_name();
        match external.entry.attach_command() {
            Some(command) => match super::util::copy_to_clipboard(&command) {
                Ok(()) => self.push_toast_success(format!("Copied: {command}")),
                Err(_) => self.push_toast_info(format!("Run: {command}")),
            },
            None => self.push_toast_info(format!(
                "'{name}' runs in another terminal; switch to it there (only background sessions can be attached)"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_app_with_claude_session() -> (App, tempfile::TempDir) {
        let mut app = crate::app::fork_workspace::workspace_app(&[SessionKind::Claude]);
        let dir = tempfile::TempDir::new().unwrap();
        app.db = Some(crate::db::AiwDb::open(&dir.path().join("aiw.db")).unwrap());
        (app, dir)
    }

    fn entry(cwd: &str) -> RosterEntry {
        RosterEntry {
            cwd: cwd.into(),
            kind: "interactive".into(),
            ..Default::default()
        }
    }

    #[test]
    fn names_are_assigned_persisted_and_forgotten() {
        let (mut app, _dir) = fixture_app_with_claude_session();
        let session_id = app.store.projects[0].features[0].sessions[0].id.clone();
        let name = app.assign_session_name(&session_id).unwrap();
        assert_eq!(app.session_harness_name(&session_id), Some(name.as_str()));
        app.persist_session_names();
        let stored = app.db.as_ref().unwrap().all_session_names().unwrap();
        assert_eq!(stored.get(&session_id), Some(&name));
        // Reloading keeps it.
        app.session_identity.names = SessionNames::default();
        app.load_session_names();
        assert_eq!(app.session_harness_name(&session_id), Some(name.as_str()));
        app.forget_session_identity(&session_id);
        assert_eq!(app.session_harness_name(&session_id), None);
        assert!(
            app.db
                .as_ref()
                .unwrap()
                .all_session_names()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn roster_entries_split_into_own_waiting_and_external() {
        let (mut app, _dir) = fixture_app_with_claude_session();
        // A worktree outside the repo, as AIW makes them.
        let workdir = PathBuf::from("/tmp/aiw-roster-test-wt");
        app.store.projects[0].features[0].workdir = workdir.clone();
        let repo = app.store.projects[0].repo.clone();
        let feature_id = app.store.projects[0].features[0].id.clone();
        let session_id = app.store.projects[0].features[0].sessions[0].id.clone();
        app.store.projects[0].features[0].sessions[0].claude_session_id = Some("conv-1".into());
        let name = app.assign_session_name(&session_id).unwrap();

        let mut own_by_id = entry(workdir.to_str().unwrap());
        own_by_id.session_id = Some("conv-1".into());
        own_by_id.status = Some("waiting".into());
        own_by_id.waiting_for = Some("permission prompt".into());
        let mut own_by_name = entry(workdir.to_str().unwrap());
        own_by_name.name = Some(name);
        let mut external_in_feature = entry(workdir.join("src").to_str().unwrap());
        external_in_feature.session_id = Some("other".into());
        let external_in_repo = entry(repo.join("docs").to_str().unwrap());
        let elsewhere = entry("/somewhere/else");

        assert!(app.apply_claude_roster(vec![
            own_by_id,
            own_by_name,
            external_in_feature,
            external_in_repo,
            elsewhere,
        ]));
        assert_eq!(
            app.roster_waiting_for(&session_id),
            Some("permission prompt")
        );
        let in_feature = app.external_sessions_for_feature(&feature_id);
        assert_eq!(in_feature.len(), 1);
        assert_eq!(in_feature[0].entry.session_id.as_deref(), Some("other"));
        let project_name = app.store.projects[0].name.clone();
        assert_eq!(app.external_sessions_for_project(&project_name).len(), 1);
        // A feature in the repo dir takes the repo's sessions instead.
        app.store.projects[0].features[0].workdir = repo.clone();
        app.apply_claude_roster(app.session_identity.roster.clone());
        assert_eq!(app.external_sessions_for_project(&project_name).len(), 0);
        assert_eq!(app.external_sessions_for_feature(&feature_id).len(), 2);
        app.store.projects[0].features[0].workdir = workdir.clone();
        app.apply_claude_roster(app.session_identity.roster.clone());
        // A hook showing the agent running again clears the roster's
        // waiting signal before the next read; an unrelated message keeps it.
        app.roster_saw_hook_activity(&session_id, "session-status");
        assert_eq!(
            app.roster_waiting_for(&session_id),
            Some("permission prompt")
        );
        app.roster_saw_hook_activity(&session_id, "tool-start");
        assert_eq!(app.roster_waiting_for(&session_id), None);
        app.apply_claude_roster(app.session_identity.roster.clone());
        // The same roster again changes nothing; an empty one clears all.
        assert!(!app.apply_claude_roster(app.session_identity.roster.clone()));
        assert!(app.apply_claude_roster(Vec::new()));
        assert!(app.session_identity.externals.is_empty());
        assert_eq!(app.roster_waiting_for(&session_id), None);
    }

    #[test]
    fn fork_needs_a_saved_conversation_and_offers_both_targets() {
        let (mut app, _dir) = fixture_app_with_claude_session();
        app.selection = Selection::Session(0, 0, 0);
        app.start_fork_session();
        assert!(matches!(app.mode, AppMode::Normal), "no conversation yet");
        app.store.projects[0].features[0].sessions[0].claude_session_id = Some("conv-1".into());
        app.start_fork_session();
        let AppMode::ForkSession(state) = &app.mode else {
            panic!("expected the fork dialog");
        };
        assert_eq!(state.source.harness_session_id, "conv-1");
        assert_eq!(state.source.agent, AgentKind::Claude);
        assert_eq!(state.choice, 0);
    }

    #[test]
    fn a_feature_fork_is_claimed_by_the_first_matching_session() {
        let (mut app, _dir) = fixture_app_with_claude_session();
        let project_name = app.store.projects[0].name.clone();
        let feature_name = app.store.projects[0].features[0].name.clone();
        let session_id = app.store.projects[0].features[0].sessions[0].id.clone();
        let source = ForkSource {
            agent: AgentKind::Claude,
            harness_session_id: "conv-src".into(),
            source_label: "Claude 1".into(),
        };
        app.session_identity
            .pending_feature_forks
            .insert((project_name.clone(), feature_name), source.clone());
        let feature = app.store.projects[0].features[0].clone();
        claim_feature_fork(&mut app.session_identity, &project_name, &feature);
        assert!(app.session_identity.pending_feature_forks.is_empty());
        assert_eq!(
            app.session_identity.pending_forks.get(&session_id),
            Some(&source)
        );
    }
}
