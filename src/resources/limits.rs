//! How many agent harnesses are running right now, across every project.
//!
//! The store is machine-global, so this census is too: the concurrency limit
//! is about what the host is carrying, not what one project is doing. Only
//! agent-harness sessions count — terminals, editors, and TODOs sessions cost
//! the machine little and are not what the gate is protecting against.
//!
//! "Running" means a process is alive in the session's tmux pane, not merely
//! that the pane exists — see [`LiveHarnesses`].

use std::collections::{HashMap, HashSet};

use crate::project::{ProjectStatus, ProjectStore, SessionKind};
use crate::resources::procs::{self, ProcInfo};
use crate::traits::TmuxOps;

/// The tmux windows that have something running in them right now, grouped by
/// session.
///
/// A feature's stored session list is a record of what was *created*, not what
/// is *running*, and neither is the window list: feature startup creates a
/// window for every saved session even when
/// `max_agent_autostart_sessions` stops it short of launching the harness in
/// them, and an agent the user quit leaves its window behind at a shell
/// prompt. Both look identical to `list-windows`.
///
/// What separates them is the pane's process tree. tmux starts a shell in each
/// pane and reports its pid; AIW launches a harness as a child of that shell
/// (`sh /tmp/aiw-agent-launch-*.sh`, which in turn runs the agent), so the
/// shell has a child for exactly as long as the harness lives. A pane sitting
/// at a prompt — never launched, or exited — has none.
///
/// The test is deliberately "any child" rather than "a process that looks like
/// a harness": harness binaries are not reliably identifiable from their argv
/// (Claude Code execs a version-numbered path), and a user who quits and
/// re-runs the agent by hand is still running an agent. Something else being
/// run in an agent window counts too, which is the safe direction to be wrong
/// in for a gate that only ever warns.
#[derive(Debug, Default, Clone)]
pub struct LiveHarnesses {
    windows: HashMap<String, HashSet<String>>,
}

impl LiveHarnesses {
    /// Ask tmux for every pane on the server, then ask `ps` which of those
    /// panes is running something: two process spawns, regardless of how many
    /// sessions exist.
    pub fn probe(tmux: &dyn TmuxOps) -> Self {
        Self::from_census(&tmux.list_panes(), &procs::list_processes())
    }

    /// Decide which windows are busy from an already-gathered census.
    ///
    /// An empty process list means `ps` could not be reached, not that the
    /// machine is idle — there is no signal to filter with, so every live pane
    /// counts. That is the pre-existing over-counting answer, and the right
    /// direction to fail in when the alternative is a gate that silently never
    /// fires.
    pub fn from_census(panes: &[(String, String, i64)], processes: &[ProcInfo]) -> Self {
        let no_process_signal = processes.is_empty();
        let parents: HashSet<i64> = processes.iter().map(|proc| proc.ppid).collect();

        let mut windows: HashMap<String, HashSet<String>> = HashMap::new();
        for (session, window, pane_pid) in panes {
            if no_process_signal || parents.contains(pane_pid) {
                windows
                    .entry(session.clone())
                    .or_default()
                    .insert(window.clone());
            }
        }
        Self { windows }
    }

    /// Whether this window has a live process in it.
    pub fn is_running(&self, session: &str, window: &str) -> bool {
        self.windows
            .get(session)
            .is_some_and(|names| names.contains(window))
    }

    /// Test helper: these windows are running something.
    #[cfg(test)]
    pub fn from_pairs<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let mut windows: HashMap<String, HashSet<String>> = HashMap::new();
        for (session, window) in pairs {
            windows
                .entry(session.to_string())
                .or_default()
                .insert(window.to_string());
        }
        Self { windows }
    }
}

/// One running harness, identified well enough to name it in a warning dialog
/// or an `aiw doctor` report.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveHarness {
    pub project_name: String,
    pub feature_id: String,
    pub feature_name: String,
    pub session_id: String,
    pub session_label: String,
    pub kind: SessionKind,
}

/// Every agent-harness session that is actually running.
pub fn active_harness_sessions(store: &ProjectStore, live: &LiveHarnesses) -> Vec<ActiveHarness> {
    let mut active = Vec::new();
    for project in &store.projects {
        for feature in &project.features {
            // A stopped feature has no tmux session at all; skip it before
            // looking at its saved sessions.
            if feature.status == ProjectStatus::Stopped {
                continue;
            }
            for session in &feature.sessions {
                if !session.kind.is_agent_harness()
                    || !live.is_running(&feature.tmux_session, &session.tmux_window)
                {
                    continue;
                }
                active.push(ActiveHarness {
                    project_name: project.name.clone(),
                    feature_id: feature.id.clone(),
                    feature_name: feature.name.clone(),
                    session_id: session.id.clone(),
                    session_label: session.label.clone(),
                    kind: session.kind.clone(),
                });
            }
        }
    }
    active
}

/// Count of running agent-harness sessions across all projects.
pub fn count_active_harness_sessions(store: &ProjectStore, live: &LiveHarnesses) -> usize {
    active_harness_sessions(store, live).len()
}

/// Upper bound on running harnesses, from the store alone.
///
/// Every session counted by [`count_active_harness_sessions`] is also counted
/// here — the live-pane check can only ever remove sessions — so a caller
/// whose bound is already under the limit can skip asking tmux entirely.
pub fn max_possible_harness_sessions(store: &ProjectStore) -> usize {
    store
        .projects
        .iter()
        .flat_map(|project| project.features.iter())
        .filter(|feature| feature.status != ProjectStatus::Stopped)
        .flat_map(|feature| feature.sessions.iter())
        .filter(|session| session.kind.is_agent_harness())
        .count()
}

/// Everything the concurrency limit is measured against: the interactive
/// harness sessions that are running.
pub fn total_active_agents(store: &ProjectStore, live: &LiveHarnesses) -> usize {
    count_active_harness_sessions(store, live)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{Feature, FeatureSession, Project, VibeMode};
    use chrono::Utc;
    use std::path::PathBuf;

    fn session(id: &str, kind: SessionKind, window: &str) -> FeatureSession {
        FeatureSession {
            id: id.to_string(),
            kind,
            label: id.to_string(),
            tmux_window: window.to_string(),
            claude_session_id: None,
            todo_reference: None,
            token_usage_source: None,
            token_usage_source_match: None,
            created_at: Utc::now(),
            command: None,
            on_stop: None,
            pre_check: None,
            status_text: None,
            token_usage: None,
            stopped: false,
        }
    }

    fn feature(name: &str, status: ProjectStatus, sessions: Vec<FeatureSession>) -> Feature {
        Feature {
            id: format!("feat-{name}"),
            name: name.to_string(),
            branch: name.to_string(),
            workdir: PathBuf::from("/tmp").join(name),
            is_worktree: true,
            tmux_session: format!("aiw-{name}"),
            sessions,
            collapsed: true,
            mode: VibeMode::default(),
            agent: Default::default(),
            enable_chrome: false,
            pending_worktree_script: false,
            ready: true,
            status,
            created_at: Utc::now(),
            last_accessed: Utc::now(),
            summary: None,
            summary_updated_at: None,
            nickname: None,
            issue_source: None,
        }
    }

    fn store(projects: Vec<Project>) -> ProjectStore {
        ProjectStore {
            version: 1,
            projects,
            session_bookmarks: Vec::new(),
            available_harnesses: Vec::new(),
            prompt_templates: Vec::new(),
            extra: Default::default(),
        }
    }

    fn project(name: &str, features: Vec<Feature>) -> Project {
        Project {
            id: format!("proj-{name}"),
            name: name.to_string(),
            repo: PathBuf::from("/tmp").join(name),
            collapsed: false,
            features,
            created_at: Utc::now(),
            preferred_agent: Default::default(),
            is_git: true,
        }
    }

    #[test]
    fn counts_only_harness_sessions() {
        let feat = feature(
            "alpha",
            ProjectStatus::Active,
            vec![
                session("s1", SessionKind::Claude, "claude"),
                session("s2", SessionKind::Codex, "codex"),
                session("s3", SessionKind::Terminal, "terminal"),
                session("s4", SessionKind::Nvim, "nvim"),
                session("s5", SessionKind::Vscode, "vscode"),
                session("s6", SessionKind::Custom, "dev-server"),
                session("s7", SessionKind::Todos, "todos"),
            ],
        );
        let live = LiveHarnesses::from_pairs([
            ("aiw-alpha", "claude"),
            ("aiw-alpha", "codex"),
            ("aiw-alpha", "terminal"),
            ("aiw-alpha", "nvim"),
            ("aiw-alpha", "vscode"),
            ("aiw-alpha", "dev-server"),
            ("aiw-alpha", "todos"),
        ]);
        let store = store(vec![project("p", vec![feat])]);

        assert_eq!(count_active_harness_sessions(&store, &live), 2);
    }

    #[test]
    fn counts_across_projects_and_features() {
        let store = store(vec![
            project(
                "one",
                vec![
                    feature(
                        "alpha",
                        ProjectStatus::Active,
                        vec![session("s1", SessionKind::Claude, "claude")],
                    ),
                    feature(
                        "beta",
                        ProjectStatus::Idle,
                        vec![session("s2", SessionKind::Codex, "codex")],
                    ),
                ],
            ),
            project(
                "two",
                vec![feature(
                    "gamma",
                    ProjectStatus::Active,
                    vec![session("s3", SessionKind::Pi, "pi")],
                )],
            ),
        ]);
        let live = LiveHarnesses::from_pairs([
            ("aiw-alpha", "claude"),
            ("aiw-beta", "codex"),
            ("aiw-gamma", "pi"),
        ]);

        assert_eq!(count_active_harness_sessions(&store, &live), 3);
    }

    #[test]
    fn skips_stopped_features() {
        let store = store(vec![project(
            "p",
            vec![feature(
                "alpha",
                ProjectStatus::Stopped,
                vec![session("s1", SessionKind::Claude, "claude")],
            )],
        )]);
        // Even if a stale window were reported, a stopped feature never counts.
        let live = LiveHarnesses::from_pairs([("aiw-alpha", "claude")]);

        assert_eq!(count_active_harness_sessions(&store, &live), 0);
    }

    #[test]
    fn skips_saved_sessions_whose_harness_is_not_running() {
        // Second harness pane was recreated but left at a shell prompt, or was
        // never recreated at all: only panes with a process count.
        let store = store(vec![project(
            "p",
            vec![feature(
                "alpha",
                ProjectStatus::Active,
                vec![
                    session("s1", SessionKind::Claude, "claude"),
                    session("s2", SessionKind::Claude, "claude-2"),
                ],
            )],
        )]);
        let live = LiveHarnesses::from_pairs([("aiw-alpha", "claude")]);

        assert_eq!(count_active_harness_sessions(&store, &live), 1);
    }

    fn proc(pid: i64, ppid: i64, args: &str) -> ProcInfo {
        ProcInfo {
            pid,
            ppid,
            args: args.to_string(),
        }
    }

    /// The census's whole job: three windows that all exist, only one of which
    /// has a harness in it. The other two are the two ways a window outlives
    /// its agent — created past `max_agent_autostart_sessions` and so never
    /// launched, and launched but since exited back to the prompt.
    #[test]
    fn an_idle_pane_is_not_a_running_harness() {
        let panes = vec![
            ("aiw-alpha".to_string(), "claude".to_string(), 100),
            ("aiw-alpha".to_string(), "claude-2".to_string(), 200),
            ("aiw-alpha".to_string(), "claude-3".to_string(), 300),
        ];
        let processes = vec![
            proc(100, 1, "-zsh"),
            proc(101, 100, "sh /tmp/aiw-agent-launch-abc.sh"),
            proc(102, 101, "/home/me/.local/share/claude/versions/2.1.226"),
            // Never launched: tmux made the window, nothing was ever run.
            proc(200, 1, "-zsh"),
            // Launched once, then quit: the shell is back in the foreground.
            proc(300, 1, "-zsh"),
        ];

        let live = LiveHarnesses::from_census(&panes, &processes);

        assert!(live.is_running("aiw-alpha", "claude"));
        assert!(!live.is_running("aiw-alpha", "claude-2"));
        assert!(!live.is_running("aiw-alpha", "claude-3"));
    }

    #[test]
    fn a_window_tmux_no_longer_reports_is_not_running() {
        let live = LiveHarnesses::from_census(&[], &[proc(100, 1, "-zsh")]);

        assert!(!live.is_running("aiw-alpha", "claude"));
    }

    /// A window with several panes counts as busy when any one of them is.
    #[test]
    fn a_split_window_counts_once_if_either_pane_is_busy() {
        let panes = vec![
            ("aiw-alpha".to_string(), "claude".to_string(), 100),
            ("aiw-alpha".to_string(), "claude".to_string(), 200),
        ];
        let processes = vec![
            proc(100, 1, "-zsh"),
            proc(200, 1, "-zsh"),
            proc(201, 200, "sh /tmp/aiw-agent-launch-abc.sh"),
        ];

        let store = store(vec![project(
            "p",
            vec![feature(
                "alpha",
                ProjectStatus::Active,
                vec![session("s1", SessionKind::Claude, "claude")],
            )],
        )]);

        let live = LiveHarnesses::from_census(&panes, &processes);
        assert_eq!(count_active_harness_sessions(&store, &live), 1);
    }

    /// No `ps` means no signal, which must not read as "nothing is running" —
    /// that would silently switch the gate off.
    #[test]
    fn an_unreadable_process_list_falls_back_to_pane_existence() {
        let panes = vec![("aiw-alpha".to_string(), "claude".to_string(), 100)];

        let live = LiveHarnesses::from_census(&panes, &[]);

        assert!(live.is_running("aiw-alpha", "claude"));
    }

    #[test]
    fn reports_who_is_running() {
        let store = store(vec![project(
            "workstation",
            vec![feature(
                "alpha",
                ProjectStatus::Active,
                vec![session("s1", SessionKind::Claude, "claude")],
            )],
        )]);
        let live = LiveHarnesses::from_pairs([("aiw-alpha", "claude")]);

        let active = active_harness_sessions(&store, &live);
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].project_name, "workstation");
        assert_eq!(active[0].feature_name, "alpha");
        assert_eq!(active[0].kind, SessionKind::Claude);
    }

    #[test]
    fn empty_store_counts_zero() {
        assert_eq!(
            count_active_harness_sessions(&store(Vec::new()), &LiveHarnesses::default()),
            0
        );
    }
}
