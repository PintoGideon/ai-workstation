//! FORK: keep the session list honest when a tmux window closes on its own,
//! or when an agent exits and leaves its pane at a shell prompt.
//!
//! A session's window can disappear without AI Workstation doing anything: the user types
//! `exit` in a terminal, or an agent CLI quits. Upstream AIW only learns a
//! session is down when it stopped it itself, so the vanished session stayed
//! listed as running and opening it failed while attaching to a missing window.
//!
//! Reconciliation, run on the background tick and before a session is opened:
//! - a plain **terminal** whose window is gone is removed from the list (closing
//!   a shell means you're done with it);
//! - any other session (agents, editors, custom commands) is marked **stopped**
//!   (shown as ■), so opening it restarts it — agents resume their conversation
//!   through AIW's existing stopped-session flow;
//! - an **agent** window that is still there but whose agent pane (the
//!   window's original pane) is back at its shell — the shell in the
//!   foreground *and* with no child processes — is marked **exited**: ■ with
//!   an "exited" status, no "finished" attention, and `c` or `S` launch the
//!   agent again into that pane. Requiring no children keeps a harness that
//!   runs as a child of `sh` (one started before agents were `exec`ed, or
//!   one that is a shell script) from reading as exited; looking only at
//!   the original pane keeps a split pane running `htop` from hiding an
//!   exit. An exited agent that is still exited after AIW restarts is
//!   marked again on the first sweep.
//!
//! Nothing is touched when tmux can't be reached, when the feature itself is
//! stopped or its tmux session is gone (AIW's feature-level recovery handles
//! that), or for sessions created in the last few seconds (their window may
//! not exist yet, or the agent may still be starting). The session open on
//! screen is handled last: when its window turns out to be gone the view is
//! left for the dashboard with a message, instead of showing a dead frame.
//!
//! One `tmux list-panes -a` per sweep answers both questions.

use std::collections::HashMap;

use crate::app::{App, Selection};
use crate::project::{ProjectStatus, SessionKind};
use crate::tmux::PaneCommand;

/// Sessions younger than this are left alone: their window may still be
/// being created, or their agent still starting up at the shell.
const CREATION_GRACE_SECS: i64 = 10;

/// Status shown on a session whose agent exited but whose shell survived.
pub(crate) const EXITED_STATUS: &str = "exited";

/// What leaving the view says when the viewed window is gone.
pub(crate) const VIEWED_WINDOW_GONE: &str =
    "That session's tmux window is gone — returned to the dashboard";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fix {
    Remove,
    MarkStopped,
    MarkExited,
    /// Already stopped (and its window still at the shell), but not known
    /// to be exited: AIW restarted since. Restore the marker.
    RestoreExited,
    /// Marked exited, but the pane runs an agent again (relaunched by `c`
    /// while a sweep still saw the shell, or started by hand): clear it.
    ClearExited,
}

/// FORK: a window's agent pane: the pane the window was created with (the
/// lowest pane id), where AIW launched the agent.
#[derive(Debug, Clone)]
pub(crate) struct AgentPane {
    pub(crate) pane: PaneCommand,
    /// Whether the window has other panes (the user split it).
    pub(crate) split: bool,
}

impl AgentPane {
    /// The agent in this pane has exited and left its shell idle.
    pub(crate) fn agent_exited(&self) -> bool {
        is_plain_shell(&self.pane.command) && self.pane.shell_has_children == Some(false)
    }
}

/// Group panes by `(session, window)`, keeping each window's agent pane.
pub(crate) fn agent_panes(panes: Vec<PaneCommand>) -> HashMap<(String, String), AgentPane> {
    let mut windows: HashMap<(String, String), AgentPane> = HashMap::new();
    for pane in panes {
        let key = (pane.session.clone(), pane.window.clone());
        match windows.get_mut(&key) {
            None => {
                windows.insert(key, AgentPane { pane, split: false });
            }
            Some(window) => {
                window.split = true;
                if pane.pane_number() < window.pane.pane_number() {
                    window.pane = pane;
                }
            }
        }
    }
    windows
}

/// Whether `command` (tmux's `pane_current_command`) is a plain shell, i.e.
/// nothing is running in the pane. Login shells are reported with a leading
/// `-` by some tmux versions; a path is reduced to its last component.
pub(crate) fn is_plain_shell(command: &str) -> bool {
    let name = command
        .trim()
        .trim_start_matches('-')
        .rsplit('/')
        .next()
        .unwrap_or_default();
    matches!(
        name,
        "sh" | "bash"
            | "zsh"
            | "fish"
            | "dash"
            | "ksh"
            | "mksh"
            | "tcsh"
            | "csh"
            | "ash"
            | "nu"
            | "elvish"
            | "xonsh"
            | "login"
    )
}

impl App {
    /// Reconciles sessions against the panes tmux actually has. Returns
    /// `true` when anything changed (and was saved).
    pub(crate) fn reconcile_vanished_windows(&mut self) -> bool {
        let panes = self.tmux.pane_commands();
        // Empty means tmux could not be reached (or nothing runs at all):
        // never conclude every window vanished from that.
        if panes.is_empty() {
            return false;
        }
        let commands = agent_panes(panes);

        // FORK(cockpit): the agent pane shows this window; when it goes the
        // view client ends and the placeholder takes over by itself, the
        // sidebar only has to say so.
        let viewing = self
            .fork_workspace
            .live
            .as_ref()
            .map(|live| (live.session.clone(), live.window.clone()));
        let cutoff = chrono::Utc::now() - chrono::Duration::seconds(CREATION_GRACE_SECS);

        let mut fixes: Vec<(usize, usize, usize, Fix)> = Vec::new();
        for (pi, project) in self.store.projects.iter().enumerate() {
            for (fi, feature) in project.features.iter().enumerate() {
                // A feature whose whole tmux session is gone (its last window
                // closed) is treated like one that lost every window: each
                // session is marked, so Enter restarts it instead of typing
                // into nothing. Stopped features are already settled.
                if feature.status == ProjectStatus::Stopped {
                    continue;
                }
                for (si, session) in feature.sessions.iter().enumerate() {
                    if !session.kind.is_tmux_backed() || session.created_at >= cutoff {
                        continue;
                    }
                    let key = (feature.tmux_session.clone(), session.tmux_window.clone());
                    if session.stopped {
                        // The in-memory marker does not survive a restart of
                        // AIW; the stopped flag does. Without the marker a
                        // restart would take the window for "up after all".
                        if session.kind.is_agent_harness()
                            && !self.exited_sessions.contains(&session.id)
                            && commands.get(&key).is_some_and(AgentPane::agent_exited)
                        {
                            fixes.push((pi, fi, si, Fix::RestoreExited));
                        }
                        continue;
                    }
                    let Some(window) = commands.get(&key) else {
                        // The window is gone. The one on screen is handled
                        // below, after the list is settled.
                        if viewing.as_ref() == Some(&key) {
                            continue;
                        }
                        let fix = if session.kind == SessionKind::Terminal {
                            Fix::Remove
                        } else {
                            Fix::MarkStopped
                        };
                        fixes.push((pi, fi, si, fix));
                        continue;
                    };
                    if session.kind.is_agent_harness() && window.agent_exited() {
                        fixes.push((pi, fi, si, Fix::MarkExited));
                    } else if session.kind.is_agent_harness()
                        && self.exited_sessions.contains(&session.id)
                        && !is_plain_shell(&window.pane.command)
                    {
                        fixes.push((pi, fi, si, Fix::ClearExited));
                    }
                }
            }
        }

        // The viewed session: its window is gone (or its whole tmux session
        // is), so the pane on screen is a dead frame. Mark it like any other
        // vanished agent and go back to the dashboard.
        let mut leave_view = false;
        if let Some(key) = &viewing
            && !commands.contains_key(key)
        {
            leave_view = true;
            if let Some((pi, fi, si)) = self.session_indices_for_window(&key.0, &key.1)
                && let Some(session) = self
                    .store
                    .projects
                    .get(pi)
                    .and_then(|p| p.features.get(fi))
                    .and_then(|f| f.sessions.get(si))
                && session.kind.is_tmux_backed()
                && !session.stopped
                && session.kind != SessionKind::Terminal
            {
                fixes.push((pi, fi, si, Fix::MarkStopped));
            }
        }

        if fixes.is_empty() && !leave_view {
            return false;
        }

        // Highest index first so removals don't shift pending indices.
        fixes.sort_by_key(|f| std::cmp::Reverse((f.0, f.1, f.2)));
        fixes.dedup();
        for (pi, fi, si, fix) in fixes {
            match fix {
                Fix::MarkStopped => self.set_session_stopped(pi, fi, si, true),
                Fix::MarkExited => self.mark_session_exited(pi, fi, si),
                Fix::RestoreExited => {
                    let session = &mut self.store.projects[pi].features[fi].sessions[si];
                    session.status_text = Some(EXITED_STATUS.to_string());
                    self.exited_sessions.insert(session.id.clone());
                }
                Fix::ClearExited => {
                    let session = &mut self.store.projects[pi].features[fi].sessions[si];
                    if self.exited_sessions.remove(&session.id)
                        && session.status_text.as_deref() == Some(EXITED_STATUS)
                    {
                        session.status_text = None;
                    }
                }
                Fix::Remove => {
                    self.store.projects[pi].features[fi].sessions.remove(si);
                    self.selection = selection_after_removal(&self.selection, pi, fi, si);
                }
            }
        }
        if leave_view {
            self.message = Some(VIEWED_WINDOW_GONE.to_string());
            self.log_info("reconcile", VIEWED_WINDOW_GONE.to_string());
        }
        if let Err(err) = self.save() {
            self.message = Some(format!("Could not save session list: {err}"));
        }
        true
    }

    /// The agent in `(pi, fi, si)` is gone but its pane's shell is still
    /// there: ■ "exited", not "finished". The feature's "completed, awaiting
    /// review" attention is dropped when this was its last running agent —
    /// a crash is not a finished turn.
    fn mark_session_exited(&mut self, pi: usize, fi: usize, si: usize) {
        let Some(feature) = self
            .store
            .projects
            .get_mut(pi)
            .and_then(|p| p.features.get_mut(fi))
        else {
            return;
        };
        let tmux_session = feature.tmux_session.clone();
        let feature_name = feature.name.clone();
        let Some(session) = feature.sessions.get_mut(si) else {
            return;
        };
        session.stopped = true;
        session.status_text = Some(EXITED_STATUS.to_string());
        let id = session.id.clone();
        let label = session.label.clone();
        let other_agent_running = feature
            .sessions
            .iter()
            .enumerate()
            .any(|(i, s)| i != si && s.kind.is_agent_harness() && s.runs_with_feature());
        self.exited_sessions.insert(id);
        if !other_agent_running {
            self.clear_attention(&tmux_session);
        }
        self.log_info(
            "reconcile",
            format!("{label} in {feature_name} exited; its shell is still open (■ exited)"),
        );
    }

    /// `(project, feature, session)` indices of the session shown in tmux
    /// window `window` of `tmux_session`.
    fn session_indices_for_window(
        &self,
        tmux_session: &str,
        window: &str,
    ) -> Option<(usize, usize, usize)> {
        self.store
            .projects
            .iter()
            .enumerate()
            .find_map(|(pi, project)| {
                project
                    .features
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| f.tmux_session == tmux_session)
                    .find_map(|(fi, feature)| {
                        feature
                            .sessions
                            .iter()
                            .position(|s| s.tmux_window == window)
                            .map(|si| (pi, fi, si))
                    })
            })
    }

    /// FORK: the agent pane of `tmux_session:window` when its agent has
    /// exited and left the shell idle, asked of tmux now. `None` when the
    /// agent is running, the window is gone, or it cannot be told.
    pub(crate) fn exited_agent_pane(&self, tmux_session: &str, window: &str) -> Option<AgentPane> {
        agent_panes(self.tmux.pane_commands())
            .remove(&(tmux_session.to_string(), window.to_string()))
            .filter(AgentPane::agent_exited)
    }

    /// Whether session `id` is marked exited (agent gone, shell alive).
    pub fn session_is_exited(&self, id: &str) -> bool {
        self.exited_sessions.contains(id)
    }

    /// Projects whose repo folder no longer exists: flag them (once, with a
    /// toast) so the dashboard can mark the row; sessions keep running in a
    /// deleted directory until the user acts. A folder that comes back
    /// clears the flag. Returns whether the set of flagged projects changed.
    pub(crate) fn reconcile_missing_project_dirs(&mut self) -> bool {
        let mut changed = false;
        let mut gone: Vec<(String, String, String, usize)> = Vec::new();
        for project in &self.store.projects {
            let exists = project.repo.exists();
            let flagged = self.missing_project_repos.contains(&project.id);
            if !exists && !flagged {
                let running = project
                    .features
                    .iter()
                    .filter(|f| f.status != ProjectStatus::Stopped)
                    .count();
                gone.push((
                    project.id.clone(),
                    project.name.clone(),
                    project.repo.display().to_string(),
                    running,
                ));
            } else if exists && flagged {
                self.missing_project_repos.remove(&project.id);
                changed = true;
            }
        }
        for (id, name, path, running) in gone {
            self.missing_project_repos.insert(id);
            let msg = if running > 0 {
                format!(
                    "Project '{name}' folder is gone ({path}); {running} feature(s) still run in a deleted directory"
                )
            } else {
                format!("Project '{name}' folder is gone ({path})")
            };
            self.log_warn("reconcile", msg.clone());
            self.push_toast_warning(msg);
            changed = true;
        }
        changed
    }
}

/// Where the selection should point after session `(pi, fi, si)` is removed.
pub(crate) fn selection_after_removal(
    selection: &Selection,
    pi: usize,
    fi: usize,
    si: usize,
) -> Selection {
    match *selection {
        Selection::Session(p, f, s) if p == pi && f == fi && s == si => Selection::Feature(pi, fi),
        Selection::Session(p, f, s) if p == pi && f == fi && s > si => {
            Selection::Session(p, f, s - 1)
        }
        ref other => other.clone(),
    }
}

impl App {}

#[cfg(test)]
mod tests {
    use super::is_plain_shell;

    #[test]
    fn shells_are_recognised_in_every_spelling() {
        for shell in ["bash", "-zsh", "/bin/sh", "fish", "-/usr/bin/dash", "login"] {
            assert!(is_plain_shell(shell), "{shell}");
        }
        for agent in [
            "claude", "codex", "python3", "node", "", "sh-agent", "bashful",
        ] {
            assert!(!is_plain_shell(agent), "{agent}");
        }
    }
}
