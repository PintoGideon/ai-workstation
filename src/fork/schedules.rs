//! FORK: §19.9 — scheduled and recurring prompts and commands.
//!
//! Pure parts: the schedule spec grammar, the cron subset, the next fire time
//! (in any [`TimeZone`], so DST is testable without the machine's zone), the
//! run outcomes, and the `aiw.json` entry. `src/app/fork_schedules.rs` owns
//! the timer, delivery and persistence.
//!
//! Spec grammar (case-insensitive):
//!
//! - `every <n><unit>[<n><unit>…]` — units `s`, `m`/`min`, `h`/`hr`, `d`;
//!   words (`minutes`, `hours`) and spaces are accepted: `every 90m`,
//!   `every 1h30m`, `every 2 hours`. Not shorter than
//!   `schedule_min_interval_secs` (default 60).
//! - `daily HH:MM` (or `daily at HH:MM`) — local time.
//! - `cron M H DOM MON DOW` (the `cron` word is optional) — local time; see
//!   [`CronSpec`] for the supported subset.
//! - `at YYYY-MM-DD HH:MM` (local), `at <RFC 3339>`, or `at HH:MM` (the next
//!   such local time; [`normalize_spec`] pins it to a date when saved) — once.

use std::collections::BTreeSet;
use std::fmt;

use chrono::{
    DateTime, Datelike, Duration, LocalResult, NaiveDate, NaiveDateTime, NaiveTime, TimeZone,
    Timelike, Utc,
};
use serde::{Deserialize, Serialize};

/// The automation action `aiw automation schedule …` sends.
pub const SCHEDULE_ACTION: &str = "schedule";

/// Default shortest interval for `every` (seconds).
pub const DEFAULT_MIN_INTERVAL_SECS: u64 = 60;

/// Run history kept per schedule.
pub const HISTORY_CAP: usize = 100;

/// How often the timer looks at the schedules.
pub const TICK_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// How often `aiw.json` schedules are re-read.
pub const CONFIG_SYNC_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// How often a fire deferred because the agent waits on you is retried.
pub const DEFER_RETRY_SECS: i64 = 30;

/// A one-shot that keeps waiting on you gives up after this long.
pub const ONE_SHOT_DEFER_LIMIT_SECS: i64 = 3600;

/// The terminal a shell schedule runs in when none is named.
pub const DEFAULT_TERMINAL: &str = "schedule";

/// Days searched for the next cron match (covers Feb 29 on a given weekday).
const CRON_SEARCH_DAYS: i64 = 366 * 8;

// ── the spec ─────────────────────────────────────────────────────────────

/// When a schedule fires.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScheduleSpec {
    /// Every `n` seconds after the previous fire (or after creation).
    Every(u64),
    /// Local wall-clock rules.
    Cron(CronSpec),
    /// Once, at a local wall-clock time.
    At(NaiveDateTime),
    /// Once, at an instant (RFC 3339 with an offset).
    AtInstant(DateTime<Utc>),
}

impl ScheduleSpec {
    /// Whether it fires once only.
    pub fn is_one_shot(&self) -> bool {
        matches!(self, Self::At(_) | Self::AtInstant(_))
    }

    /// The first fire strictly after `after`, read in `tz`; `None` when a
    /// one-shot's time is not after it.
    pub fn next_after<Tz: TimeZone>(&self, after: DateTime<Utc>, tz: &Tz) -> Option<DateTime<Utc>> {
        match self {
            Self::Every(secs) => Some(after + Duration::seconds(*secs as i64)),
            Self::Cron(cron) => cron.next_after(after, tz),
            Self::At(naive) => Some(resolve_local(*naive, tz)).filter(|at| *at > after),
            Self::AtInstant(at) => Some(*at).filter(|at| *at > after),
        }
    }

    /// For people: `every 30 min`, `daily at 09:30`, `once at …`.
    pub fn describe(&self) -> String {
        match self {
            Self::Every(secs) => format!("every {}", describe_duration(*secs)),
            Self::Cron(cron) => match cron.as_daily() {
                Some((h, m)) => format!("daily at {h:02}:{m:02}"),
                None => format!("cron {}", cron.source),
            },
            Self::At(naive) => format!("once at {}", naive.format("%Y-%m-%d %H:%M")),
            Self::AtInstant(at) => format!("once at {}", at.to_rfc3339()),
        }
    }
}

impl fmt::Display for ScheduleSpec {
    /// The canonical text: parsing it gives the same spec back.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Every(secs) => write!(f, "every {}", compact_duration(*secs)),
            Self::Cron(cron) => match cron.as_daily() {
                Some((h, m)) => write!(f, "daily {h:02}:{m:02}"),
                None => write!(f, "cron {}", cron.source),
            },
            Self::At(naive) => write!(f, "at {}", naive.format("%Y-%m-%d %H:%M")),
            Self::AtInstant(at) => write!(f, "at {}", at.to_rfc3339()),
        }
    }
}

/// Parse a spec. `min_interval_secs` bounds `every`.
pub fn parse_spec(text: &str, min_interval_secs: u64) -> Result<ScheduleSpec, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("the schedule is empty (e.g. `every 30m`, `daily 09:00`, `cron 0 9 * * 1-5`, `at 2026-10-12 15:00`)".into());
    }
    let lower = text.to_ascii_lowercase();
    let (word, rest) = match lower.split_once(char::is_whitespace) {
        Some((w, r)) => (w, r.trim()),
        None => (lower.as_str(), ""),
    };
    match word {
        "every" => {
            let secs = parse_duration(rest)?;
            let min = min_interval_secs.max(1);
            if secs < min {
                return Err(format!(
                    "`every` must be at least {} (schedule_min_interval_secs)",
                    describe_duration(min)
                ));
            }
            Ok(ScheduleSpec::Every(secs))
        }
        "daily" => {
            let rest = rest.strip_prefix("at").map(str::trim).unwrap_or(rest);
            let (h, m) = parse_hhmm(rest)?;
            Ok(ScheduleSpec::Cron(CronSpec::daily(h, m)))
        }
        "cron" => Ok(ScheduleSpec::Cron(CronSpec::parse(rest)?)),
        "at" => parse_at(&text[2..]),
        _ if lower.split_whitespace().count() == 5 => {
            Ok(ScheduleSpec::Cron(CronSpec::parse(&lower)?))
        }
        _ => Err(format!(
            "unknown schedule `{text}`: start with every, daily, cron or at"
        )),
    }
}

/// Parse `at …`: `YYYY-MM-DD HH:MM`, RFC 3339, or a bare `HH:MM` (kept as
/// today's date at midnight + time; [`normalize_spec`] moves it forward).
fn parse_at(rest: &str) -> Result<ScheduleSpec, String> {
    let rest = rest.trim();
    if let Ok(at) = DateTime::parse_from_rfc3339(rest) {
        return Ok(ScheduleSpec::AtInstant(at.with_timezone(&Utc)));
    }
    for format in ["%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M:%S"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(rest, format) {
            return Ok(ScheduleSpec::At(naive.with_second(0).unwrap_or(naive)));
        }
    }
    if let Ok((h, m)) = parse_hhmm(rest) {
        // A placeholder date: the time-of-day travels; `normalize_spec` pins it.
        let time = NaiveTime::from_hms_opt(h, m, 0).expect("checked");
        return Ok(ScheduleSpec::At(NaiveDate::MIN.and_time(time)));
    }
    Err(format!(
        "`at {rest}` is not a time: use `at 2026-10-12 15:00`, `at 15:00` or an RFC 3339 time"
    ))
}

/// The spec as it is saved: canonical text, with `at HH:MM` pinned to its
/// next local occurrence after `now`. Errors on a one-shot in the past.
pub fn normalize_spec<Tz: TimeZone>(
    text: &str,
    min_interval_secs: u64,
    now: DateTime<Utc>,
    tz: &Tz,
) -> Result<String, String> {
    let spec = match parse_spec(text, min_interval_secs)? {
        ScheduleSpec::At(naive) if naive.date() == NaiveDate::MIN => {
            let local_now = now.with_timezone(tz).naive_local();
            let mut candidate = local_now.date().and_time(naive.time());
            if resolve_local(candidate, tz) <= now {
                candidate += Duration::days(1);
            }
            ScheduleSpec::At(candidate)
        }
        other => other,
    };
    if spec.is_one_shot() && spec.next_after(now, tz).is_none() {
        return Err(format!("{} has already passed", spec.describe()));
    }
    Ok(spec.to_string())
}

fn parse_hhmm(text: &str) -> Result<(u32, u32), String> {
    let (h, m) = text
        .trim()
        .split_once(':')
        .ok_or_else(|| format!("`{text}` is not HH:MM"))?;
    let h: u32 = h
        .trim()
        .parse()
        .map_err(|_| format!("`{text}` is not HH:MM"))?;
    let m: u32 = m
        .trim()
        .parse()
        .map_err(|_| format!("`{text}` is not HH:MM"))?;
    if h > 23 || m > 59 {
        return Err(format!("`{text}` is not a time of day"));
    }
    Ok((h, m))
}

/// `30m`, `1h30m`, `2 hours`, `90 min`, `45s` → seconds.
fn parse_duration(text: &str) -> Result<u64, String> {
    let bad = || format!("`{text}` is not a duration (e.g. 30m, 2h, 1h30m)");
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.is_empty() {
        return Err(bad());
    }
    let mut total: u64 = 0;
    let mut chars = compact.chars().peekable();
    let mut any = false;
    while chars.peek().is_some() {
        let mut digits = String::new();
        while let Some(c) = chars.peek().copied().filter(char::is_ascii_digit) {
            digits.push(c);
            chars.next();
        }
        let mut unit = String::new();
        while let Some(c) = chars.peek().copied().filter(char::is_ascii_alphabetic) {
            unit.push(c);
            chars.next();
        }
        if digits.is_empty() || unit.is_empty() {
            return Err(bad());
        }
        let n: u64 = digits.parse().map_err(|_| bad())?;
        let scale = match unit.as_str() {
            "s" | "sec" | "secs" | "second" | "seconds" => 1,
            "m" | "min" | "mins" | "minute" | "minutes" => 60,
            "h" | "hr" | "hrs" | "hour" | "hours" => 3600,
            "d" | "day" | "days" => 86_400,
            _ => return Err(bad()),
        };
        total = total
            .checked_add(n.checked_mul(scale).ok_or_else(bad)?)
            .ok_or_else(bad)?;
        any = true;
    }
    if !any || total == 0 {
        return Err(bad());
    }
    Ok(total)
}

/// `1h30m` style, for the canonical text.
fn compact_duration(secs: u64) -> String {
    let mut out = String::new();
    let mut left = secs;
    for (scale, unit) in [(86_400, "d"), (3600, "h"), (60, "m"), (1, "s")] {
        if left >= scale {
            out.push_str(&format!("{}{unit}", left / scale));
            left %= scale;
        }
    }
    out
}

/// `30 min`, `2 h`, `1 h 30 min`, `45 s`.
pub fn describe_duration(secs: u64) -> String {
    let mut parts = Vec::new();
    let mut left = secs;
    for (scale, unit) in [(86_400, "d"), (3600, "h"), (60, "min"), (1, "s")] {
        if left >= scale {
            parts.push(format!("{} {unit}", left / scale));
            left %= scale;
        }
    }
    parts.join(" ")
}

// ── cron ─────────────────────────────────────────────────────────────────

/// The cron subset: five numeric fields — minute (0-59), hour (0-23), day of
/// month (1-31), month (1-12), day of week (0-7, 0 and 7 are Sunday). Each
/// field is `*`, a number, a range `a-b`, a list `a,b,c`, or a step `*/n`,
/// `a-b/n`, `a/n`. Names (`MON`, `JAN`), `L`, `W`, `#`, `?` and `@daily`-style
/// macros are not supported. When both day of month and day of week are
/// restricted, either matching is enough (as in Vixie cron).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronSpec {
    pub minutes: BTreeSet<u32>,
    pub hours: BTreeSet<u32>,
    pub days: BTreeSet<u32>,
    pub months: BTreeSet<u32>,
    pub weekdays: BTreeSet<u32>,
    pub days_any: bool,
    pub weekdays_any: bool,
    /// The five fields as written (normalized spacing).
    pub source: String,
}

impl CronSpec {
    fn daily(hour: u32, minute: u32) -> Self {
        Self {
            minutes: [minute].into(),
            hours: [hour].into(),
            days: (1..=31).collect(),
            months: (1..=12).collect(),
            weekdays: (0..=6).collect(),
            days_any: true,
            weekdays_any: true,
            source: format!("{minute} {hour} * * *"),
        }
    }

    /// `(hour, minute)` when this is a plain daily rule.
    fn as_daily(&self) -> Option<(u32, u32)> {
        (self.minutes.len() == 1
            && self.hours.len() == 1
            && self.days_any
            && self.weekdays_any
            && self.months.len() == 12)
            .then(|| {
                (
                    *self.hours.iter().next().unwrap(),
                    *self.minutes.iter().next().unwrap(),
                )
            })
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let fields: Vec<&str> = text.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(format!(
                "cron needs 5 fields (minute hour day-of-month month day-of-week), got {}",
                fields.len()
            ));
        }
        let minutes = parse_field(fields[0], 0, 59, "minute")?;
        let hours = parse_field(fields[1], 0, 23, "hour")?;
        let days = parse_field(fields[2], 1, 31, "day of month")?;
        let months = parse_field(fields[3], 1, 12, "month")?;
        let mut weekdays = parse_field(fields[4], 0, 7, "day of week")?;
        if weekdays.remove(&7) {
            weekdays.insert(0);
        }
        Ok(Self {
            minutes,
            hours,
            days,
            months,
            weekdays,
            days_any: fields[2] == "*",
            weekdays_any: fields[4] == "*",
            source: fields.join(" "),
        })
    }

    fn day_matches(&self, date: NaiveDate) -> bool {
        if !self.months.contains(&date.month()) {
            return false;
        }
        let dom = self.days.contains(&date.day());
        let dow = self
            .weekdays
            .contains(&date.weekday().num_days_from_sunday());
        match (self.days_any, self.weekdays_any) {
            (true, true) => true,
            (true, false) => dow,
            (false, true) => dom,
            (false, false) => dom || dow,
        }
    }

    /// The first matching local minute strictly after `after`.
    pub fn next_after<Tz: TimeZone>(&self, after: DateTime<Utc>, tz: &Tz) -> Option<DateTime<Utc>> {
        let start = after.with_timezone(tz).naive_local().date() - Duration::days(1);
        for offset in 0..CRON_SEARCH_DAYS {
            let date = start + Duration::days(offset);
            if !self.day_matches(date) {
                continue;
            }
            for hour in &self.hours {
                for minute in &self.minutes {
                    let Some(time) = NaiveTime::from_hms_opt(*hour, *minute, 0) else {
                        continue;
                    };
                    let instant = resolve_local(date.and_time(time), tz);
                    if instant > after {
                        return Some(instant);
                    }
                }
            }
        }
        None
    }
}

fn parse_field(text: &str, min: u32, max: u32, name: &str) -> Result<BTreeSet<u32>, String> {
    const HELP: &str = "cron supports numbers, *, ranges (a-b), lists (a,b) and steps (*/n, a-b/n); names (MON, JAN), L, W, # and ? are not";
    let bad = |why: String| format!("cron {name} field `{text}`: {why}; {HELP}");
    let mut out = BTreeSet::new();
    for item in text.split(',') {
        if item.is_empty() {
            return Err(bad("empty list item".into()));
        }
        let (range, step) = match item.split_once('/') {
            Some((range, step)) => {
                let step: u32 = step
                    .parse()
                    .map_err(|_| bad(format!("`{step}` is not a step")))?;
                if step == 0 {
                    return Err(bad("a step of 0".into()));
                }
                (range, step)
            }
            None => (item, 1),
        };
        let number = |s: &str| -> Result<u32, String> {
            let n: u32 = s
                .parse()
                .map_err(|_| bad(format!("`{s}` is not a number")))?;
            if n < min || n > max {
                return Err(bad(format!("{n} is outside {min}-{max}")));
            }
            Ok(n)
        };
        let (lo, hi) = if range == "*" {
            (min, max)
        } else if let Some((a, b)) = range.split_once('-') {
            let (a, b) = (number(a)?, number(b)?);
            if a > b {
                return Err(bad(format!("the range {a}-{b} runs backwards")));
            }
            (a, b)
        } else {
            let a = number(range)?;
            // `a/n` runs from a to the end of the field.
            (a, if item.contains('/') { max } else { a })
        };
        out.extend((lo..=hi).step_by(step as usize));
    }
    Ok(out)
}

/// A local wall-clock time as an instant: the earlier of a repeated time
/// (clocks going back), the first valid minute after a skipped one (clocks
/// going forward).
pub fn resolve_local<Tz: TimeZone>(naive: NaiveDateTime, tz: &Tz) -> DateTime<Utc> {
    let mut candidate = naive;
    // A gap is at most a few hours; search a day to be safe.
    for _ in 0..(24 * 60) {
        match tz.from_local_datetime(&candidate) {
            LocalResult::Single(dt) => return dt.with_timezone(&Utc),
            LocalResult::Ambiguous(earlier, _) => return earlier.with_timezone(&Utc),
            LocalResult::None => candidate += Duration::minutes(1),
        }
    }
    // Unreachable for real zones; read the time as UTC.
    Utc.from_utc_datetime(&naive)
}

// ── schedules ────────────────────────────────────────────────────────────

/// What a schedule does when it fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ScheduleKind {
    /// Deliver a prompt to an agent session.
    #[default]
    Prompt,
    /// Run a shell command in a terminal session of the feature.
    Shell,
}

impl ScheduleKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prompt => "prompt",
            Self::Shell => "shell",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "prompt" => Some(Self::Prompt),
            "shell" | "command" => Some(Self::Shell),
            _ => None,
        }
    }
}

/// What happens to fires missed while AIW was not running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CatchUp {
    /// Skip them (recorded as missed).
    None,
    /// Run once when AIW starts.
    #[default]
    Once,
}

impl CatchUp {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Once => "once",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "none" | "skip" | "off" => Some(Self::None),
            "once" | "on" => Some(Self::Once),
            _ => None,
        }
    }
}

/// One schedule, as stored.
#[derive(Debug, Clone, PartialEq)]
pub struct Schedule {
    pub id: String,
    /// The feature it belongs to (always set).
    pub feature_id: String,
    /// The agent session a prompt goes to, when pinned to one.
    pub session_id: Option<String>,
    /// Prompt: the session label to deliver to when not pinned (`None`:
    /// the feature's first agent). Shell: the terminal's name.
    pub target: Option<String>,
    pub kind: ScheduleKind,
    /// The canonical spec text.
    pub spec: String,
    /// The prompt or the command.
    pub text: String,
    pub paused: bool,
    pub start_if_stopped: bool,
    pub catch_up: CatchUp,
    pub end_at: Option<DateTime<Utc>>,
    pub max_runs: Option<u32>,
    pub runs: u32,
    pub last_fire: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    /// `aiw.json` entry name for project-defined schedules.
    pub config_name: Option<String>,
    /// The follow-up this schedule's last fire queued, while it waits.
    pub queued_followup: Option<i64>,
}

impl Schedule {
    /// The spec. Saved specs were validated with the bound in force when
    /// they were saved, so a raised bound does not break them.
    pub fn parsed(&self) -> Result<ScheduleSpec, String> {
        parse_spec(&self.spec, 1)
    }

    /// Whether it can never fire again: a one-shot that ran, its run limit,
    /// or its end time.
    pub fn finished(&self, now: DateTime<Utc>) -> bool {
        if self.max_runs.is_some_and(|max| self.runs >= max) {
            return true;
        }
        if self.end_at.is_some_and(|end| now >= end) {
            return true;
        }
        let one_shot = parse_spec(&self.spec, 1).is_ok_and(|s| s.is_one_shot());
        one_shot && self.last_fire.is_some()
    }

    /// Paused or finished.
    pub fn inactive(&self, now: DateTime<Utc>) -> bool {
        self.paused || self.finished(now)
    }

    /// The next fire time, when it has one.
    pub fn next_fire<Tz: TimeZone>(&self, now: DateTime<Utc>, tz: &Tz) -> Option<DateTime<Utc>> {
        if self.inactive(now) {
            return None;
        }
        let spec = parse_spec(&self.spec, 1).ok()?;
        let next = spec.next_after(self.last_fire.unwrap_or(self.created_at), tz)?;
        if self.end_at.is_some_and(|end| next >= end) {
            return None;
        }
        Some(next)
    }

    /// Whether this came from `aiw.json`.
    pub fn from_config(&self) -> bool {
        self.config_name.is_some()
    }

    /// `→ Claude 1` / `→ first agent` / `$ schedule`.
    pub fn target_label(&self, session_label: Option<&str>) -> String {
        match self.kind {
            ScheduleKind::Prompt => match (session_label, &self.target) {
                (Some(label), _) => format!("→ {label}"),
                (None, Some(label)) => format!("→ {label}"),
                (None, None) => "→ first agent".to_string(),
            },
            ScheduleKind::Shell => {
                format!("$ {}", self.target.as_deref().unwrap_or(DEFAULT_TERMINAL))
            }
        }
    }
}

/// When a fire is due, and why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Due {
    /// Its time came while AIW ran.
    Now(DateTime<Utc>),
    /// Its time came before AIW started (`catch_up` decides).
    Missed(DateTime<Utc>),
}

/// Whether `schedule` is due at `now`. `started` is when AIW started.
pub fn due<Tz: TimeZone>(
    schedule: &Schedule,
    now: DateTime<Utc>,
    started: DateTime<Utc>,
    tz: &Tz,
) -> Option<Due> {
    let next = schedule.next_fire(now, tz)?;
    if next > now {
        return None;
    }
    Some(if next < started {
        Due::Missed(next)
    } else {
        Due::Now(next)
    })
}

/// What one fire did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Pasted into the idle agent and sent.
    Delivered,
    /// Into a Claude session's inbox (background sessions).
    DeliveredInbox,
    /// Queued: the agent was working (or had something queued); it goes
    /// at its turn end.
    Queued,
    /// The command was typed into its terminal.
    Ran,
    /// The session was started; the prompt goes once it is ready.
    Started,
    /// The agent waits on you; retried shortly.
    SkippedWaiting,
    SkippedNotRunning,
    SkippedStillQueued,
    /// The terminal runs something else.
    SkippedBusy(String),
    SkippedLimits,
    /// Missed while AIW was not running (`catch_up: none`).
    Missed,
    Failed(String),
}

impl Outcome {
    pub fn label(&self) -> String {
        match self {
            Self::Delivered => "delivered".into(),
            Self::DeliveredInbox => "delivered (inbox)".into(),
            Self::Queued => "queued".into(),
            Self::Ran => "ran".into(),
            Self::Started => "started".into(),
            Self::SkippedWaiting => "skipped: waiting on you".into(),
            Self::SkippedNotRunning => "skipped: not running".into(),
            Self::SkippedStillQueued => "skipped: previous run still queued".into(),
            Self::SkippedBusy(what) => format!("skipped: busy ({what})"),
            Self::SkippedLimits => "skipped: resource limits".into(),
            Self::Missed => "missed while aiw was not running".into(),
            Self::Failed(why) => format!("failed: {why}"),
        }
    }

    /// Whether it counts towards `max_runs`.
    pub fn counts_as_run(&self) -> bool {
        matches!(
            self,
            Self::Delivered | Self::DeliveredInbox | Self::Queued | Self::Ran | Self::Started
        )
    }
}

/// One line of a schedule's history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRecord {
    pub at: DateTime<Utc>,
    pub outcome: String,
    pub detail: String,
}

// ── aiw.json ─────────────────────────────────────────────────────────────

/// A project-defined schedule (`schedules` in `aiw.json` or the global
/// extension config). It applies to every feature whose name or branch
/// matches `match` (`*` and `?` wildcards; default every feature).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ScheduleConfig {
    pub name: String,
    #[serde(rename = "match", skip_serializing_if = "Option::is_none")]
    pub matches: Option<String>,
    /// When (`every 30m`, `daily 09:00`, `cron …`).
    #[serde(alias = "when")]
    pub spec: String,
    /// A prompt for an agent (`session` picks which; default the first).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    /// Or a shell command, run in `terminal` (default `schedule`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal: Option<String>,
    pub start_if_stopped: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catch_up: Option<CatchUp>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_runs: Option<u32>,
}

impl ScheduleConfig {
    /// What it does, after checking it.
    pub fn validate(&self, min_interval_secs: u64) -> Result<(ScheduleKind, String), String> {
        let name = if self.name.trim().is_empty() {
            return Err("a schedule in aiw.json has no name".into());
        } else {
            self.name.trim()
        };
        let spec = parse_spec(&self.spec, min_interval_secs)
            .map_err(|e| format!("schedule `{name}`: {e}"))?;
        if spec.is_one_shot() {
            return Err(format!(
                "schedule `{name}`: aiw.json schedules recur; use every, daily or cron"
            ));
        }
        if let Some(end) = &self.end {
            parse_end(end).map_err(|e| format!("schedule `{name}`: {e}"))?;
        }
        match (&self.prompt, &self.command) {
            (Some(p), None) if !p.trim().is_empty() => Ok((ScheduleKind::Prompt, p.clone())),
            (None, Some(c)) if !c.trim().is_empty() => Ok((ScheduleKind::Shell, c.clone())),
            (Some(_), Some(_)) => Err(format!(
                "schedule `{name}` has both a prompt and a command; give one"
            )),
            _ => Err(format!("schedule `{name}` needs a prompt or a command")),
        }
    }

    /// Whether it applies to a feature with this name and branch.
    pub fn applies_to(&self, feature_name: &str, branch: &str) -> bool {
        let pattern = self.matches.as_deref().unwrap_or("*");
        glob_match(pattern, feature_name) || glob_match(pattern, branch)
    }
}

/// `YYYY-MM-DD HH:MM` (local, read as UTC here: callers convert) or RFC 3339.
pub fn parse_end(text: &str) -> Result<NaiveOrInstant, String> {
    let text = text.trim();
    if let Ok(at) = DateTime::parse_from_rfc3339(text) {
        return Ok(NaiveOrInstant::Instant(at.with_timezone(&Utc)));
    }
    for format in ["%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(text, format) {
            return Ok(NaiveOrInstant::Naive(naive));
        }
    }
    if let Ok(date) = NaiveDate::parse_from_str(text, "%Y-%m-%d") {
        return Ok(NaiveOrInstant::Naive(
            date.and_hms_opt(23, 59, 0).expect("valid"),
        ));
    }
    Err(format!(
        "end `{text}` is not a time: use `2026-10-31 18:00`, `2026-10-31` or RFC 3339"
    ))
}

/// A parsed end time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NaiveOrInstant {
    Naive(NaiveDateTime),
    Instant(DateTime<Utc>),
}

impl NaiveOrInstant {
    pub fn in_zone<Tz: TimeZone>(self, tz: &Tz) -> DateTime<Utc> {
        match self {
            Self::Naive(naive) => resolve_local(naive, tz),
            Self::Instant(at) => at,
        }
    }
}

/// `*` (any run) and `?` (one character) wildcards; otherwise exact.
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (None::<usize>, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// The id of an `aiw.json` schedule materialized for a feature.
pub fn config_schedule_id(feature_id: &str, name: &str) -> String {
    format!("cfg:{feature_id}:{name}")
}

/// `14:30` today, `Mon 14:30` within a week, else the date.
pub fn short_when<Tz: TimeZone>(at: DateTime<Utc>, now: DateTime<Utc>, tz: &Tz) -> String
where
    Tz::Offset: fmt::Display,
{
    let local = at.with_timezone(tz);
    let today = now.with_timezone(tz).date_naive();
    let days = (local.date_naive() - today).num_days();
    if days == 0 {
        local.format("%H:%M").to_string()
    } else if (-6..=6).contains(&days) {
        local.format("%a %H:%M").to_string()
    } else {
        local.format("%Y-%m-%d %H:%M").to_string()
    }
}

#[cfg(test)]
pub(crate) mod test_zone {
    //! A US-Eastern-like zone with fixed 2026 DST dates: UTC-5, and UTC-4
    //! from 2026-03-08 02:00 local to 2026-11-01 02:00 local.
    use chrono::{FixedOffset, LocalResult, NaiveDate, NaiveDateTime, Offset, TimeZone};
    use std::fmt;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct TestEastern;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct TestOffset(pub FixedOffset);

    impl Offset for TestOffset {
        fn fix(&self) -> FixedOffset {
            self.0
        }
    }

    impl fmt::Display for TestOffset {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}", self.0)
        }
    }

    fn est() -> FixedOffset {
        FixedOffset::west_opt(5 * 3600).unwrap()
    }
    fn edt() -> FixedOffset {
        FixedOffset::west_opt(4 * 3600).unwrap()
    }
    /// DST starts at 2026-03-08 07:00 UTC, ends at 2026-11-01 06:00 UTC.
    fn dst_start_utc() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 3, 8)
            .unwrap()
            .and_hms_opt(7, 0, 0)
            .unwrap()
    }
    fn dst_end_utc() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, 1)
            .unwrap()
            .and_hms_opt(6, 0, 0)
            .unwrap()
    }

    impl TimeZone for TestEastern {
        type Offset = TestOffset;

        fn from_offset(_offset: &TestOffset) -> Self {
            TestEastern
        }

        fn offset_from_local_date(&self, local: &NaiveDate) -> LocalResult<TestOffset> {
            self.offset_from_local_datetime(&local.and_hms_opt(12, 0, 0).unwrap())
        }

        fn offset_from_local_datetime(&self, local: &NaiveDateTime) -> LocalResult<TestOffset> {
            let as_est = *local + chrono::Duration::hours(5); // UTC if EST
            let as_edt = *local + chrono::Duration::hours(4); // UTC if EDT
            let est_ok = !(dst_start_utc() <= as_est && as_est < dst_end_utc());
            let edt_ok = dst_start_utc() <= as_edt && as_edt < dst_end_utc();
            match (edt_ok, est_ok) {
                (true, true) => LocalResult::Ambiguous(TestOffset(edt()), TestOffset(est())),
                (true, false) => LocalResult::Single(TestOffset(edt())),
                (false, true) => LocalResult::Single(TestOffset(est())),
                (false, false) => LocalResult::None,
            }
        }

        fn offset_from_utc_date(&self, utc: &NaiveDate) -> TestOffset {
            self.offset_from_utc_datetime(&utc.and_hms_opt(12, 0, 0).unwrap())
        }

        fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> TestOffset {
            if dst_start_utc() <= *utc && *utc < dst_end_utc() {
                TestOffset(edt())
            } else {
                TestOffset(est())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_zone::TestEastern;
    use super::*;

    fn utc(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn schedule(spec: &str) -> Schedule {
        Schedule {
            id: "s1".into(),
            feature_id: "f1".into(),
            session_id: None,
            target: None,
            kind: ScheduleKind::Prompt,
            spec: spec.into(),
            text: "go".into(),
            paused: false,
            start_if_stopped: false,
            catch_up: CatchUp::Once,
            end_at: None,
            max_runs: None,
            runs: 0,
            last_fire: None,
            created_at: utc("2026-10-12T10:00:00Z"),
            config_name: None,
            queued_followup: None,
        }
    }

    #[test]
    fn every_parses_units_words_and_combinations() {
        assert_eq!(parse_spec("every 30m", 60), Ok(ScheduleSpec::Every(1800)));
        assert_eq!(
            parse_spec("Every 2 hours", 60),
            Ok(ScheduleSpec::Every(7200))
        );
        assert_eq!(parse_spec("every 1h30m", 60), Ok(ScheduleSpec::Every(5400)));
        assert_eq!(
            parse_spec("every 1h 30 min", 60),
            Ok(ScheduleSpec::Every(5400))
        );
        assert_eq!(parse_spec("every 1d", 60), Ok(ScheduleSpec::Every(86_400)));
        assert_eq!(parse_spec("every 45s", 5), Ok(ScheduleSpec::Every(45)));
        assert_eq!(
            parse_spec("every 90m", 60).unwrap().to_string(),
            "every 1h30m"
        );
    }

    #[test]
    fn every_rejects_garbage_and_short_intervals() {
        for bad in [
            "every",
            "every m",
            "every 30",
            "every 3x",
            "every 0m",
            "every -5m",
        ] {
            assert!(parse_spec(bad, 60).is_err(), "{bad} should fail");
        }
        let err = parse_spec("every 30s", 60).unwrap_err();
        assert!(err.contains("at least 1 min"), "{err}");
        assert!(parse_spec("every 30s", 10).is_ok());
    }

    #[test]
    fn daily_and_cron_parse_and_round_trip() {
        let daily = parse_spec("daily 09:30", 60).unwrap();
        assert_eq!(daily.to_string(), "daily 09:30");
        assert_eq!(daily.describe(), "daily at 09:30");
        assert_eq!(
            parse_spec("daily at 9:05", 60).unwrap().to_string(),
            "daily 09:05"
        );
        let cron = parse_spec("cron 0 9 * * 1-5", 60).unwrap();
        assert_eq!(cron.to_string(), "cron 0 9 * * 1-5");
        assert_eq!(parse_spec(&cron.to_string(), 60).unwrap(), cron);
        // The `cron` word is optional.
        assert_eq!(
            parse_spec("*/15 * * * *", 60).unwrap().to_string(),
            "cron */15 * * * *"
        );
        // A cron that is a plain daily rule prints as daily.
        assert_eq!(
            parse_spec("cron 0 18 * * *", 60).unwrap().to_string(),
            "daily 18:00"
        );
    }

    #[test]
    fn cron_fields_support_lists_ranges_and_steps() {
        let c = CronSpec::parse("0,30 9-17/2 1,15 */3 0-6").unwrap();
        assert_eq!(c.minutes, [0, 30].into());
        assert_eq!(c.hours, [9, 11, 13, 15, 17].into());
        assert_eq!(c.days, [1, 15].into());
        assert_eq!(c.months, [1, 4, 7, 10].into());
        assert_eq!(c.weekdays.len(), 7);
        let c = CronSpec::parse("5/20 * * * 7").unwrap();
        assert_eq!(c.minutes, [5, 25, 45].into());
        assert_eq!(c.weekdays, [0].into(), "7 is Sunday");
    }

    #[test]
    fn cron_rejects_what_it_does_not_support() {
        for bad in [
            "0 9 * * MON",
            "0 9 * JAN *",
            "0 9 L * *",
            "0 9 * * 5#2",
            "0 9 ? * *",
            "60 * * * *",
            "* 24 * * *",
            "* * 0 * *",
            "* * * 13 *",
            "* * * * 8",
            "*/0 * * * *",
            "5-1 * * * *",
            "0 9 * *",
            "0 9 * * * *",
            "@daily",
            "0,,5 * * * *",
        ] {
            let err = parse_spec(&format!("cron {bad}"), 60)
                .err()
                .or_else(|| parse_spec(bad, 60).err());
            assert!(err.is_some(), "{bad} should fail");
        }
        let err = parse_spec("cron 0 9 * * MON", 60).unwrap_err();
        assert!(err.contains("names (MON, JAN)"), "{err}");
    }

    #[test]
    fn at_parses_dates_instants_and_bare_times() {
        let at = parse_spec("at 2026-10-12 15:00", 60).unwrap();
        assert_eq!(at.to_string(), "at 2026-10-12 15:00");
        assert!(at.is_one_shot());
        let instant = parse_spec("at 2026-10-12T15:00:00+02:00", 60).unwrap();
        assert_eq!(
            instant,
            ScheduleSpec::AtInstant(utc("2026-10-12T13:00:00Z"))
        );
        assert!(parse_spec("at tomorrow", 60).is_err());
        assert!(parse_spec("sometimes", 60).is_err());
        assert!(parse_spec("", 60).is_err());
    }

    #[test]
    fn normalize_pins_a_bare_time_and_refuses_the_past() {
        let tz = TestEastern;
        // 10:00 UTC = 06:00 EDT on 2026-10-12.
        let now = utc("2026-10-12T10:00:00Z");
        assert_eq!(
            normalize_spec("at 15:00", 60, now, &tz).unwrap(),
            "at 2026-10-12 15:00"
        );
        assert_eq!(
            normalize_spec("at 05:00", 60, now, &tz).unwrap(),
            "at 2026-10-13 05:00"
        );
        let err = normalize_spec("at 2026-10-11 09:00", 60, now, &tz).unwrap_err();
        assert!(err.contains("already passed"), "{err}");
        assert_eq!(
            normalize_spec("every 90m", 60, now, &tz).unwrap(),
            "every 1h30m"
        );
    }

    #[test]
    fn every_fires_one_interval_after_the_previous_fire() {
        let tz = TestEastern;
        let mut s = schedule("every 30m");
        let now = utc("2026-10-12T10:20:00Z");
        assert_eq!(s.next_fire(now, &tz), Some(utc("2026-10-12T10:30:00Z")));
        s.last_fire = Some(utc("2026-10-12T10:31:07Z"));
        assert_eq!(s.next_fire(now, &tz), Some(utc("2026-10-12T11:01:07Z")));
    }

    #[test]
    fn daily_next_fire_is_local_and_crosses_midnight() {
        let tz = TestEastern;
        let spec = parse_spec("daily 09:00", 60).unwrap();
        // 12:00 UTC = 08:00 EDT: today 09:00 EDT = 13:00 UTC.
        assert_eq!(
            spec.next_after(utc("2026-10-12T12:00:00Z"), &tz),
            Some(utc("2026-10-12T13:00:00Z"))
        );
        // Just after it: tomorrow.
        assert_eq!(
            spec.next_after(utc("2026-10-12T13:00:00Z"), &tz),
            Some(utc("2026-10-13T13:00:00Z"))
        );
    }

    #[test]
    fn daily_follows_the_clock_across_dst_changes() {
        let tz = TestEastern;
        let spec = parse_spec("daily 09:00", 60).unwrap();
        // Before the November change 09:00 EDT = 13:00 UTC; after it 09:00
        // EST = 14:00 UTC.
        assert_eq!(
            spec.next_after(utc("2026-10-31T13:00:00Z"), &tz),
            Some(utc("2026-11-01T14:00:00Z"))
        );
        // March: 09:00 EST = 14:00 UTC on the 7th, 09:00 EDT = 13:00 UTC on the 8th.
        assert_eq!(
            spec.next_after(utc("2026-03-07T14:00:00Z"), &tz),
            Some(utc("2026-03-08T13:00:00Z"))
        );
    }

    #[test]
    fn a_skipped_local_time_runs_at_the_first_minute_after_the_gap() {
        let tz = TestEastern;
        // 2026-03-08 02:30 local does not exist (02:00 → 03:00).
        let spec = parse_spec("daily 02:30", 60).unwrap();
        let next = spec.next_after(utc("2026-03-07T12:00:00Z"), &tz).unwrap();
        // 03:00 EDT = 07:00 UTC.
        assert_eq!(next, utc("2026-03-08T07:00:00Z"));
        // And the day after it is 02:30 EDT again.
        assert_eq!(
            spec.next_after(next, &tz),
            Some(utc("2026-03-09T06:30:00Z"))
        );
    }

    #[test]
    fn a_repeated_local_time_fires_once() {
        let tz = TestEastern;
        // 2026-11-01 01:30 local happens twice (EDT 05:30 UTC, EST 06:30 UTC).
        let spec = parse_spec("daily 01:30", 60).unwrap();
        let first = spec.next_after(utc("2026-10-31T12:00:00Z"), &tz).unwrap();
        assert_eq!(first, utc("2026-11-01T05:30:00Z"), "the earlier one");
        let second = spec.next_after(first, &tz).unwrap();
        assert_eq!(second, utc("2026-11-02T06:30:00Z"), "then the next day");
    }

    #[test]
    fn cron_handles_month_ends_weekdays_and_the_or_rule() {
        let tz = TestEastern;
        // The 31st: skips 30-day months.
        let spec = parse_spec("cron 0 12 31 * *", 60).unwrap();
        assert_eq!(
            spec.next_after(utc("2026-10-31T17:00:00Z"), &tz),
            Some(utc("2026-12-31T17:00:00Z"))
        );
        // Feb 29 exists in 2028.
        let leap = parse_spec("cron 0 0 29 2 *", 60).unwrap();
        let next = leap.next_after(utc("2026-10-12T00:00:00Z"), &tz).unwrap();
        assert_eq!(
            next.with_timezone(&tz).date_naive(),
            NaiveDate::from_ymd_opt(2028, 2, 29).unwrap()
        );
        // Weekdays only: Friday 2026-10-16 09:00 → Monday the 19th.
        let weekdays = parse_spec("cron 0 9 * * 1-5", 60).unwrap();
        assert_eq!(
            weekdays.next_after(utc("2026-10-16T13:00:00Z"), &tz),
            Some(utc("2026-10-19T13:00:00Z"))
        );
        // Day of month OR day of week: the 1st or any Sunday.
        let either = parse_spec("cron 0 9 1 * 0", 60).unwrap();
        let after_sat = utc("2026-10-17T14:00:00Z");
        assert_eq!(
            either
                .next_after(after_sat, &tz)
                .unwrap()
                .with_timezone(&tz)
                .date_naive(),
            NaiveDate::from_ymd_opt(2026, 10, 18).unwrap(),
            "Sunday the 18th"
        );
        // Every 15 minutes.
        let quarter = parse_spec("*/15 * * * *", 60).unwrap();
        assert_eq!(
            quarter.next_after(utc("2026-10-12T10:07:30Z"), &tz),
            Some(utc("2026-10-12T10:15:00Z"))
        );
    }

    #[test]
    fn due_tells_missed_from_now_and_honours_limits() {
        let tz = TestEastern;
        let started = utc("2026-10-12T12:00:00Z");
        let mut s = schedule("every 1h");
        // Created 10:00, next 11:00, which is before the start: missed.
        assert_eq!(
            due(&s, started, started, &tz),
            Some(Due::Missed(utc("2026-10-12T11:00:00Z")))
        );
        s.last_fire = Some(utc("2026-10-12T12:05:00Z"));
        assert_eq!(due(&s, utc("2026-10-12T13:00:00Z"), started, &tz), None);
        assert_eq!(
            due(&s, utc("2026-10-12T13:05:00Z"), started, &tz),
            Some(Due::Now(utc("2026-10-12T13:05:00Z")))
        );
        s.paused = true;
        assert_eq!(due(&s, utc("2026-10-12T14:00:00Z"), started, &tz), None);
        s.paused = false;
        s.max_runs = Some(2);
        s.runs = 2;
        assert!(s.finished(utc("2026-10-12T14:00:00Z")));
        assert_eq!(due(&s, utc("2026-10-12T14:00:00Z"), started, &tz), None);
        s.max_runs = None;
        s.end_at = Some(utc("2026-10-12T13:00:00Z"));
        let before_end = utc("2026-10-12T12:30:00Z");
        assert!(!s.finished(before_end));
        assert_eq!(
            s.next_fire(before_end, &tz),
            None,
            "the next fire is past the end"
        );
    }

    #[test]
    fn a_one_shot_finishes_after_its_fire() {
        let tz = TestEastern;
        let mut s = schedule("at 2026-10-12 08:00");
        // 08:00 EDT = 12:00 UTC.
        assert_eq!(
            s.next_fire(utc("2026-10-12T11:00:00Z"), &tz),
            Some(utc("2026-10-12T12:00:00Z"))
        );
        s.last_fire = Some(utc("2026-10-12T12:00:01Z"));
        assert!(s.finished(utc("2026-10-12T12:01:00Z")));
        assert_eq!(s.next_fire(utc("2026-10-12T12:01:00Z"), &tz), None);
    }

    #[test]
    fn outcomes_label_and_count() {
        assert_eq!(Outcome::SkippedWaiting.label(), "skipped: waiting on you");
        assert_eq!(
            Outcome::SkippedBusy("vim".into()).label(),
            "skipped: busy (vim)"
        );
        assert!(Outcome::Queued.counts_as_run());
        assert!(Outcome::Started.counts_as_run());
        assert!(!Outcome::SkippedNotRunning.counts_as_run());
        assert!(!Outcome::Missed.counts_as_run());
        assert!(!Outcome::Failed("x".into()).counts_as_run());
    }

    #[test]
    fn config_entries_validate_and_match_features() {
        let entry: ScheduleConfig = serde_json::from_str(
            r#"{"name":"nightly","match":"api-*","when":"daily 02:00","command":"make test"}"#,
        )
        .unwrap();
        assert_eq!(
            entry.validate(60),
            Ok((ScheduleKind::Shell, "make test".into()))
        );
        assert!(entry.applies_to("api-auth", "x"));
        assert!(
            entry.applies_to("other", "api-fix"),
            "matches the branch too"
        );
        assert!(!entry.applies_to("web", "web"));
        let both = ScheduleConfig {
            name: "b".into(),
            spec: "every 1h".into(),
            prompt: Some("p".into()),
            command: Some("c".into()),
            ..Default::default()
        };
        assert!(both.validate(60).unwrap_err().contains("both"));
        let none = ScheduleConfig {
            name: "n".into(),
            spec: "every 1h".into(),
            ..Default::default()
        };
        assert!(none.validate(60).unwrap_err().contains("needs a prompt"));
        let once = ScheduleConfig {
            name: "o".into(),
            spec: "at 2030-01-01 10:00".into(),
            prompt: Some("p".into()),
            ..Default::default()
        };
        assert!(once.validate(60).unwrap_err().contains("recur"));
        let nameless = ScheduleConfig {
            spec: "every 1h".into(),
            prompt: Some("p".into()),
            ..Default::default()
        };
        assert!(nameless.validate(60).is_err());
        let all = ScheduleConfig::default();
        assert!(
            all.applies_to("anything", "anything"),
            "no match: every feature"
        );
    }

    #[test]
    fn glob_matching() {
        assert!(glob_match("*", ""));
        assert!(glob_match("api-*", "api-"));
        assert!(glob_match("a?c", "abc"));
        assert!(!glob_match("a?c", "ac"));
        assert!(glob_match("*fix*", "hotfix-1"));
        assert!(!glob_match("feat", "feature"));
        assert!(glob_match("feat*", "feature"));
    }

    #[test]
    fn end_times_parse() {
        let tz = TestEastern;
        assert_eq!(
            parse_end("2026-10-31 18:00").unwrap().in_zone(&tz),
            utc("2026-10-31T22:00:00Z")
        );
        assert_eq!(
            parse_end("2026-10-31").unwrap().in_zone(&tz),
            utc("2026-11-01T03:59:00Z")
        );
        assert!(parse_end("soon").is_err());
    }

    #[test]
    fn short_when_is_relative_to_today() {
        let tz = TestEastern;
        let now = utc("2026-10-12T14:00:00Z");
        assert_eq!(short_when(utc("2026-10-12T18:30:00Z"), now, &tz), "14:30");
        assert_eq!(
            short_when(utc("2026-10-13T18:30:00Z"), now, &tz),
            "Tue 14:30"
        );
        assert_eq!(
            short_when(utc("2026-12-13T18:30:00Z"), now, &tz),
            "2026-12-13 13:30"
        );
    }

    #[test]
    fn durations_describe() {
        assert_eq!(describe_duration(1800), "30 min");
        assert_eq!(describe_duration(5400), "1 h 30 min");
        assert_eq!(describe_duration(45), "45 s");
        assert_eq!(parse_spec("every 2h", 60).unwrap().describe(), "every 2 h");
    }
}
