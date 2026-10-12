//! FORK: messages to agent sessions without typing into their panes
//! (DESIGN.md §19.5).
//!
//! Claude Code (2.1.224+) binds a per-session inbox: a Unix domain socket
//! whose path it exports to hooks as `CLAUDE_CODE_MESSAGING_SOCKET`, with a
//! per-session token in `CLAUDE_CODE_MESSAGING_TOKEN`. The `SessionStart`
//! hook (`aiw hook-event --kind session-start`) forwards both, so AIW knows
//! every Claude session's inbox. A message is newline-delimited JSON on that
//! socket:
//!
//! ```text
//! {"type":"auth","token":"<CLAUDE_CODE_MESSAGING_TOKEN>"}        (optional on Linux)
//! {"type":"user","message":{"role":"user","content":"<text>"},"from":"<sender>","uuid":"<id>","session_id":"<conversation>"}
//! ```
//!
//! The shape is the one Claude Code's own inbox handler accepts (read from the
//! installed 2.1.296 bundle; see DESIGN.md §19.5): `type: "user"` with a
//! string `message.content` is queued as the session's next prompt, `from` is
//! shown as the sender, and a `session_id` that is not the receiving
//! conversation makes Claude drop the line — so a socket path that outlived
//! its session (or a pid reused by another Claude) never gets the message.
//!
//! Everything without an inbox (Codex, older Claude, a Claude whose hook never
//! reported one) falls back to AIW's follow-up queue: delivered when the agent
//! next finishes a turn.
//!
//! Pure apart from [`post`], which does the socket write on a helper thread so
//! a stuck listener can never freeze the dashboard.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::project::VibeMode;

/// The automation action `aiw automation message` sends.
pub const MESSAGE_ACTION: &str = "message_session";

/// The sender name AIW uses when no session is named as the sender.
pub const AIW_SENDER: &str = "aiw";

/// Longest message accepted (Claude caps a same-machine message at about a
/// million characters; AIW keeps well under it).
pub const MAX_MESSAGE_CHARS: usize = 64 * 1024;

/// How long a post may take before it counts as failed.
pub const POST_TIMEOUT: Duration = Duration::from_secs(2);

/// Where a Claude session's inbox is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxAddress {
    /// `CLAUDE_CODE_MESSAGING_SOCKET`.
    pub socket: PathBuf,
    /// `CLAUDE_CODE_MESSAGING_TOKEN`. Kept in memory only (never persisted or
    /// logged): auth is optional on Linux, so an address reloaded after a
    /// restart still works without it.
    pub token: Option<String>,
    /// The Claude conversation the socket belongs to; sent as `session_id`
    /// so Claude drops the line if the socket now serves someone else.
    pub conversation_id: Option<String>,
}

impl InboxAddress {
    /// Build from a `session-start` hook event's forwarded fields. `None`
    /// when no usable socket path came with it.
    pub fn from_hook_event(raw: &Value) -> Option<Self> {
        let text = |key: &str| {
            raw.get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(str::to_string)
        };
        let socket = text("messaging_socket")?;
        let socket = PathBuf::from(socket);
        if !socket.is_absolute() {
            return None;
        }
        Some(Self {
            socket,
            token: text("messaging_token").filter(|t| valid_token(t)),
            conversation_id: text("session_id"),
        })
    }
}

/// A token as Claude issues it: 32 lowercase hex characters. Anything else
/// is not sent (it would make Claude close the connection where auth is
/// required).
pub fn valid_token(token: &str) -> bool {
    token.len() == 32 && token.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The optional first line.
pub fn auth_line(token: &str) -> String {
    json!({ "type": "auth", "token": token }).to_string()
}

/// The message line.
pub fn user_line(text: &str, from: &str, conversation_id: Option<&str>, uuid: &str) -> String {
    let mut line = json!({
        "type": "user",
        "message": { "role": "user", "content": text },
        "from": from,
        "uuid": uuid,
    });
    if let Some(id) = conversation_id.filter(|id| !id.is_empty()) {
        line["session_id"] = Value::String(id.to_string());
    }
    line.to_string()
}

/// The lines one post writes, in order.
pub fn post_lines(address: &InboxAddress, text: &str, from: &str, uuid: &str) -> Vec<String> {
    let mut lines = Vec::with_capacity(2);
    if let Some(token) = address.token.as_deref().filter(|t| valid_token(t)) {
        lines.push(auth_line(token));
    }
    lines.push(user_line(
        text,
        from,
        address.conversation_id.as_deref(),
        uuid,
    ));
    lines
}

/// Why a post did not reach the inbox.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PostError {
    /// No socket at the path (the session exited).
    Missing,
    /// Something is at the path but it is not a socket owned by this user.
    NotOurSocket,
    /// Nothing listens (a stale socket file).
    Refused,
    /// The connect or write took longer than [`POST_TIMEOUT`].
    Timeout,
    Io(String),
}

impl std::fmt::Display for PostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PostError::Missing => write!(f, "the inbox socket is gone"),
            PostError::NotOurSocket => write!(f, "the inbox path is not a socket you own"),
            PostError::Refused => write!(f, "nothing is listening on the inbox socket"),
            PostError::Timeout => write!(f, "the inbox did not accept the message in time"),
            PostError::Io(err) => write!(f, "{err}"),
        }
    }
}

fn classify(err: &std::io::Error) -> PostError {
    match err.kind() {
        std::io::ErrorKind::NotFound => PostError::Missing,
        std::io::ErrorKind::ConnectionRefused => PostError::Refused,
        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => PostError::Timeout,
        _ => PostError::Io(err.to_string()),
    }
}

/// Check the path is a socket owned by the current user before connecting.
pub fn check_socket(path: &Path) -> Result<(), PostError> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) => return Err(classify(&err)),
    };
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    if !meta.file_type().is_socket() || meta.uid() != uid {
        return Err(PostError::NotOurSocket);
    }
    Ok(())
}

/// Write `lines` to the socket at `path` (connect, write, close). The
/// work runs on a helper thread and is abandoned after `timeout`, so a
/// listener with a full backlog cannot block the caller.
pub fn post_raw(path: &Path, lines: &[String], timeout: Duration) -> Result<(), PostError> {
    check_socket(path)?;
    let mut payload = lines.join("\n");
    payload.push('\n');
    let path = path.to_path_buf();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("aiw-inbox-post".into())
        .spawn(move || {
            let result = (|| -> std::io::Result<()> {
                let mut stream = UnixStream::connect(&path)?;
                stream.set_write_timeout(Some(timeout))?;
                stream.write_all(payload.as_bytes())?;
                stream.flush()?;
                // Dropping the stream closes it: Claude reads the last line
                // on end-of-stream.
                Ok(())
            })();
            let _ = tx.send(result);
        })
        .map_err(|err| PostError::Io(err.to_string()))?;
    match rx.recv_timeout(timeout) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(err)) => Err(classify(&err)),
        Err(_) => Err(PostError::Timeout),
    }
}

/// Post `text` to a Claude session's inbox.
pub fn post(address: &InboxAddress, text: &str, from: &str) -> Result<(), PostError> {
    let uuid = uuid::Uuid::new_v4().to_string();
    post_raw(
        &address.socket,
        &post_lines(address, text, from, &uuid),
        POST_TIMEOUT,
    )
}

/// How a message will travel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// Straight into the Claude session's inbox socket.
    Inbox,
    /// AIW's follow-up queue, delivered on the agent's next turn end.
    Queue(&'static str),
}

/// Pick the path: the inbox when the target is a Claude session with a known
/// socket and inbox delivery is on, the follow-up queue otherwise.
pub fn route(is_claude: bool, has_inbox: bool, inbox_enabled: bool) -> Route {
    if !is_claude {
        Route::Queue("this harness has no inbox")
    } else if !inbox_enabled {
        Route::Queue("inbox delivery is off (messaging_inbox in config.json)")
    } else if !has_inbox {
        Route::Queue("no inbox reported for this session yet")
    } else {
        Route::Inbox
    }
}

/// What happened to a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Delivery {
    /// Written to the Claude session's inbox.
    Inbox,
    /// Queued as a follow-up at `position` (1-based).
    Queued {
        position: usize,
        reason: String,
    },
    Failed(String),
}

impl Delivery {
    /// `delivered`, `queued` or `failed` (the automation result).
    pub fn status(&self) -> &'static str {
        match self {
            Delivery::Inbox => "delivered",
            Delivery::Queued { .. } => "queued",
            Delivery::Failed(_) => "failed",
        }
    }

    /// One line for a toast or the automation result.
    pub fn summary(&self, target: &str) -> String {
        match self {
            Delivery::Inbox => format!("Sent to {target}'s inbox"),
            Delivery::Queued { position, reason } => {
                format!("Queued for {target} (#{position}, next turn end): {reason}")
            }
            Delivery::Failed(reason) => format!("Could not message {target}: {reason}"),
        }
    }
}

/// The text a follow-up carries when a message falls back to the queue: the
/// sender is named, since the queue pastes it as a plain prompt.
pub fn queued_text(text: &str, from: &str) -> String {
    if from == AIW_SENDER {
        text.to_string()
    } else {
        format!("Message from {from}:\n{text}")
    }
}

/// The sender name for a message from session `name` (or from AIW itself).
pub fn sender(name: Option<&str>) -> String {
    name.map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| AIW_SENDER.to_string())
}

/// Check message text before it goes anywhere.
pub fn validate_text(text: &str) -> Result<&str, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("the message is empty".into());
    }
    if text.chars().count() > MAX_MESSAGE_CHARS {
        return Err(format!(
            "the message is longer than {MAX_MESSAGE_CHARS} characters"
        ));
    }
    Ok(text)
}

/// A session a message can be addressed to.
#[derive(Debug, Clone)]
pub struct TargetCandidate<'a> {
    pub id: &'a str,
    /// The `--name` it was launched with (Claude, §19.2).
    pub harness_name: Option<&'a str>,
    pub feature: &'a str,
    pub label: &'a str,
}

/// Find the session `query` names: its id, its harness name, its derived
/// `<feature>-<label>` name, `<feature>/<label>`, or a label unique across
/// AIW. Case-insensitive except for ids.
pub fn resolve_target(candidates: &[TargetCandidate<'_>], query: &str) -> Result<String, String> {
    let query = query.trim();
    if query.is_empty() {
        return Err("no target session given".into());
    }
    if let Some(c) = candidates.iter().find(|c| c.id == query) {
        return Ok(c.id.to_string());
    }
    let lower = query.to_lowercase();
    let pick = |matches: Vec<&TargetCandidate<'_>>| -> Option<Result<String, String>> {
        match matches.len() {
            0 => None,
            1 => Some(Ok(matches[0].id.to_string())),
            _ => Some(Err(format!(
                "'{query}' names {} sessions: {}",
                matches.len(),
                matches
                    .iter()
                    .take(4)
                    .map(|c| format!("{}/{}", c.feature, c.label))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))),
        }
    };
    let by_name: Vec<_> = candidates
        .iter()
        .filter(|c| c.harness_name.is_some_and(|n| n.to_lowercase() == lower))
        .collect();
    if let Some(result) = pick(by_name) {
        return result;
    }
    let by_base: Vec<_> = candidates
        .iter()
        .filter(|c| crate::fork::session_identity::base_name(c.feature, c.label) == lower)
        .collect();
    if let Some(result) = pick(by_base) {
        return result;
    }
    if let Some((feature, label)) = query.split_once(['/', ':']) {
        let (feature, label) = (feature.trim().to_lowercase(), label.trim().to_lowercase());
        let by_pair: Vec<_> = candidates
            .iter()
            .filter(|c| c.feature.to_lowercase() == feature && c.label.to_lowercase() == label)
            .collect();
        if let Some(result) = pick(by_pair) {
            return result;
        }
    }
    let by_label: Vec<_> = candidates
        .iter()
        .filter(|c| c.label.to_lowercase() == lower)
        .collect();
    pick(by_label).unwrap_or_else(|| Err(format!("no agent session named '{query}'")))
}

/// What AIW sets as Claude's `crossSessionInbound` for the sessions it
/// launches (`claude_cross_session_inbound` in config.json).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InboundPolicy {
    /// Deliver messages from your other sessions and from AIW. A SuperVibe
    /// session keeps Claude's own default instead (see [`settings_value`]).
    #[default]
    Accept,
    /// Show each message and wait for approval in the pane.
    Hold,
    /// Drop every message.
    Refuse,
    /// Set nothing; Claude decides from the two sessions' permission modes.
    Default,
}

impl InboundPolicy {
    fn code(self) -> u8 {
        match self {
            InboundPolicy::Accept => 1,
            InboundPolicy::Hold => 2,
            InboundPolicy::Refuse => 3,
            InboundPolicy::Default => 4,
        }
    }

    fn from_code(code: u8) -> Self {
        match code {
            2 => InboundPolicy::Hold,
            3 => InboundPolicy::Refuse,
            4 => InboundPolicy::Default,
            _ => InboundPolicy::Accept,
        }
    }
}

/// The `crossSessionInbound` value to put in a launch's `--settings`, if any.
///
/// `accept` is not applied to a SuperVibe (bypass-permissions) session: there
/// a message from another session would run without any permission prompt,
/// which is exactly the case Claude's default holds for approval. `hold` and
/// `refuse` apply to every mode.
pub fn settings_value(policy: InboundPolicy, mode: &VibeMode) -> Option<&'static str> {
    match policy {
        InboundPolicy::Accept if matches!(mode, VibeMode::SuperVibe) => None,
        InboundPolicy::Accept => Some("accept"),
        InboundPolicy::Hold => Some("hold"),
        InboundPolicy::Refuse => Some("refuse"),
        InboundPolicy::Default => None,
    }
}

static POLICY: AtomicU8 = AtomicU8::new(0);

#[cfg(test)]
thread_local! {
    static TEST_POLICY: std::cell::Cell<Option<InboundPolicy>> = const { std::cell::Cell::new(None) };
}

/// Decide the policy for this process (from the config, at start).
pub fn set_inbound_policy(policy: InboundPolicy) {
    POLICY.store(policy.code(), Ordering::Relaxed);
}

/// The policy launches use.
pub fn inbound_policy() -> InboundPolicy {
    #[cfg(test)]
    if let Some(forced) = TEST_POLICY.with(|cell| cell.get()) {
        return forced;
    }
    InboundPolicy::from_code(POLICY.load(Ordering::Relaxed))
}

/// Force the policy for the current test thread.
#[cfg(test)]
pub fn force_policy_for_test(policy: Option<InboundPolicy>) {
    TEST_POLICY.with(|cell| cell.set(policy));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixListener;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    fn address(socket: PathBuf) -> InboxAddress {
        InboxAddress {
            socket,
            token: Some(TOKEN.into()),
            conversation_id: Some("conv-1".into()),
        }
    }

    #[test]
    fn the_lines_are_the_shape_claudes_inbox_reads() {
        let lines = post_lines(&address("/x.sock".into()), "hi there", "api-worker", "u-1");
        assert_eq!(lines.len(), 2);
        let auth: Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(auth, json!({"type": "auth", "token": TOKEN}));
        let user: Value = serde_json::from_str(&lines[1]).unwrap();
        assert_eq!(user["type"], "user");
        assert_eq!(user["message"]["role"], "user");
        assert_eq!(user["message"]["content"], "hi there");
        assert_eq!(user["from"], "api-worker");
        assert_eq!(user["uuid"], "u-1");
        assert_eq!(user["session_id"], "conv-1");
        // One JSON object per line: a newline in the text stays escaped.
        let multi = user_line("a\nb", "aiw", None, "u");
        assert!(!multi.contains('\n'));
        assert!(
            serde_json::from_str::<Value>(&multi)
                .unwrap()
                .get("session_id")
                .is_none()
        );
    }

    #[test]
    fn a_missing_or_malformed_token_sends_no_auth_line() {
        let mut addr = address("/x.sock".into());
        addr.token = None;
        assert_eq!(post_lines(&addr, "t", "aiw", "u").len(), 1);
        addr.token = Some("not-hex-and-short".into());
        assert_eq!(post_lines(&addr, "t", "aiw", "u").len(), 1);
        assert!(valid_token(TOKEN));
        assert!(!valid_token(&TOKEN[1..]));
    }

    #[test]
    fn the_hook_event_carries_the_address() {
        let raw = json!({"session_id": "conv-9", "messaging_socket": "/run/cc-socks/1.sock", "messaging_token": TOKEN});
        let addr = InboxAddress::from_hook_event(&raw).unwrap();
        assert_eq!(addr.socket, PathBuf::from("/run/cc-socks/1.sock"));
        assert_eq!(addr.token.as_deref(), Some(TOKEN));
        assert_eq!(addr.conversation_id.as_deref(), Some("conv-9"));
        assert!(InboxAddress::from_hook_event(&json!({"session_id": "c"})).is_none());
        assert!(InboxAddress::from_hook_event(&json!({"messaging_socket": "rel.sock"})).is_none());
        let bad_token = json!({"messaging_socket": "/s.sock", "messaging_token": "zz"});
        assert!(
            InboxAddress::from_hook_event(&bad_token)
                .unwrap()
                .token
                .is_none()
        );
    }

    #[test]
    fn a_post_reaches_a_listening_socket_once() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("inbox.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let reader = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            BufReader::new(stream)
                .lines()
                .map(Result::unwrap)
                .collect::<Vec<_>>()
        });
        post(&address(path), "ship it", "aiw").unwrap();
        let lines = reader.join().unwrap();
        assert_eq!(lines.len(), 2);
        let user: Value = serde_json::from_str(&lines[1]).unwrap();
        assert_eq!(user["message"]["content"], "ship it");
    }

    #[test]
    fn failures_are_told_apart() {
        let dir = tempfile::TempDir::new().unwrap();
        // Gone.
        let gone = dir.path().join("gone.sock");
        assert_eq!(post(&address(gone), "x", "aiw"), Err(PostError::Missing));
        // A plain file is not a socket.
        let file = dir.path().join("file.sock");
        std::fs::write(&file, "").unwrap();
        assert_eq!(
            post(&address(file), "x", "aiw"),
            Err(PostError::NotOurSocket)
        );
        // A socket file nobody listens on.
        let stale = dir.path().join("stale.sock");
        drop(UnixListener::bind(&stale).unwrap());
        assert_eq!(post(&address(stale), "x", "aiw"), Err(PostError::Refused));
    }

    #[test]
    fn a_listener_that_never_accepts_times_out_without_blocking_the_caller() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("slow.sock");
        let _listener = UnixListener::bind(&path).unwrap();
        // A payload far bigger than the socket buffer: the write blocks
        // because nobody reads, and the post gives up at the timeout.
        let huge = vec!["x".repeat(4 * 1024 * 1024)];
        let started = std::time::Instant::now();
        let result = post_raw(&path, &huge, Duration::from_millis(300));
        assert_eq!(result, Err(PostError::Timeout));
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn routing_prefers_the_inbox_and_falls_back_to_the_queue() {
        assert_eq!(route(true, true, true), Route::Inbox);
        assert!(matches!(route(false, true, true), Route::Queue(_)));
        assert!(matches!(route(true, false, true), Route::Queue(_)));
        assert!(matches!(route(true, true, false), Route::Queue(_)));
    }

    #[test]
    fn delivery_reports_a_status_and_a_line() {
        assert_eq!(Delivery::Inbox.status(), "delivered");
        let queued = Delivery::Queued {
            position: 2,
            reason: "no inbox".into(),
        };
        assert_eq!(queued.status(), "queued");
        assert!(queued.summary("auth-codex-1").contains("#2"));
        assert_eq!(Delivery::Failed("x".into()).status(), "failed");
    }

    #[test]
    fn the_queue_names_the_sender_except_for_aiw_itself() {
        assert_eq!(queued_text("do it", AIW_SENDER), "do it");
        assert_eq!(queued_text("do it", "api"), "Message from api:\ndo it");
        assert_eq!(sender(None), "aiw");
        assert_eq!(sender(Some("  ")), "aiw");
        assert_eq!(sender(Some("api")), "api");
    }

    #[test]
    fn text_is_checked() {
        assert!(validate_text("  ").is_err());
        assert_eq!(validate_text(" hi ").unwrap(), "hi");
        assert!(validate_text(&"x".repeat(MAX_MESSAGE_CHARS + 1)).is_err());
    }

    fn candidates() -> Vec<TargetCandidate<'static>> {
        vec![
            TargetCandidate {
                id: "s1",
                harness_name: Some("auth-fix-claude-1"),
                feature: "auth-fix",
                label: "Claude 1",
            },
            TargetCandidate {
                id: "s2",
                harness_name: None,
                feature: "auth-fix",
                label: "Codex 1",
            },
            TargetCandidate {
                id: "s3",
                harness_name: Some("billing-claude-1"),
                feature: "billing",
                label: "Claude 1",
            },
        ]
    }

    #[test]
    fn targets_resolve_by_id_name_derived_name_pair_or_unique_label() {
        let c = candidates();
        assert_eq!(resolve_target(&c, "s2").unwrap(), "s2");
        assert_eq!(resolve_target(&c, "AUTH-FIX-CLAUDE-1").unwrap(), "s1");
        assert_eq!(resolve_target(&c, "auth-fix-codex-1").unwrap(), "s2");
        assert_eq!(resolve_target(&c, "billing/claude 1").unwrap(), "s3");
        assert_eq!(resolve_target(&c, "auth-fix:Codex 1").unwrap(), "s2");
        assert_eq!(resolve_target(&c, "codex 1").unwrap(), "s2");
        let ambiguous = resolve_target(&c, "Claude 1").unwrap_err();
        assert!(ambiguous.contains("2 sessions"), "{ambiguous}");
        assert!(
            resolve_target(&c, "nope")
                .unwrap_err()
                .contains("no agent session")
        );
        assert!(resolve_target(&c, " ").is_err());
    }

    #[test]
    fn accept_is_not_applied_to_a_bypass_session() {
        use InboundPolicy::*;
        assert_eq!(settings_value(Accept, &VibeMode::Vibe), Some("accept"));
        assert_eq!(settings_value(Accept, &VibeMode::Vibeless), Some("accept"));
        assert_eq!(settings_value(Accept, &VibeMode::SuperVibe), None);
        assert_eq!(settings_value(Hold, &VibeMode::SuperVibe), Some("hold"));
        assert_eq!(settings_value(Refuse, &VibeMode::Vibe), Some("refuse"));
        assert_eq!(settings_value(Default, &VibeMode::Vibe), None);
    }

    #[test]
    fn the_policy_round_trips_through_config_and_the_process_flag() {
        let p: InboundPolicy = serde_json::from_str("\"hold\"").unwrap();
        assert_eq!(p, InboundPolicy::Hold);
        assert_eq!(
            serde_json::to_string(&InboundPolicy::Default).unwrap(),
            "\"default\""
        );
        for policy in [
            InboundPolicy::Accept,
            InboundPolicy::Hold,
            InboundPolicy::Refuse,
            InboundPolicy::Default,
        ] {
            assert_eq!(InboundPolicy::from_code(policy.code()), policy);
        }
        force_policy_for_test(Some(InboundPolicy::Refuse));
        assert_eq!(inbound_policy(), InboundPolicy::Refuse);
        force_policy_for_test(None);
    }
}
