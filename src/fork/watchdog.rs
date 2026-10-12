//! FORK: §19.8 — the watchdog's pure half: what counts as an agent that
//! needs rescuing, and how long to wait before saying so.
//!
//! Five conditions, checked per agent session:
//! - **exited**: the agent quit and left its pane at the shell (the exited
//!   sweep in `fork::session_reconcile` marks it);
//! - **background failed**: a `claude --bg` session Claude's roster reports
//!   as failed (§19.7);
//! - **stuck**: the session reads as working, *and* no hook event has come
//!   for the stuck window, *and* its pane has not changed for the same window.
//!   Both signals are required: a long build prints but sends no hooks, and a
//!   long tool call sends `PreToolUse` and then nothing while its output
//!   still moves. A session with subagents running is never stuck — its
//!   parent waits on them quietly. The pane comparison ignores digits and
//!   spinner glyphs, because Claude and Codex animate a spinner and count
//!   seconds and tokens even while an API call hangs;
//! - **queued, idle**: follow-ups or messages wait for a session that is not
//!   working, so the `Stop` hook that would deliver them never comes;
//! - **context critical, idle**: the session sits at its prompt past the
//!   context critical threshold.
//!
//! Nothing here acts: `app::fork_watchdog` surfaces the flags and runs the
//! actions the user picks.

use std::hash::{Hash, Hasher};
use std::time::Duration;

use chrono::{DateTime, Utc};

/// Default minutes of silence (no hooks, unchanged pane) before a working
/// session is called stuck.
pub const DEFAULT_STUCK_MINUTES: f64 = 10.0;
/// Default seconds a queued item may wait for an idle session.
pub const DEFAULT_QUEUED_IDLE_SECS: u64 = 30;
/// The fewest minutes between two automatic restarts of one session.
pub const AUTO_RESTART_EVERY_MINUTES: i64 = 60;
/// How often the app re-evaluates the flags.
pub const EVAL_INTERVAL: Duration = Duration::from_secs(2);
/// How long a stuck agent's kill may take to leave its pane at the shell
/// before the restart is given up.
pub const RESTART_DEADLINE_SECS: i64 = 30;

/// Why a session is flagged, most urgent first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum WatchKind {
    Exited,
    BackgroundFailed,
    Stuck,
    QueuedIdle,
    ContextCritical,
}

impl WatchKind {
    /// A short noun for the needs-you list and logs.
    pub fn label(self) -> &'static str {
        match self {
            WatchKind::Exited => "exited",
            WatchKind::BackgroundFailed => "background session failed",
            WatchKind::Stuck => "stuck",
            WatchKind::QueuedIdle => "queued, idle",
            WatchKind::ContextCritical => "context critical",
        }
    }
}

/// A raised flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchFlag {
    pub kind: WatchKind,
    /// When the condition was first seen this time.
    pub since: DateTime<Utc>,
    /// The specific part: `no hooks or output for 12m`, `2 queued`, `88%`.
    pub detail: String,
}

impl WatchFlag {
    /// The row's reason line, ending with the key that opens the actions.
    pub fn reason(&self) -> String {
        let head = match self.kind {
            WatchKind::Exited => "exited".to_string(),
            WatchKind::BackgroundFailed => format!("failed: {}", self.detail),
            WatchKind::Stuck => format!("stuck · {}", self.detail),
            WatchKind::QueuedIdle => "queued for an idle agent".to_string(),
            WatchKind::ContextCritical => format!("ctx {} at the prompt", self.detail),
        };
        format!("{head} · ! recover")
    }
}

/// What the app knows about one agent session right now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Observation {
    /// The agent should be running: the feature runs and the session is not
    /// stopped (or it is a live background session).
    pub running: bool,
    /// Marked exited by the sweep.
    pub exited: bool,
    /// Claude's roster says the background session failed (with why).
    pub background_failed: Option<String>,
    /// Reads as working (a turn or a tool is running).
    pub working: bool,
    /// Subagents running under it.
    pub subagents: usize,
    /// The last hook event of any kind.
    pub last_activity: Option<DateTime<Utc>>,
    /// Since when the pane's fingerprint has not changed.
    pub pane_unchanged_since: Option<DateTime<Utc>>,
    /// Context used, when past the critical threshold.
    pub context_critical: Option<u8>,
    /// Follow-ups and messages queued for it.
    pub queued: usize,
    /// Since when it has been seen not working (while running).
    pub idle_since: Option<DateTime<Utc>>,
    /// It waits on the user already: a permission prompt, a question, an
    /// input request. A session asking for permission looks "working" with
    /// quiet hooks and a still pane, and pasting into its prompt would answer
    /// the question — so nothing but exited / failed is flagged then.
    pub waiting_on_user: bool,
}

/// The windows the detectors use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Thresholds {
    pub stuck: chrono::Duration,
    pub queued_idle: chrono::Duration,
}

impl Thresholds {
    /// From the config's minutes (fractions allowed, for tests) and seconds.
    pub fn from_config(stuck_minutes: f64, queued_idle_secs: u64) -> Self {
        let stuck_ms = if stuck_minutes.is_finite() && stuck_minutes > 0.0 {
            (stuck_minutes * 60_000.0).round() as i64
        } else {
            (DEFAULT_STUCK_MINUTES * 60_000.0) as i64
        };
        Self {
            stuck: chrono::Duration::milliseconds(stuck_ms.max(1_000)),
            queued_idle: chrono::Duration::seconds(queued_idle_secs.max(1) as i64),
        }
    }

    /// How long a dismissed flag stays quiet: the stuck window, at least a
    /// minute.
    pub fn snooze(&self) -> chrono::Duration {
        self.stuck.max(chrono::Duration::minutes(1))
    }
}

/// The condition `obs` meets, most urgent first, with its detail.
pub fn detect(
    obs: &Observation,
    t: &Thresholds,
    now: DateTime<Utc>,
) -> Option<(WatchKind, String)> {
    if obs.exited {
        return Some((WatchKind::Exited, String::new()));
    }
    if let Some(why) = &obs.background_failed {
        return Some((
            WatchKind::BackgroundFailed,
            if why.is_empty() {
                "error".to_string()
            } else {
                why.clone()
            },
        ));
    }
    if !obs.running || obs.waiting_on_user {
        return None;
    }
    if obs.working {
        if obs.subagents > 0 {
            return None;
        }
        let quiet_hooks = obs.last_activity.is_some_and(|at| now - at >= t.stuck);
        let quiet_pane = obs
            .pane_unchanged_since
            .is_some_and(|at| now - at >= t.stuck);
        if quiet_hooks && quiet_pane {
            let since = obs
                .last_activity
                .into_iter()
                .chain(obs.pane_unchanged_since)
                .max()
                .unwrap_or(now);
            return Some((
                WatchKind::Stuck,
                format!("no hooks or output for {}", short_duration(now - since)),
            ));
        }
        return None;
    }
    if obs.queued > 0 && obs.idle_since.is_some_and(|at| now - at >= t.queued_idle) {
        return Some((WatchKind::QueuedIdle, format!("{} queued", obs.queued)));
    }
    obs.context_critical
        .map(|pct| (WatchKind::ContextCritical, format!("{pct}%")))
}

/// `12m`, `1h05m`, `40s`.
pub fn short_duration(d: chrono::Duration) -> String {
    let secs = d.num_seconds().max(0);
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    }
}

/// Spinner glyphs Claude and Codex cycle through on their status line.
fn is_spinner(c: char) -> bool {
    ('\u{2800}'..='\u{28FF}').contains(&c) // braille spinners (Codex)
        || matches!(c, '✻' | '✽' | '✶' | '✳' | '✢' | '·' | '*' | '◐' | '◓' | '◑' | '◒')
}

/// Whether `line` is the harness's own animated status line ("✻ Thinking…
/// (12s · ↑ 3.2k tokens · esc to interrupt)", "⠋ Working (3s • esc to
/// interrupt)"). Only that line ticks on its own while a turn hangs; every
/// other line keeps its digits, so real progress output still counts.
fn is_status_line(line: &str) -> bool {
    line.contains("esc to interrupt")
        || line.contains("to interrupt)")
        || line.trim_start().chars().next().is_some_and(is_spinner)
}

/// A fingerprint of a pane's visible text that ignores what animates on its
/// own — the status line's spinner glyph, elapsed seconds and token counters
/// — and trailing blanks.
pub fn pane_fingerprint(captured: &str) -> u64 {
    let plain = crate::fork::background::strip_ansi(captured);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for line in plain.lines() {
        let kept: String = if is_status_line(line) {
            line.chars()
                .filter(|c| !c.is_ascii_digit() && !is_spinner(*c))
                .collect()
        } else {
            line.to_string()
        };
        let kept = kept.trim_end();
        if !kept.is_empty() {
            kept.hash(&mut hasher);
        }
    }
    hasher.finish()
}

/// How often working panes are captured: a quarter of the stuck window,
/// between 2 and 30 seconds.
pub fn capture_interval(t: &Thresholds) -> Duration {
    let quarter = (t.stuck / 4).to_std().unwrap_or(Duration::from_secs(30));
    quarter.clamp(Duration::from_secs(2), Duration::from_secs(30))
}

/// Whether an automatic restart may run, given the last one.
pub fn auto_restart_allowed(last: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    last.is_none_or(|at| now - at >= chrono::Duration::minutes(AUTO_RESTART_EVERY_MINUTES))
}

/// What the user can do about a flagged (or any) agent session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchAction {
    /// Relaunch resuming the conversation (kill a stuck agent first; respawn
    /// a background one).
    RestartResume,
    /// Press Esc in the agent: interrupts the running turn.
    Interrupt,
    /// Type `/compact` into an idle agent.
    Compact,
    /// Paste the prompt still waiting, else the latest prompt, unsent.
    ReplayPrompt,
    /// Deliver the first queued item now (paste and send).
    DeliverQueued,
    /// Leave it: the flag stays quiet for the snooze window.
    Dismiss,
}

impl WatchAction {
    pub fn label(self) -> &'static str {
        match self {
            WatchAction::RestartResume => "Restart, resuming the conversation",
            WatchAction::Interrupt => "Interrupt the turn (Esc)",
            WatchAction::Compact => "Send /compact",
            WatchAction::ReplayPrompt => "Paste the latest prompt again (unsent)",
            WatchAction::DeliverQueued => "Deliver the first queued item now",
            WatchAction::Dismiss => "Dismiss (keep waiting)",
        }
    }
}

/// The actions that make sense for a session, given its flag and state.
pub fn actions_for(
    flag: Option<WatchKind>,
    obs: &Observation,
    restartable: bool,
    has_prompt: bool,
) -> Vec<WatchAction> {
    let mut out = Vec::new();
    let idle = obs.running && !obs.working && !obs.exited;
    match flag {
        Some(WatchKind::Exited) | Some(WatchKind::BackgroundFailed) => {
            if restartable {
                out.push(WatchAction::RestartResume);
            }
        }
        Some(WatchKind::Stuck) => {
            out.push(WatchAction::Interrupt);
            if restartable {
                out.push(WatchAction::RestartResume);
            }
        }
        Some(WatchKind::QueuedIdle) => out.push(WatchAction::DeliverQueued),
        Some(WatchKind::ContextCritical) => out.push(WatchAction::Compact),
        None => {
            if obs.exited && restartable {
                out.push(WatchAction::RestartResume);
            } else if obs.working {
                out.push(WatchAction::Interrupt);
            } else if idle {
                if obs.queued > 0 {
                    out.push(WatchAction::DeliverQueued);
                }
                out.push(WatchAction::Compact);
            }
        }
    }
    if idle && has_prompt && !out.contains(&WatchAction::ReplayPrompt) {
        out.push(WatchAction::ReplayPrompt);
    }
    if flag.is_some() {
        out.push(WatchAction::Dismiss);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t() -> Thresholds {
        Thresholds::from_config(10.0, 30)
    }

    fn mins(n: i64) -> chrono::Duration {
        chrono::Duration::minutes(n)
    }

    fn working(now: DateTime<Utc>, hooks_ago: i64, pane_ago: i64) -> Observation {
        Observation {
            running: true,
            working: true,
            last_activity: Some(now - mins(hooks_ago)),
            pane_unchanged_since: Some(now - mins(pane_ago)),
            ..Default::default()
        }
    }

    #[test]
    fn stuck_needs_both_quiet_hooks_and_an_unchanged_pane() {
        let now = Utc::now();
        assert_eq!(
            detect(&working(now, 12, 11), &t(), now).map(|f| f.0),
            Some(WatchKind::Stuck)
        );
        // Output still moves (a long build): not stuck.
        assert_eq!(detect(&working(now, 30, 2), &t(), now), None);
        // Hooks still come (a chatty tool loop): not stuck.
        assert_eq!(detect(&working(now, 1, 30), &t(), now), None);
        // No hook ever seen: one signal only, not stuck.
        let mut obs = working(now, 30, 30);
        obs.last_activity = None;
        assert_eq!(detect(&obs, &t(), now), None);
        // No pane fingerprint yet: not stuck.
        let mut obs = working(now, 30, 30);
        obs.pane_unchanged_since = None;
        assert_eq!(detect(&obs, &t(), now), None);
    }

    #[test]
    fn a_session_waiting_on_the_user_is_never_stuck_queued_or_critical() {
        let now = Utc::now();
        let mut obs = working(now, 30, 30);
        obs.waiting_on_user = true;
        assert_eq!(detect(&obs, &t(), now), None);
        let obs = Observation {
            running: true,
            queued: 1,
            idle_since: Some(now - mins(5)),
            context_critical: Some(95),
            waiting_on_user: true,
            ..Default::default()
        };
        assert_eq!(detect(&obs, &t(), now), None);
        // An exit still counts: the question died with the agent.
        let obs = Observation {
            exited: true,
            waiting_on_user: true,
            ..Default::default()
        };
        assert_eq!(
            detect(&obs, &t(), now).map(|f| f.0),
            Some(WatchKind::Exited)
        );
    }

    #[test]
    fn running_subagents_suppress_stuck() {
        let now = Utc::now();
        let mut obs = working(now, 30, 30);
        obs.subagents = 2;
        assert_eq!(detect(&obs, &t(), now), None);
    }

    #[test]
    fn the_stuck_detail_counts_from_the_newer_signal() {
        let now = Utc::now();
        let (_, detail) = detect(&working(now, 25, 12), &t(), now).unwrap();
        assert_eq!(detail, "no hooks or output for 12m");
    }

    #[test]
    fn exited_and_failed_win_over_everything_else() {
        let now = Utc::now();
        let mut obs = working(now, 30, 30);
        obs.exited = true;
        assert_eq!(
            detect(&obs, &t(), now).map(|f| f.0),
            Some(WatchKind::Exited)
        );
        let obs = Observation {
            background_failed: Some(String::new()),
            ..Default::default()
        };
        assert_eq!(
            detect(&obs, &t(), now),
            Some((WatchKind::BackgroundFailed, "error".into()))
        );
    }

    #[test]
    fn queued_items_flag_an_idle_session_after_the_grace() {
        let now = Utc::now();
        let mut obs = Observation {
            running: true,
            queued: 2,
            idle_since: Some(now - chrono::Duration::seconds(10)),
            ..Default::default()
        };
        assert_eq!(detect(&obs, &t(), now), None);
        obs.idle_since = Some(now - chrono::Duration::seconds(45));
        assert_eq!(
            detect(&obs, &t(), now),
            Some((WatchKind::QueuedIdle, "2 queued".into()))
        );
        // A working session delivers at its Stop: nothing to flag.
        obs.working = true;
        obs.last_activity = Some(now);
        assert_eq!(detect(&obs, &t(), now), None);
        // A stopped one is not waited on.
        let obs = Observation {
            running: false,
            queued: 1,
            idle_since: Some(now - mins(5)),
            ..Default::default()
        };
        assert_eq!(detect(&obs, &t(), now), None);
    }

    #[test]
    fn context_critical_counts_only_at_the_prompt() {
        let now = Utc::now();
        let mut obs = Observation {
            running: true,
            context_critical: Some(91),
            ..Default::default()
        };
        assert_eq!(
            detect(&obs, &t(), now),
            Some((WatchKind::ContextCritical, "91%".into()))
        );
        obs.working = true;
        obs.last_activity = Some(now);
        assert_eq!(detect(&obs, &t(), now), None);
    }

    #[test]
    fn the_fingerprint_ignores_spinners_timers_and_token_counts() {
        let a = "⏺ Bash(cargo test)\n\x1b[2m✻ Thinking… (12s · ↑ 3.2k tokens · esc to interrupt)\x1b[0m\n";
        let b = "⏺ Bash(cargo test)\n\x1b[2m✽ Thinking… (47s · ↑ 3.9k tokens · esc to interrupt)\x1b[0m  \n";
        assert_eq!(pane_fingerprint(a), pane_fingerprint(b));
        let codex_a = "⠋ Working (3s • esc to interrupt)";
        let codex_b = "⠹ Working (95s • esc to interrupt)";
        assert_eq!(pane_fingerprint(codex_a), pane_fingerprint(codex_b));
        let moved = "⏺ Bash(cargo test)\n  ⎿ running 3 tests … ok\n";
        assert_ne!(pane_fingerprint(a), pane_fingerprint(moved));
        // Progress outside the status line keeps its digits: it is output.
        assert_ne!(
            pane_fingerprint("Compiling 12/400 crates"),
            pane_fingerprint("Compiling 13/400 crates")
        );
    }

    #[test]
    fn thresholds_accept_fractions_and_reject_nonsense() {
        let t = Thresholds::from_config(0.1, 5);
        assert_eq!(t.stuck, chrono::Duration::seconds(6));
        assert_eq!(t.queued_idle, chrono::Duration::seconds(5));
        assert_eq!(
            Thresholds::from_config(f64::NAN, 0).stuck,
            chrono::Duration::minutes(10)
        );
        assert_eq!(Thresholds::from_config(-1.0, 0).stuck, mins(10));
        assert_eq!(Thresholds::from_config(0.1, 5).snooze(), mins(1));
        assert_eq!(capture_interval(&t), Duration::from_secs(2));
        assert_eq!(
            capture_interval(&Thresholds::from_config(10.0, 30)),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn automatic_restarts_are_at_most_hourly() {
        let now = Utc::now();
        assert!(auto_restart_allowed(None, now));
        assert!(!auto_restart_allowed(Some(now - mins(59)), now));
        assert!(auto_restart_allowed(Some(now - mins(60)), now));
    }

    #[test]
    fn actions_follow_the_flag_and_the_state() {
        let idle = Observation {
            running: true,
            ..Default::default()
        };
        assert_eq!(
            actions_for(Some(WatchKind::QueuedIdle), &idle, true, true),
            vec![
                WatchAction::DeliverQueued,
                WatchAction::ReplayPrompt,
                WatchAction::Dismiss
            ]
        );
        let stuck = Observation {
            running: true,
            working: true,
            ..Default::default()
        };
        assert_eq!(
            actions_for(Some(WatchKind::Stuck), &stuck, true, true),
            vec![
                WatchAction::Interrupt,
                WatchAction::RestartResume,
                WatchAction::Dismiss
            ]
        );
        let exited = Observation {
            exited: true,
            ..Default::default()
        };
        assert_eq!(
            actions_for(Some(WatchKind::Exited), &exited, true, true),
            vec![WatchAction::RestartResume, WatchAction::Dismiss]
        );
        // Unflagged and idle: the manual tools, no dismiss.
        assert_eq!(
            actions_for(None, &idle, true, false),
            vec![WatchAction::Compact]
        );
    }

    #[test]
    fn reasons_end_with_the_key() {
        let flag = WatchFlag {
            kind: WatchKind::Stuck,
            since: Utc::now(),
            detail: "no hooks or output for 12m".into(),
        };
        assert_eq!(
            flag.reason(),
            "stuck · no hooks or output for 12m · ! recover"
        );
        let flag = WatchFlag {
            kind: WatchKind::QueuedIdle,
            since: Utc::now(),
            detail: "1 queued".into(),
        };
        assert_eq!(flag.reason(), "queued for an idle agent · ! recover");
    }
}
