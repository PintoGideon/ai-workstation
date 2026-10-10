//! FORK: keep the session list honest when a tmux window closes on its own.
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
//!   through AIW's existing stopped-session flow.
//!
//! Nothing is touched when tmux can't be reached, when the feature itself is
//! stopped or its tmux session is gone (AIW's feature-level recovery handles
//! that), for sessions created in the last few seconds (their window may not
//! exist yet), or for the session currently open on screen.

use std::collections::HashSet;

use crate::app::{App, AppMode, Selection};
use crate::project::{ProjectStatus, SessionKind};

/// Sessions younger than this are left alone: their window may still be
/// being created.
const CREATION_GRACE_SECS: i64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Fix {
    Remove,
    MarkStopped,
}

impl App {
    /// Reconciles sessions against the windows tmux actually has. Returns
    /// `true` when anything changed (and was saved).
    pub(crate) fn reconcile_vanished_windows(&mut self) -> bool {
        let windows = self.tmux.window_activity();
        // Empty means tmux could not be reached (or nothing runs at all):
        // never conclude every window vanished from that.
        if windows.is_empty() {
            return false;
        }
        let live: HashSet<(String, String)> = windows.into_iter().map(|(s, w, _)| (s, w)).collect();
        let live_sessions: HashSet<&str> = live.iter().map(|(s, _)| s.as_str()).collect();

        let viewing = match &self.mode {
            AppMode::Viewing(view) => Some((view.session.clone(), view.window.clone())),
            _ => None,
        };
        let cutoff = chrono::Utc::now() - chrono::Duration::seconds(CREATION_GRACE_SECS);

        let mut fixes: Vec<(usize, usize, usize, Fix)> = Vec::new();
        for (pi, project) in self.store.projects.iter().enumerate() {
            for (fi, feature) in project.features.iter().enumerate() {
                if feature.status == ProjectStatus::Stopped
                    || !live_sessions.contains(feature.tmux_session.as_str())
                {
                    continue;
                }
                for (si, session) in feature.sessions.iter().enumerate() {
                    let key = (feature.tmux_session.clone(), session.tmux_window.clone());
                    let vanished = session.kind.is_tmux_backed()
                        && !session.stopped
                        && session.created_at < cutoff
                        && !live.contains(&key)
                        && viewing.as_ref() != Some(&key);
                    if !vanished {
                        continue;
                    }
                    let fix = if session.kind == SessionKind::Terminal {
                        Fix::Remove
                    } else {
                        Fix::MarkStopped
                    };
                    fixes.push((pi, fi, si, fix));
                }
            }
        }
        if fixes.is_empty() {
            return false;
        }

        // Highest index first so removals don't shift pending indices.
        fixes.sort_by_key(|f| std::cmp::Reverse((f.0, f.1, f.2)));
        for (pi, fi, si, fix) in fixes {
            match fix {
                Fix::MarkStopped => self.set_session_stopped(pi, fi, si, true),
                Fix::Remove => {
                    self.store.projects[pi].features[fi].sessions.remove(si);
                    self.selection = selection_after_removal(&self.selection, pi, fi, si);
                }
            }
        }
        if let Err(err) = self.save() {
            self.message = Some(format!("Could not save session list: {err}"));
        }
        true
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
