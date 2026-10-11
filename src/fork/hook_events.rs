//! FORK: the harness hook events AIW used to ignore, turned into per-session
//! row state (DESIGN.md §19.1).
//!
//! Claude Code reports far more than a turn starting and ending: a context
//! compaction, a model switch, a turn that died on an API error, the task list
//! growing and shrinking, the working directory moving, an auto-mode denial,
//! an idle nudge, a quota pause, and — on every `Stop` — the background tasks
//! and `/loop` schedules still alive in the session. Each of those runs
//! `scripts/hook-event.sh <kind> <harness>` → `aiw hook-event`, which sends a
//! small `hook-event` IPC message; [`HookEventState::apply`] folds it into
//! what the row and the info block show.
//!
//! Pure: no tmux, no files, no App. The app side lives in
//! `app/fork_hook_events.rs`.

use chrono::{DateTime, Duration, Utc};
use serde_json::Value;

/// The IPC message type `aiw hook-event` sends.
pub const HOOK_EVENT_TYPE: &str = "hook-event";

/// The hook script every new event runs.
pub const SCRIPT_NAME: &str = "hook-event.sh";
pub const SCRIPT: &str = include_str!("../../scripts/hook-event.sh");

/// How long a one-off event ("compacted", "model → sonnet") stays on the row
/// when no new turn retires it first.
pub const EVENT_TTL_SECS: i64 = 10 * 60;

/// Longest event text kept (a model name, an error, a tool).
const MAX_TEXT: usize = 48;

/// The hook kinds `aiw hook-event --kind` accepts, as the scripts pass them.
pub const KINDS: &[&str] = &[
    "notification",
    "stop",
    "stop-failure",
    "post-compact",
    "post-model-switch",
    "task-created",
    "task-completed",
    "cwd-changed",
    "permission-denied",
    "session-end",
];

/// Payload keys `aiw hook-event` forwards from the harness's hook JSON. Arrays
/// (`background_tasks`, `session_crons`) are sent as counts instead, so an IPC
/// line never carries a whole task list.
pub const FORWARDED_KEYS: &[&str] = &[
    "session_id",
    "conversation_id",
    "cwd",
    "new_cwd",
    "notification_type",
    "message",
    "trigger",
    "from_model",
    "to_model",
    "error",
    "error_type",
    "error_message",
    "reason",
    "task_id",
    "task_subject",
    "teammate_name",
    "tool_name",
    "agent_id",
];

/// The array fields a `Stop` payload carries, sent as `<key>_count`.
pub const COUNTED_KEYS: &[&str] = &["background_tasks", "session_crons"];

/// A one-off thing the session reported, shown on the row for a while.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RowEventKind {
    /// The context was compacted (`PostCompact`).
    Compacted,
    /// The model changed (`PostModelSwitch`), to the named model.
    ModelSwitched(String),
    /// An auto-mode classifier denied a tool call (`PermissionDenied`).
    Denied(String),
    /// Claude's 60-second idle nudge (`Notification` `idle_prompt`).
    Idle,
    /// A quota pause or auto-resume (`Notification` `quota_auto_resume_*`).
    Quota(&'static str),
}

impl RowEventKind {
    /// The short text the row shows.
    pub fn text(&self) -> String {
        match self {
            RowEventKind::Compacted => "compacted".into(),
            RowEventKind::ModelSwitched(model) => format!("model → {model}"),
            RowEventKind::Denied(tool) => format!("denied: {tool}"),
            RowEventKind::Idle => "idle".into(),
            RowEventKind::Quota(what) => format!("quota: {what}"),
        }
    }

    /// Whether a new turn retires it. A model switch stays: it is still true.
    fn retired_by_new_turn(&self) -> bool {
        !matches!(self, RowEventKind::ModelSwitched(_))
    }
}

/// A [`RowEventKind`] and when it happened.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RowEvent {
    pub kind: RowEventKind,
    pub at: DateTime<Utc>,
}

/// What the hooks have told AIW about one session.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct HookEventState {
    /// The latest one-off event, when it is still worth showing.
    pub event: Option<RowEvent>,
    /// The turn ended on an API error (`StopFailure`); the text says why.
    /// Cleared when the next turn starts.
    pub turn_failed: Option<(String, DateTime<Utc>)>,
    /// Tasks created and completed in the session's task list since it last
    /// emptied.
    pub tasks_total: usize,
    pub tasks_done: usize,
    /// Background tasks and `/loop` schedules alive at the last `Stop`.
    pub background_tasks: usize,
    pub crons: usize,
    /// The model the session last switched to.
    pub model: Option<String>,
    /// The working directory the session last moved to.
    pub cwd: Option<String>,
}

/// One parsed hook message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookEvent {
    /// A `Notification` the attention layer does not turn into a question.
    Notification {
        notification_type: String,
    },
    Stop {
        background_tasks: usize,
        crons: usize,
    },
    StopFailure {
        reason: String,
    },
    PostCompact,
    PostModelSwitch {
        to_model: String,
    },
    TaskCreated,
    TaskCompleted,
    CwdChanged {
        cwd: String,
    },
    PermissionDenied {
        tool: String,
    },
    SessionEnd,
}

fn text(raw: &Value, key: &str) -> Option<String> {
    raw.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

fn count(raw: &Value, key: &str) -> usize {
    raw.get(key)
        .and_then(Value::as_u64)
        .map(|n| n as usize)
        .unwrap_or(0)
}

/// `s` on one line, at most [`MAX_TEXT`] characters.
pub fn short(s: &str) -> String {
    let line = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = line.chars().take(MAX_TEXT).collect();
    if out.chars().count() < line.chars().count() {
        out.pop();
        out.push('…');
    }
    out
}

/// The last path component, so a row says `cd api` rather than a full path.
fn last_component(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

impl HookEvent {
    /// Parse a `hook-event` IPC message. `None` for an unknown kind or a
    /// payload missing what the kind needs.
    pub fn parse(raw: &Value) -> Option<Self> {
        let kind = text(raw, "kind")?;
        Some(match kind.as_str() {
            "notification" => HookEvent::Notification {
                notification_type: text(raw, "notification_type")?.to_ascii_lowercase(),
            },
            "stop" => HookEvent::Stop {
                background_tasks: count(raw, "background_tasks_count"),
                crons: count(raw, "session_crons_count"),
            },
            "stop-failure" => HookEvent::StopFailure {
                reason: ["error_message", "error", "error_type", "reason", "message"]
                    .iter()
                    .find_map(|key| text(raw, key))
                    .map(|reason| short(&reason))
                    .unwrap_or_else(|| "API error".into()),
            },
            "post-compact" => HookEvent::PostCompact,
            "post-model-switch" => HookEvent::PostModelSwitch {
                to_model: short(&text(raw, "to_model")?),
            },
            "task-created" => HookEvent::TaskCreated,
            "task-completed" => HookEvent::TaskCompleted,
            "cwd-changed" => HookEvent::CwdChanged {
                cwd: text(raw, "new_cwd").or_else(|| text(raw, "cwd"))?,
            },
            "permission-denied" => HookEvent::PermissionDenied {
                tool: short(&text(raw, "tool_name").unwrap_or_else(|| "tool".into())),
            },
            "session-end" => HookEvent::SessionEnd,
            _ => return None,
        })
    }
}

/// The row event a notification type means, if any. Blocking ones
/// (`permission_prompt`, `agent_needs_input`, elicitation dialogs) are the
/// attention layer's; `agent_completed` is the `Stop` hook's; logins are noise.
fn notification_event(notification_type: &str) -> Option<RowEventKind> {
    Some(match notification_type {
        "idle_prompt" => RowEventKind::Idle,
        "quota_auto_resume_fired" => RowEventKind::Quota("resumed"),
        "quota_auto_resume_stale" => RowEventKind::Quota("paused"),
        "quota_auto_resume_disabled" => RowEventKind::Quota("no auto-resume"),
        _ => return None,
    })
}

impl HookEventState {
    /// Fold `event` in. Returns `true` when the turn ended (a failure or a
    /// session end), so the caller clears its working markers.
    pub fn apply(&mut self, event: HookEvent, now: DateTime<Utc>) -> bool {
        let note = |state: &mut Self, kind: RowEventKind| {
            state.event = Some(RowEvent { kind, at: now });
        };
        match event {
            HookEvent::Notification { notification_type } => {
                if let Some(kind) = notification_event(&notification_type) {
                    note(self, kind);
                }
                false
            }
            HookEvent::Stop {
                background_tasks,
                crons,
            } => {
                self.background_tasks = background_tasks;
                self.crons = crons;
                false
            }
            HookEvent::StopFailure { reason } => {
                self.turn_failed = Some((reason, now));
                true
            }
            HookEvent::PostCompact => {
                note(self, RowEventKind::Compacted);
                false
            }
            HookEvent::PostModelSwitch { to_model } => {
                self.model = Some(to_model.clone());
                note(self, RowEventKind::ModelSwitched(to_model));
                false
            }
            HookEvent::TaskCreated => {
                // A list that had emptied starts counting again.
                if self.tasks_total > 0 && self.tasks_done >= self.tasks_total {
                    self.tasks_total = 0;
                    self.tasks_done = 0;
                }
                self.tasks_total += 1;
                false
            }
            HookEvent::TaskCompleted => {
                self.tasks_done = (self.tasks_done + 1).min(self.tasks_total.max(1));
                self.tasks_total = self.tasks_total.max(self.tasks_done);
                false
            }
            HookEvent::CwdChanged { cwd } => {
                self.cwd = Some(cwd);
                false
            }
            HookEvent::PermissionDenied { tool } => {
                note(self, RowEventKind::Denied(tool));
                false
            }
            HookEvent::SessionEnd => {
                *self = HookEventState::default();
                true
            }
        }
    }

    /// A new turn started: a failure and the idle nudge are history.
    pub fn turn_started(&mut self) {
        self.turn_failed = None;
        if self
            .event
            .as_ref()
            .is_some_and(|event| event.kind.retired_by_new_turn())
        {
            self.event = None;
        }
    }

    /// The event text to show now, if it has not gone stale.
    pub fn event_text(&self, now: DateTime<Utc>) -> Option<String> {
        let event = self.event.as_ref()?;
        (now - event.at < Duration::seconds(EVENT_TTL_SECS)).then(|| event.kind.text())
    }

    /// `tasks 2/5` while the list has open items.
    pub fn tasks_text(&self) -> Option<String> {
        (self.tasks_total > 0 && self.tasks_done < self.tasks_total)
            .then(|| format!("tasks {}/{}", self.tasks_done, self.tasks_total))
    }

    /// Background tasks plus schedules alive at the last `Stop`.
    pub fn background_work(&self) -> usize {
        self.background_tasks + self.crons
    }

    /// `1 bg · 1 loop`, or nothing.
    pub fn background_text(&self) -> Option<String> {
        let mut parts = Vec::new();
        if self.background_tasks > 0 {
            parts.push(format!("{} bg", self.background_tasks));
        }
        match self.crons {
            0 => {}
            1 => parts.push("1 loop".into()),
            n => parts.push(format!("{n} loops")),
        }
        (!parts.is_empty()).then(|| parts.join(" · "))
    }

    /// `cd api` when the session moved somewhere else than `workdir`.
    pub fn cwd_text(&self, workdir: &std::path::Path) -> Option<String> {
        let cwd = self.cwd.as_deref()?;
        (std::path::Path::new(cwd) != workdir).then(|| format!("cd {}", last_component(cwd)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn t0() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-10-11T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    fn apply(state: &mut HookEventState, raw: Value) -> bool {
        state.apply(HookEvent::parse(&raw).expect("parses"), t0())
    }

    #[test]
    fn every_kind_the_scripts_pass_parses_with_a_minimal_payload() {
        let samples = [
            json!({"kind": "notification", "notification_type": "idle_prompt"}),
            json!({"kind": "stop"}),
            json!({"kind": "stop-failure"}),
            json!({"kind": "post-compact"}),
            json!({"kind": "post-model-switch", "to_model": "sonnet"}),
            json!({"kind": "task-created"}),
            json!({"kind": "task-completed"}),
            json!({"kind": "cwd-changed", "cwd": "/a"}),
            json!({"kind": "permission-denied"}),
            json!({"kind": "session-end"}),
        ];
        assert_eq!(samples.len(), KINDS.len());
        for (sample, kind) in samples.iter().zip(KINDS) {
            assert_eq!(sample["kind"], *kind);
            assert!(HookEvent::parse(sample).is_some(), "{kind}");
        }
        assert!(HookEvent::parse(&json!({"kind": "nonsense"})).is_none());
        assert!(HookEvent::parse(&json!({"kind": "post-model-switch"})).is_none());
    }

    #[test]
    fn compaction_and_model_switch_show_and_a_new_turn_retires_only_the_first() {
        let mut state = HookEventState::default();
        apply(&mut state, json!({"kind": "post-compact"}));
        assert_eq!(state.event_text(t0()).as_deref(), Some("compacted"));
        state.turn_started();
        assert_eq!(state.event_text(t0()), None);

        apply(
            &mut state,
            json!({"kind": "post-model-switch", "from_model": "opus", "to_model": "sonnet"}),
        );
        assert_eq!(state.event_text(t0()).as_deref(), Some("model → sonnet"));
        state.turn_started();
        assert_eq!(state.event_text(t0()).as_deref(), Some("model → sonnet"));
        assert_eq!(state.model.as_deref(), Some("sonnet"));
    }

    #[test]
    fn an_event_goes_stale_after_its_ttl() {
        let mut state = HookEventState::default();
        apply(&mut state, json!({"kind": "post-compact"}));
        let later = t0() + Duration::seconds(EVENT_TTL_SECS + 1);
        assert_eq!(state.event_text(later), None);
    }

    #[test]
    fn a_stop_failure_ends_the_turn_with_the_reason_until_the_next_turn() {
        let mut state = HookEventState::default();
        let ended = apply(
            &mut state,
            json!({"kind": "stop-failure", "error": "overloaded_error: Overloaded"}),
        );
        assert!(ended);
        assert_eq!(
            state.turn_failed.as_ref().map(|(r, _)| r.as_str()),
            Some("overloaded_error: Overloaded")
        );
        state.turn_started();
        assert!(state.turn_failed.is_none());
        let mut bare = HookEventState::default();
        apply(&mut bare, json!({"kind": "stop-failure"}));
        assert_eq!(bare.turn_failed.unwrap().0, "API error");
    }

    #[test]
    fn tasks_count_up_and_restart_once_the_list_emptied() {
        let mut state = HookEventState::default();
        for _ in 0..3 {
            apply(
                &mut state,
                json!({"kind": "task-created", "task_subject": "x"}),
            );
        }
        apply(&mut state, json!({"kind": "task-completed"}));
        assert_eq!(state.tasks_text().as_deref(), Some("tasks 1/3"));
        apply(&mut state, json!({"kind": "task-completed"}));
        apply(&mut state, json!({"kind": "task-completed"}));
        assert_eq!(state.tasks_text(), None, "all done shows nothing");
        apply(&mut state, json!({"kind": "task-created"}));
        assert_eq!(state.tasks_text().as_deref(), Some("tasks 0/1"));
        // A completion without a creation never shows more done than total.
        let mut odd = HookEventState::default();
        apply(&mut odd, json!({"kind": "task-completed"}));
        assert_eq!((odd.tasks_done, odd.tasks_total), (1, 1));
    }

    #[test]
    fn stop_records_background_work_and_session_end_resets_everything() {
        let mut state = HookEventState::default();
        apply(
            &mut state,
            json!({"kind": "stop", "background_tasks_count": 2, "session_crons_count": 1}),
        );
        assert_eq!(state.background_work(), 3);
        assert_eq!(state.background_text().as_deref(), Some("2 bg · 1 loop"));
        apply(&mut state, json!({"kind": "stop"}));
        assert_eq!(state.background_work(), 0);
        assert_eq!(state.background_text(), None);
        apply(&mut state, json!({"kind": "post-compact"}));
        assert!(apply(
            &mut state,
            json!({"kind": "session-end", "reason": "other"})
        ));
        assert_eq!(state, HookEventState::default());
    }

    #[test]
    fn only_non_blocking_notifications_become_row_events() {
        let mut state = HookEventState::default();
        for kind in [
            "permission_prompt",
            "agent_needs_input",
            "agent_completed",
            "auth_success",
            "elicitation_dialog",
        ] {
            apply(
                &mut state,
                json!({"kind": "notification", "notification_type": kind}),
            );
            assert_eq!(state.event, None, "{kind}");
        }
        apply(
            &mut state,
            json!({"kind": "notification", "notification_type": "IDLE_PROMPT"}),
        );
        assert_eq!(state.event_text(t0()).as_deref(), Some("idle"));
        apply(
            &mut state,
            json!({"kind": "notification", "notification_type": "quota_auto_resume_stale"}),
        );
        assert_eq!(state.event_text(t0()).as_deref(), Some("quota: paused"));
    }

    #[test]
    fn denials_and_directory_moves_are_short() {
        let mut state = HookEventState::default();
        apply(
            &mut state,
            json!({"kind": "permission-denied", "tool_name": "Bash"}),
        );
        assert_eq!(state.event_text(t0()).as_deref(), Some("denied: Bash"));
        apply(
            &mut state,
            json!({"kind": "cwd-changed", "new_cwd": "/repo/packages/api"}),
        );
        assert_eq!(
            state.cwd_text(std::path::Path::new("/repo")).as_deref(),
            Some("cd api")
        );
        assert_eq!(
            state.cwd_text(std::path::Path::new("/repo/packages/api")),
            None
        );
    }

    #[test]
    fn short_keeps_one_line_and_marks_a_cut() {
        assert_eq!(short("a\n  b"), "a b");
        let long = "x".repeat(100);
        let cut = short(&long);
        assert_eq!(cut.chars().count(), MAX_TEXT);
        assert!(cut.ends_with('…'));
    }
}
