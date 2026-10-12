//! FORK: the one badge model (DESIGN.md §15) — what a session is doing, derived
//! from state the app already holds, and shown the same way everywhere: the
//! tree rows, the workspace sidebar, split-view tile titles, the needs-you
//! picker and the header pills.
//!
//! Nothing here touches tmux or the filesystem. A badge is derived from the
//! attention records, the thinking sets, the stopped/exited markers and the
//! session's own status text; counts are rolled up per feature so the header
//! pills, the project rows and the sidebar can never disagree.

use chrono::{DateTime, Utc};
use ratatui::style::{Color, Modifier, Style};
use std::collections::HashMap;

use crate::app::App;
use crate::app::attention::{AttentionRecord, AttentionState};
use crate::project::{AgentKind, Feature, FeatureSession, ProjectStatus, SessionKind};
use crate::theme::Theme;

/// What a session is doing, strongest first: the order is the roll-up order
/// (a feature shows its most pressing session) and the sort order of the
/// workspace's "next needing you".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BadgeKind {
    /// Permission prompt, question, or a stop nothing explained.
    NeedsYou,
    /// The agent is thinking or running a tool.
    Working,
    /// Finished a turn and nobody has looked at it yet.
    FinishedUnread,
    /// The agent exited at its shell (the window is still there).
    Exited,
    /// Finished and already looked at, or a feature marked ready.
    FinishedRead,
    /// Running, nothing to report.
    Idle,
    /// Stopped (window gone or feature stopped).
    Stopped,
}

impl BadgeKind {
    /// Stronger kinds roll up over weaker ones.
    fn rank(self) -> u8 {
        match self {
            BadgeKind::NeedsYou => 6,
            BadgeKind::Working => 5,
            BadgeKind::FinishedUnread => 4,
            BadgeKind::Exited => 3,
            BadgeKind::FinishedRead => 2,
            BadgeKind::Idle => 1,
            BadgeKind::Stopped => 0,
        }
    }

    pub fn glyph(self) -> &'static str {
        match self {
            BadgeKind::NeedsYou => "⛔",
            BadgeKind::Working => "●",
            BadgeKind::FinishedUnread | BadgeKind::FinishedRead => "✓",
            BadgeKind::Idle => "○",
            BadgeKind::Stopped | BadgeKind::Exited => "■",
        }
    }

    /// The theme colour of the glyph — never hard-coded.
    pub fn tone(self, theme: &Theme) -> Color {
        match self {
            BadgeKind::NeedsYou => theme.status_waiting.to_color(),
            BadgeKind::Working => theme.status_working.to_color(),
            BadgeKind::FinishedUnread => theme.success.to_color(),
            BadgeKind::FinishedRead => theme.text_muted.to_color(),
            BadgeKind::Idle => theme.status_idle.to_color(),
            BadgeKind::Stopped => theme.status_stopped.to_color(),
            BadgeKind::Exited => theme.danger.to_color(),
        }
    }

    /// The glyph's style everywhere it is drawn: its tone, bold when it
    /// asks for a look (needs you, finished and not yet read).
    pub fn style(self, theme: &Theme) -> Style {
        let style = Style::default().fg(self.tone(theme));
        if matches!(self, BadgeKind::NeedsYou | BadgeKind::FinishedUnread) {
            style.add_modifier(Modifier::BOLD)
        } else {
            style
        }
    }

    pub fn needs_you(self) -> bool {
        self == BadgeKind::NeedsYou
    }

    /// The kind a timestamp is kept for: a finished turn keeps its age when
    /// it is read.
    fn age_family(self) -> BadgeKind {
        match self {
            BadgeKind::FinishedRead => BadgeKind::FinishedUnread,
            other => other,
        }
    }
}

/// One session's badge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionBadge {
    pub kind: BadgeKind,
    /// When the state began.
    pub since: DateTime<Utc>,
    /// The reason or activity text behind the glyph, when something said it:
    /// the permission subject, the question, the status line of a working
    /// agent. Never invented.
    pub detail: Option<String>,
    /// Context used, as a percentage of the window, when the harness or the
    /// collector reported it (P4).
    pub context: Option<u8>,
    /// Subagents the session has running (P5 hooks; cleared at the turn's end).
    pub subagents: usize,
    /// Follow-ups queued for the session (P7).
    pub queued: usize,
    /// `tasks 2/5` while the session's task list has open items (§19.1).
    pub tasks: Option<String>,
    /// `1 bg · 1 loop`: background tasks and schedules alive at the last
    /// `Stop` (§19.1).
    pub background: Option<String>,
    /// A recent one-off event: `compacted`, `model → sonnet`, `idle` (§19.1).
    pub event: Option<String>,
    /// FORK: §19.7 — the session runs under Claude's supervisor
    /// (`claude --bg`); its label starts with `bg`.
    pub in_background: bool,
}

impl SessionBadge {
    pub fn new(kind: BadgeKind, since: DateTime<Utc>, detail: Option<String>) -> Self {
        Self {
            kind,
            since,
            detail,
            context: None,
            subagents: 0,
            queued: 0,
            tasks: None,
            background: None,
            event: None,
            in_background: false,
        }
    }

    pub fn glyph(&self) -> &'static str {
        self.kind.glyph()
    }

    pub fn tone(&self, theme: &Theme) -> Color {
        self.kind.tone(theme)
    }

    pub fn style(&self, theme: &Theme) -> Style {
        self.kind.style(theme)
    }

    /// Compact elapsed time since the state began: `0:40`, `6m`, `2h`.
    pub fn age(&self, now: DateTime<Utc>) -> String {
        compact_elapsed((now - self.since).num_seconds())
    }

    /// Short reason text for a second sidebar line or a tile title:
    /// `working 6m · 2 subagents · 42% ctx · Edit src/auth.rs · ⏭ 2 queued`.
    pub fn label(&self, now: DateTime<Utc>) -> String {
        self.label_with_age(Some(now))
    }

    /// [`Self::label`] without the age — for the sidebar's reason line, whose
    /// row already shows the age at its right edge and has little room.
    pub fn label_compact(&self) -> String {
        self.label_with_age(None)
    }

    fn label_with_age(&self, now: Option<DateTime<Utc>>) -> String {
        let age = |text: &str| match now {
            Some(now) => format!("{text} {}", self.age(now)),
            None => text.to_string(),
        };
        let mut label = match self.kind {
            BadgeKind::NeedsYou => self.detail.clone().unwrap_or_else(|| "needs you".into()),
            BadgeKind::Working => {
                let mut parts = vec![age("working")];
                match self.subagents {
                    0 => {}
                    1 => parts.push("1 subagent".into()),
                    n => parts.push(format!("{n} subagents")),
                }
                if let Some(pct) = self.context {
                    parts.push(format!("{pct}% ctx"));
                }
                parts.extend(self.tasks.clone());
                parts.extend(self.background.clone());
                parts.extend(self.event.clone());
                if let Some(detail) = &self.detail {
                    parts.push(detail.clone());
                }
                parts.join(" · ")
            }
            BadgeKind::FinishedUnread | BadgeKind::FinishedRead => {
                let mut parts = vec![match now {
                    Some(now) => format!("done · {}", self.age(now)),
                    None => "done".into(),
                }];
                parts.extend(self.background.clone());
                parts.extend(self.event.clone());
                parts.join(" · ")
            }
            BadgeKind::Exited => {
                let mut parts = vec!["exited".to_string()];
                parts.extend(self.background.clone());
                // FORK: §19.8 — the watchdog's hint.
                parts.extend(self.detail.clone());
                parts.join(" · ")
            }
            // A background session says why: stopped (attach resumes it)
            // or not listed by Claude any more (`c` resumes it).
            BadgeKind::Stopped => self
                .detail
                .clone()
                .filter(|_| self.in_background)
                .unwrap_or_else(|| "stopped".into()),
            BadgeKind::Idle => {
                let mut parts: Vec<String> = Vec::new();
                parts.extend(self.detail.clone());
                parts.extend(self.event.clone());
                if parts.is_empty() {
                    parts.push("idle".into());
                }
                parts.extend(self.background.clone());
                parts.join(" · ")
            }
        };
        if self.queued > 0 && !matches!(self.kind, BadgeKind::Stopped | BadgeKind::Exited) {
            label.push_str(&format!(" · ⏭ {} queued", self.queued));
        }
        if self.in_background {
            label = format!("bg · {label}");
        }
        label
    }

    fn stronger(self, other: SessionBadge) -> SessionBadge {
        if other.kind.rank() > self.kind.rank() {
            other
        } else {
            self
        }
    }
}

/// `0:40` under a minute, `6m` under an hour, `2h` after that.
pub fn compact_elapsed(secs: i64) -> String {
    let secs = secs.max(0);
    if secs < 60 {
        format!("0:{secs:02}")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 48 * 3600 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}

/// `⛔ needs you · ● working · ✓ finished`, counted per feature from the
/// roll-up badges.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BadgeCounts {
    pub needs_you: usize,
    pub working: usize,
    pub done: usize,
}

impl BadgeCounts {
    fn add(&mut self, kind: BadgeKind) {
        match kind {
            BadgeKind::NeedsYou => self.needs_you += 1,
            BadgeKind::Working => self.working += 1,
            BadgeKind::FinishedUnread | BadgeKind::FinishedRead => self.done += 1,
            BadgeKind::Exited | BadgeKind::Idle | BadgeKind::Stopped => {}
        }
    }
}

/// When each session's current state began, keyed by session id. Kept on
/// `App` and refreshed once per main-loop iteration
/// ([`App::refresh_badge_ages`]); the pure derivation reads it so a frame
/// never mutates anything.
#[derive(Debug, Default)]
pub struct BadgeAges {
    since: HashMap<String, (BadgeKind, DateTime<Utc>)>,
}

impl BadgeAges {
    fn lookup(&self, session_id: &str, kind: BadgeKind) -> Option<DateTime<Utc>> {
        self.since
            .get(session_id)
            .filter(|(seen, _)| seen.age_family() == kind.age_family())
            .map(|(_, since)| *since)
    }
}

/// The tidy subject of a permission prompt, out of the harness's own words:
/// `Claude needs your permission to use Bash` → `Bash`.
pub fn permission_subject(message: &str) -> Option<String> {
    const PREFIXES: &[&str] = &[
        " needs your permission to use ",
        " needs your permission to ",
        " needs approval to run ",
        " needs approval to ",
        " needs your approval to ",
        " requests permission to ",
    ];
    for prefix in PREFIXES {
        if let Some(index) = message.find(prefix) {
            let subject = message[index + prefix.len()..].trim().trim_end_matches('.');
            if !subject.is_empty() {
                return Some(subject.to_string());
            }
        }
    }
    None
}

/// Why an attention record needs you, in a few words.
fn attention_detail(record: &AttentionRecord) -> Option<String> {
    let detail = record
        .detail
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty());
    let is_permission = record
        .notification_type
        .as_deref()
        .map(str::to_ascii_lowercase)
        .is_some_and(|t| t.contains("permission") || t.contains("approval"))
        || detail.is_some_and(|d| {
            let lower = d.to_ascii_lowercase();
            lower.contains("permission") || lower.contains("approval")
        });
    match record.state {
        AttentionState::Question if is_permission => Some(format!(
            "permission: {}",
            detail
                .and_then(permission_subject)
                .or_else(|| detail.map(str::to_string))
                .unwrap_or_default()
        )),
        AttentionState::Question => Some(match detail {
            Some(detail) => format!("question: {detail}"),
            None => "question".to_string(),
        }),
        AttentionState::Waiting => Some(match detail {
            Some(detail) => format!("waiting: {detail}"),
            None => "waiting for you".to_string(),
        }),
        AttentionState::CompletedAwaitingReview => None,
    }
}

fn session_kind_for_agent(agent: &AgentKind) -> SessionKind {
    match agent {
        AgentKind::Claude => SessionKind::Claude,
        AgentKind::Codex => SessionKind::Codex,
        AgentKind::Pi => SessionKind::Pi,
        AgentKind::Gemini => SessionKind::Gemini,
        AgentKind::Cursor => SessionKind::Cursor,
    }
}

/// Whether a feature-level signal raised by `agent` belongs to `session`:
/// the session of that harness when the feature lists one (even stopped or
/// exited — the signal stays with the agent that raised it), every agent
/// session otherwise.
pub(crate) fn signal_applies(
    feature: &Feature,
    agent: Option<&AgentKind>,
    session: &FeatureSession,
) -> bool {
    if !session.kind.is_agent_harness() {
        return false;
    }
    let Some(agent) = agent else {
        return true;
    };
    let kind = session_kind_for_agent(agent);
    if session.kind == kind {
        return true;
    }
    !feature.sessions.iter().any(|s| s.kind == kind)
}

/// A pending input nothing explained. A generic session wait is the same
/// stop the feature's attention record describes (the needs-you list folds
/// the two into one row), so it adds nothing while a record exists.
fn feature_pending_detail(app: &App, feature: &Feature) -> Option<String> {
    if !app.is_feature_waiting_for_input(&feature.name) {
        return None;
    }
    let explained = app.attention_for(&feature.tmux_session).is_some();
    let input = app.pending_inputs.iter().find(|input| {
        input.feature_name.as_deref() == Some(feature.name.as_str())
            && !(explained && input.is_session_wait())
    })?;
    let message = input.message.trim();
    let kind = match input.notification_type.as_str() {
        "input-request" | "stop" => "input",
        "diff-review" => "diff review",
        "change-reason" => "change reason",
        "review-ready" => "review ready",
        other => other,
    };
    Some(if message.is_empty() {
        kind.to_string()
    } else {
        let mut shown: String = message.chars().take(80).collect();
        if shown.len() < message.len() {
            shown.push('…');
        }
        format!("{kind}: {shown}")
    })
}

/// Whether `session` is working: its own IPC marker when the harness sends
/// one, else the feature's thinking marker shared by its agent sessions.
pub(crate) fn session_working(app: &App, feature: &Feature, session: &FeatureSession) -> bool {
    if !session.kind.is_agent_harness() {
        return false;
    }
    let own = |s: &FeatureSession| {
        app.ipc_thinking_feature_sessions.contains(&s.id)
            || app.ipc_tool_feature_sessions.contains(&s.id)
    };
    if own(session) {
        return true;
    }
    app.thinking_features.contains(&feature.tmux_session) && !feature.sessions.iter().any(own)
}

/// FORK: §19.7 — the badge of a background Claude session.
fn background_badge(
    app: &App,
    feature: &Feature,
    session: &FeatureSession,
    badge: &dyn Fn(BadgeKind, Option<String>) -> SessionBadge,
) -> SessionBadge {
    use crate::fork::background::BgState;
    let state = app.background_state(&session.id);
    // Hooks are faster than the roster: a turn that just started, or a
    // question just asked, shows at once.
    let hook_working = session_working(app, feature, session);
    let hook_question = app.attention_for(&feature.tmux_session).filter(|record| {
        signal_applies(feature, Some(&record.agent), session)
            && matches!(
                record.state,
                AttentionState::Question | AttentionState::Waiting
            )
    });
    let mut out = match state {
        _ if hook_question.is_some() => {
            let record = hook_question.expect("checked");
            SessionBadge::new(BadgeKind::NeedsYou, record.since, attention_detail(record))
        }
        BgState::Blocked(why) => badge(BadgeKind::NeedsYou, Some(why)),
        BgState::Failed(why) => badge(
            BadgeKind::NeedsYou,
            Some(if why.is_empty() {
                "failed".to_string()
            } else {
                format!("failed: {why}")
            }),
        ),
        BgState::Working => badge(BadgeKind::Working, None),
        BgState::Starting => badge(BadgeKind::Working, Some("starting".into())),
        _ if hook_working => badge(BadgeKind::Working, None),
        BgState::Done => badge(
            if app.finished_seen.contains(&session.id) {
                BadgeKind::FinishedRead
            } else {
                BadgeKind::FinishedUnread
            },
            None,
        ),
        BgState::Stopped => badge(BadgeKind::Stopped, Some("stopped · Enter resumes".into())),
        BgState::Gone => badge(BadgeKind::Stopped, Some("not running · c resumes".into())),
    };
    out.in_background = true;
    out
}

/// The badge of one session.
pub fn session_badge(
    app: &App,
    pi: usize,
    fi: usize,
    si: usize,
    now: DateTime<Utc>,
) -> SessionBadge {
    let feature = &app.store.projects[pi].features[fi];
    let session = &feature.sessions[si];
    let fallback_since = |kind: BadgeKind| app.badge_ages.lookup(&session.id, kind).unwrap_or(now);
    let badge = |kind: BadgeKind, detail: Option<String>| {
        SessionBadge::new(kind, fallback_since(kind), detail)
    };
    // What the session reported about itself (P4 context, P5 subagents) and
    // what the user queued for it (P7) decorate whatever state it is in.
    let hooks = app.hook_state(&session.id);
    let decorate = |mut badge: SessionBadge| {
        badge.context = app.session_context_percent(&session.id).map(|(pct, _)| pct);
        badge.queued = app.queued_followup_count(&session.id);
        badge.subagents = app.subagent_count(&session.id);
        // FORK: §19.1 — what the newer hooks reported.
        if let Some(state) = hooks {
            badge.tasks = state.tasks_text();
            badge.background = state.background_text();
            badge.event = state.event_text(now);
        }
        badge
    };

    if !session.kind.is_tmux_backed() {
        return badge(BadgeKind::Idle, None);
    }
    // FORK: §19.7 — a background session lives outside the feature's tmux
    // session: its state comes from Claude's roster and its own hooks, not
    // from the feature running or a window existing.
    if session.is_background() {
        return decorate(background_badge(app, feature, session, &badge));
    }
    let feature_stopped = feature.status == ProjectStatus::Stopped;
    if feature_stopped || session.stopped {
        let exited = !feature_stopped
            && (app.session_is_exited(&session.id)
                || session.status_text.as_deref()
                    == Some(crate::fork::session_reconcile::EXITED_STATUS));
        return if exited {
            // FORK: §19.8 — the watchdog offers the restart.
            badge(
                BadgeKind::Exited,
                app.watchdog_flag(&session.id)
                    .filter(|f| f.kind == crate::fork::watchdog::WatchKind::Exited)
                    .map(|_| "! recover".to_string()),
            )
        } else {
            badge(BadgeKind::Stopped, None)
        };
    }
    if !session.kind.is_agent_harness() {
        return badge(BadgeKind::Idle, Some("open".into()));
    }

    // FORK: §19.1 — a turn that died on an API error waits for the user. It
    // outranks the previous turn's "done": the failing turn is newer, and a
    // new turn (`thinking-start`) retires it.
    if let Some((reason, at)) = hooks.and_then(|state| state.turn_failed.clone()) {
        return decorate(SessionBadge::new(
            BadgeKind::NeedsYou,
            at,
            Some(format!("turn failed: {reason}")),
        ));
    }
    // FORK: §19.8 — a stuck agent being stopped for its relaunch.
    if app.watchdog_restart_pending(&session.id) {
        return decorate(badge(BadgeKind::Working, Some("restarting…".into())));
    }
    // FORK: §19.8 — the watchdog: a stuck turn or items queued for an idle
    // agent wait on the user, whatever else the row would say.
    if let Some(flag) = app.watchdog_flag(&session.id).filter(|f| {
        matches!(
            f.kind,
            crate::fork::watchdog::WatchKind::Stuck | crate::fork::watchdog::WatchKind::QueuedIdle
        )
    }) {
        return decorate(SessionBadge::new(
            BadgeKind::NeedsYou,
            flag.since,
            Some(flag.reason()),
        ));
    }
    // FORK: §19.2 — Claude's roster says the session waits on input that
    // its hooks did not report (an older hook set, a dialog hooks miss). A
    // hook-reported question keeps its own detail; a finished-turn record is
    // older news than a session that waits now.
    let roster_waiting = app.roster_waiting_for(&session.id);
    if let Some(record) = app.attention_for(&feature.tmux_session)
        && signal_applies(feature, Some(&record.agent), session)
        && !(roster_waiting.is_some()
            && matches!(record.state, AttentionState::CompletedAwaitingReview))
    {
        return decorate(match record.state {
            AttentionState::Question | AttentionState::Waiting => {
                SessionBadge::new(BadgeKind::NeedsYou, record.since, attention_detail(record))
            }
            AttentionState::CompletedAwaitingReview => SessionBadge::new(
                if app.finished_seen.contains(&session.id) {
                    BadgeKind::FinishedRead
                } else {
                    BadgeKind::FinishedUnread
                },
                record.since,
                None,
            ),
        });
    }
    if let Some(waiting) = roster_waiting {
        return decorate(badge(BadgeKind::NeedsYou, Some(waiting.to_string())));
    }
    if session_working(app, feature, session) {
        return decorate(badge(BadgeKind::Working, session.status_text.clone()));
    }
    if let Some(detail) = feature_pending_detail(app, feature)
        && signal_applies(feature, None, session)
    {
        return decorate(badge(BadgeKind::NeedsYou, Some(detail)));
    }
    // FORK: §19.8 — idle at the prompt past the context critical threshold.
    if let Some(flag) = app
        .watchdog_flag(&session.id)
        .filter(|f| f.kind == crate::fork::watchdog::WatchKind::ContextCritical)
    {
        return decorate(SessionBadge::new(
            BadgeKind::NeedsYou,
            flag.since,
            Some(flag.reason()),
        ));
    }
    decorate(badge(BadgeKind::Idle, None))
}

/// A feature's roll-up: its most pressing agent session, or the feature's
/// own signals when no session carries them (an attention record on a
/// feature whose agent window is gone, a feature marked ready).
pub fn feature_badge(app: &App, pi: usize, fi: usize, now: DateTime<Utc>) -> SessionBadge {
    let feature = &app.store.projects[pi].features[fi];
    let stopped = SessionBadge::new(BadgeKind::Stopped, feature.last_accessed, None);
    if feature.status == ProjectStatus::Stopped {
        // FORK: §19.7 — background sessions run on without the feature's
        // tmux session; the feature shows the most pressing of them.
        let mut best = stopped;
        for si in 0..feature.sessions.len() {
            if feature.sessions[si].is_background() {
                best = best.stronger(session_badge(app, pi, fi, si, now));
            }
        }
        return best;
    }
    let mut best = SessionBadge::new(BadgeKind::Idle, feature.last_accessed, None);
    let mut any_agent = false;
    for si in 0..feature.sessions.len() {
        if !feature.sessions[si].kind.is_agent_harness() {
            continue;
        }
        any_agent = true;
        best = best.stronger(session_badge(app, pi, fi, si, now));
    }
    if !any_agent {
        // Feature-level signals with nothing to hang them on.
        if let Some(record) = app.attention_for(&feature.tmux_session) {
            let kind = match record.state {
                AttentionState::Question | AttentionState::Waiting => BadgeKind::NeedsYou,
                AttentionState::CompletedAwaitingReview => BadgeKind::FinishedRead,
            };
            best = best.stronger(SessionBadge::new(
                kind,
                record.since,
                attention_detail(record),
            ));
        } else if app.thinking_features.contains(&feature.tmux_session) {
            best = best.stronger(SessionBadge::new(BadgeKind::Working, now, None));
        } else if let Some(detail) = feature_pending_detail(app, feature) {
            best = best.stronger(SessionBadge::new(BadgeKind::NeedsYou, now, Some(detail)));
        }
    }
    if feature.ready {
        best = best.stronger(SessionBadge::new(
            BadgeKind::FinishedRead,
            feature.last_accessed,
            Some("ready".into()),
        ));
    }
    best
}

/// The counts for one project's row.
pub fn project_counts(app: &App, pi: usize, now: DateTime<Utc>) -> BadgeCounts {
    let mut counts = BadgeCounts::default();
    for fi in 0..app.store.projects[pi].features.len() {
        counts.add(feature_badge(app, pi, fi, now).kind);
    }
    counts
}

/// The header pills: every project's counts summed.
pub fn total_counts(app: &App, now: DateTime<Utc>) -> BadgeCounts {
    let mut counts = BadgeCounts::default();
    for pi in 0..app.store.projects.len() {
        let project = project_counts(app, pi, now);
        counts.needs_you += project.needs_you;
        counts.working += project.working;
        counts.done += project.done;
    }
    counts
}

impl App {
    /// Record when each session's current badge state began. Called once
    /// per main-loop iteration; a state that is still the one recorded
    /// keeps its timestamp, so ages measure the state, not the sweep.
    pub fn refresh_badge_ages(&mut self) {
        let now = Utc::now();
        let mut updates: Vec<(String, BadgeKind)> = Vec::new();
        let mut live: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for (pi, project) in self.store.projects.iter().enumerate() {
            for (fi, feature) in project.features.iter().enumerate() {
                for (si, session) in feature.sessions.iter().enumerate() {
                    live.insert(session.id.as_str());
                    let kind = session_badge(self, pi, fi, si, now).kind;
                    let recorded = self
                        .badge_ages
                        .since
                        .get(&session.id)
                        .map(|(seen, _)| seen.age_family());
                    if recorded != Some(kind.age_family()) {
                        updates.push((session.id.clone(), kind));
                    }
                }
            }
        }
        if self.badge_ages.since.len() > live.len() {
            let live: std::collections::HashSet<String> =
                live.into_iter().map(str::to_string).collect();
            self.badge_ages.since.retain(|id, _| live.contains(id));
        }
        for (id, kind) in updates {
            self.badge_ages.since.insert(id, (kind, now));
        }
    }

    /// A finished turn was looked at: its `✓` goes dim.
    pub fn mark_finished_seen(&mut self, session_id: &str) {
        if !self.finished_seen.contains(session_id) {
            self.finished_seen.insert(session_id.to_string());
        }
    }

    /// A feature finished a turn again: its sessions' `✓` are new.
    pub(crate) fn forget_finished_seen_for(&mut self, tmux_session: &str) {
        if self.finished_seen.is_empty() {
            return;
        }
        let ids: Vec<String> = self
            .store
            .projects
            .iter()
            .flat_map(|p| p.features.iter())
            .filter(|f| f.tmux_session == tmux_session)
            .flat_map(|f| f.sessions.iter().map(|s| s.id.clone()))
            .collect();
        for id in ids {
            self.finished_seen.remove(&id);
        }
    }

    /// The badge of the session at `(pi, fi, si)` now.
    pub fn session_badge_at(&self, pi: usize, fi: usize, si: usize) -> Option<SessionBadge> {
        let feature = self.store.projects.get(pi)?.features.get(fi)?;
        feature.sessions.get(si)?;
        Some(session_badge(self, pi, fi, si, Utc::now()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::attention::AttentionSource;
    use crate::project::{Project, ProjectStore, VibeMode};
    use crate::traits::{MockTmuxOps, MockWorktreeOps};
    use chrono::Duration as ChronoDuration;
    use std::path::PathBuf;

    fn app_with(kinds: &[SessionKind]) -> App {
        let mut feature = Feature::new(
            "hx-12".into(),
            "hx-12".into(),
            PathBuf::from("/tmp"),
            false,
            VibeMode::Vibeless,
            AgentKind::Claude,
            false,
        );
        feature.status = ProjectStatus::Active;
        for kind in kinds {
            feature.add_session(kind.clone());
        }
        let mut project = Project::new("RS".into(), PathBuf::from("/tmp"), true, AgentKind::Claude);
        project.features.push(feature);
        let store = ProjectStore {
            version: 5,
            projects: vec![project],
            session_bookmarks: vec![],
            available_harnesses: vec![],
            prompt_templates: Vec::new(),
            extra: Default::default(),
        };
        App::new_for_test(
            store,
            Box::new(MockTmuxOps::new()),
            Box::new(MockWorktreeOps::new()),
        )
    }

    fn id(app: &App, si: usize) -> String {
        app.store.projects[0].features[0].sessions[si].id.clone()
    }

    #[test]
    fn every_state_has_its_glyph_and_label() {
        let now = Utc::now();
        type Setup = Box<dyn Fn(&mut App)>;
        // (name, setup, expected kind, glyph, label)
        let cases: Vec<(&str, Setup, BadgeKind, &str, &str)> = vec![
            ("idle", Box::new(|_| {}), BadgeKind::Idle, "○", "idle"),
            (
                "working",
                Box::new(|app| {
                    app.thinking_features.insert("aiw-hx-12".into());
                }),
                BadgeKind::Working,
                "●",
                "working 0:00",
            ),
            (
                "working with activity",
                Box::new(|app| {
                    app.thinking_features.insert("aiw-hx-12".into());
                    app.store.projects[0].features[0].sessions[0].status_text =
                        Some("Edit src/auth.rs".into());
                }),
                BadgeKind::Working,
                "●",
                "working 0:00 · Edit src/auth.rs",
            ),
            (
                "permission",
                Box::new(|app| {
                    app.record_attention_with_detail(
                        "aiw-hx-12",
                        &AgentKind::Claude,
                        AttentionState::Question,
                        AttentionSource::Hook,
                        Some("permission_prompt".into()),
                        Some("Claude needs your permission to use Bash".into()),
                    );
                }),
                BadgeKind::NeedsYou,
                "⛔",
                "permission: Bash",
            ),
            (
                "question",
                Box::new(|app| {
                    app.record_attention(
                        "aiw-hx-12",
                        &AgentKind::Claude,
                        AttentionState::Question,
                        AttentionSource::Hook,
                    );
                }),
                BadgeKind::NeedsYou,
                "⛔",
                "question",
            ),
            (
                "unexplained pending input",
                Box::new(|app| {
                    app.pending_inputs.push(crate::app::PendingInput {
                        feature_name: Some("hx-12".into()),
                        project_name: Some("RS".into()),
                        notification_type: "input-request".into(),
                        message: "Waiting for input".into(),
                        ..Default::default()
                    });
                }),
                BadgeKind::NeedsYou,
                "⛔",
                "input: Waiting for input",
            ),
            (
                "finished, unread",
                Box::new(|app| {
                    app.record_attention(
                        "aiw-hx-12",
                        &AgentKind::Claude,
                        AttentionState::CompletedAwaitingReview,
                        AttentionSource::Hook,
                    );
                }),
                BadgeKind::FinishedUnread,
                "✓",
                "done · 0:00",
            ),
            (
                "finished, read",
                Box::new(|app| {
                    app.record_attention(
                        "aiw-hx-12",
                        &AgentKind::Claude,
                        AttentionState::CompletedAwaitingReview,
                        AttentionSource::Hook,
                    );
                    let id = id(app, 0);
                    app.mark_finished_seen(&id);
                }),
                BadgeKind::FinishedRead,
                "✓",
                "done · 0:00",
            ),
            (
                "stopped session",
                Box::new(|app| {
                    app.store.projects[0].features[0].sessions[0].stopped = true;
                }),
                BadgeKind::Stopped,
                "■",
                "stopped",
            ),
            (
                "stopped feature",
                Box::new(|app| {
                    app.store.projects[0].features[0].status = ProjectStatus::Stopped;
                }),
                BadgeKind::Stopped,
                "■",
                "stopped",
            ),
            (
                "exited at the shell",
                Box::new(|app| {
                    let id = id(app, 0);
                    app.store.projects[0].features[0].sessions[0].stopped = true;
                    app.exited_sessions.insert(id);
                }),
                BadgeKind::Exited,
                "■",
                "exited",
            ),
        ];
        for (name, setup, kind, glyph, label) in cases {
            let mut app = app_with(&[SessionKind::Claude]);
            setup(&mut app);
            let badge = session_badge(&app, 0, 0, 0, now);
            assert_eq!(badge.kind, kind, "{name}");
            assert_eq!(badge.glyph(), glyph, "{name}");
            assert_eq!(badge.label(now), label, "{name}");
            let theme = Theme::default();
            assert_eq!(badge.tone(&theme), kind.tone(&theme), "{name}");
        }
    }

    #[test]
    fn a_terminal_is_open_or_stopped_never_working() {
        let mut app = app_with(&[SessionKind::Terminal]);
        app.thinking_features.insert("aiw-hx-12".into());
        let now = Utc::now();
        let badge = session_badge(&app, 0, 0, 0, now);
        assert_eq!(badge.kind, BadgeKind::Idle);
        assert_eq!(badge.label(now), "open");
        app.store.projects[0].features[0].sessions[0].stopped = true;
        assert_eq!(session_badge(&app, 0, 0, 0, now).kind, BadgeKind::Stopped);
    }

    #[test]
    fn a_feature_signal_lands_on_the_session_of_its_harness() {
        let mut app = app_with(&[SessionKind::Claude, SessionKind::Codex]);
        app.record_attention(
            "aiw-hx-12",
            &AgentKind::Claude,
            AttentionState::Question,
            AttentionSource::Hook,
        );
        app.thinking_features.insert("aiw-hx-12".into());
        let now = Utc::now();
        assert_eq!(session_badge(&app, 0, 0, 0, now).kind, BadgeKind::NeedsYou);
        // The codex window shares the feature's thinking marker but not the
        // question Claude asked.
        assert_eq!(session_badge(&app, 0, 0, 1, now).kind, BadgeKind::Working);
        // A per-session IPC marker narrows "working" to that window.
        let codex_id = id(&app, 1);
        app.ipc_thinking_feature_sessions.insert(codex_id);
        app.attention.clear();
        assert_eq!(session_badge(&app, 0, 0, 0, now).kind, BadgeKind::Idle);
        assert_eq!(session_badge(&app, 0, 0, 1, now).kind, BadgeKind::Working);
    }

    #[test]
    fn a_signal_stays_with_the_agent_that_raised_it_even_after_it_exited() {
        let mut app = app_with(&[SessionKind::Claude, SessionKind::Codex]);
        let now = Utc::now();
        app.record_attention(
            "aiw-hx-12",
            &AgentKind::Claude,
            AttentionState::CompletedAwaitingReview,
            AttentionSource::Hook,
        );
        let claude_id = id(&app, 0);
        app.store.projects[0].features[0].sessions[0].stopped = true;
        app.exited_sessions.insert(claude_id);
        assert_eq!(session_badge(&app, 0, 0, 0, now).kind, BadgeKind::Exited);
        assert_eq!(
            session_badge(&app, 0, 0, 1, now).kind,
            BadgeKind::Idle,
            "Claude's finished turn does not move to the Codex window"
        );
        assert_eq!(feature_badge(&app, 0, 0, now).kind, BadgeKind::Exited);
        // Codex finishing a turn of its own is the feature's most pressing state.
        app.record_attention(
            "aiw-hx-12",
            &AgentKind::Codex,
            AttentionState::Waiting,
            AttentionSource::Hook,
        );
        assert_eq!(session_badge(&app, 0, 0, 1, now).kind, BadgeKind::NeedsYou);
        assert_eq!(feature_badge(&app, 0, 0, now).kind, BadgeKind::NeedsYou);
    }

    #[test]
    fn a_session_wait_explained_by_the_record_lights_no_other_session() {
        let mut app = app_with(&[SessionKind::Claude, SessionKind::Codex]);
        let now = Utc::now();
        // Claude's permission prompt arrives twice: as the attention record
        // and as the older generic "input-request" for the feature.
        app.record_attention(
            "aiw-hx-12",
            &AgentKind::Claude,
            AttentionState::Question,
            AttentionSource::Hook,
        );
        app.pending_inputs.push(crate::app::PendingInput {
            feature_name: Some("hx-12".into()),
            project_name: Some("RS".into()),
            notification_type: "input-request".into(),
            message: "Claude needs your permission to use Bash".into(),
            ..Default::default()
        });
        assert_eq!(session_badge(&app, 0, 0, 0, now).kind, BadgeKind::NeedsYou);
        assert_eq!(session_badge(&app, 0, 0, 1, now).kind, BadgeKind::Idle);
        // A diff review is separate work and still counts.
        app.pending_inputs.push(crate::app::PendingInput {
            feature_name: Some("hx-12".into()),
            project_name: Some("RS".into()),
            notification_type: "diff-review".into(),
            ..Default::default()
        });
        let codex = session_badge(&app, 0, 0, 1, now);
        assert_eq!(codex.kind, BadgeKind::NeedsYou);
        assert_eq!(codex.label(now), "diff review");
    }

    #[test]
    fn a_new_finished_turn_is_unread_again() {
        let mut app = app_with(&[SessionKind::Claude]);
        let now = Utc::now();
        app.record_attention(
            "aiw-hx-12",
            &AgentKind::Claude,
            AttentionState::CompletedAwaitingReview,
            AttentionSource::Hook,
        );
        let session_id = id(&app, 0);
        app.mark_finished_seen(&session_id);
        assert_eq!(
            session_badge(&app, 0, 0, 0, now).kind,
            BadgeKind::FinishedRead
        );
        // Working again, then finished again: the seen mark is gone.
        app.record_attention(
            "aiw-hx-12",
            &AgentKind::Claude,
            AttentionState::Question,
            AttentionSource::Hook,
        );
        app.record_attention(
            "aiw-hx-12",
            &AgentKind::Claude,
            AttentionState::CompletedAwaitingReview,
            AttentionSource::Hook,
        );
        assert_eq!(
            session_badge(&app, 0, 0, 0, now).kind,
            BadgeKind::FinishedUnread
        );
    }

    #[test]
    fn feature_rollup_takes_the_most_pressing_session_and_ready_counts_as_done() {
        let mut app = app_with(&[
            SessionKind::Claude,
            SessionKind::Codex,
            SessionKind::Terminal,
        ]);
        let now = Utc::now();
        assert_eq!(feature_badge(&app, 0, 0, now).kind, BadgeKind::Idle);
        app.store.projects[0].features[0].ready = true;
        assert_eq!(feature_badge(&app, 0, 0, now).kind, BadgeKind::FinishedRead);
        app.thinking_features.insert("aiw-hx-12".into());
        assert_eq!(feature_badge(&app, 0, 0, now).kind, BadgeKind::Working);
        app.record_attention(
            "aiw-hx-12",
            &AgentKind::Codex,
            AttentionState::Waiting,
            AttentionSource::Hook,
        );
        assert_eq!(feature_badge(&app, 0, 0, now).kind, BadgeKind::NeedsYou);
        app.store.projects[0].features[0].status = ProjectStatus::Stopped;
        assert_eq!(feature_badge(&app, 0, 0, now).kind, BadgeKind::Stopped);
        assert_eq!(project_counts(&app, 0, now), BadgeCounts::default());
    }

    #[test]
    fn ages_are_compact_and_stable_across_refreshes() {
        assert_eq!(compact_elapsed(40), "0:40");
        assert_eq!(compact_elapsed(6 * 60 + 5), "6m");
        assert_eq!(compact_elapsed(2 * 3600 + 120), "2h");
        assert_eq!(compact_elapsed(3 * 86400), "3d");
        assert_eq!(compact_elapsed(-5), "0:00");

        let mut app = app_with(&[SessionKind::Claude]);
        app.thinking_features.insert("aiw-hx-12".into());
        app.refresh_badge_ages();
        let started = session_badge(&app, 0, 0, 0, Utc::now()).since;
        app.refresh_badge_ages();
        assert_eq!(session_badge(&app, 0, 0, 0, Utc::now()).since, started);
        // The state changes: the clock restarts.
        app.thinking_features.clear();
        app.refresh_badge_ages();
        let idle_since = session_badge(&app, 0, 0, 0, Utc::now()).since;
        assert!(idle_since >= started);
        // An attention record carries its own timestamp.
        let since = Utc::now() - ChronoDuration::seconds(40);
        app.attention.insert(
            "aiw-hx-12".into(),
            AttentionRecord::new(AgentKind::Claude, AttentionState::Question, since),
        );
        let badge = session_badge(&app, 0, 0, 0, Utc::now());
        assert_eq!(badge.since, since);
        assert_eq!(badge.age(since + ChronoDuration::seconds(40)), "0:40");
    }

    #[test]
    fn permission_subjects_come_from_the_harness_wording() {
        assert_eq!(
            permission_subject("Claude needs your permission to use Bash").as_deref(),
            Some("Bash")
        );
        assert_eq!(
            permission_subject("Gemini needs approval to run cargo test").as_deref(),
            Some("cargo test")
        );
        assert_eq!(permission_subject("Permission needed"), None);
    }
}
