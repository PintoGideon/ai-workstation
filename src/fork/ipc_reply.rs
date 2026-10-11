//! FORK: a request that stays open until the app answers (DESIGN.md §16).
//!
//! The IPC socket was fire-and-forget: a hook writes one JSON line and hangs
//! up. Some hooks now need an answer before they can finish — a permission
//! request waits for the user's allow/deny, a Stop hook asks whether a
//! follow-up is queued. Rather than the `notify-wait` reply-socket dance (a
//! second listener per request, in a directory), these keep the *same*
//! connection open: the payload carries `"await_reply": true` and a
//! `request_id`, the accept thread parks the stream in [`PENDING`] keyed by
//! that id and hands the message to the app as usual, and the app answers
//! with [`reply`], which writes one JSON line on that stream and closes it.
//!
//! Nothing here blocks the main loop: the stream sits in a map until the
//! app decides, and the accept thread stays in its read loop until the
//! client hangs up. A client that gives up (its own timeout, or it was
//! killed) is reported to the app as `{"type":"reply_abandoned",
//! "request_id":…}` so pending UI state can be dropped.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::Value;

/// The payload field a client sets to keep the connection open.
pub const AWAIT_REPLY_FIELD: &str = "await_reply";

/// The message the app receives when a waiting client hung up unanswered.
pub const ABANDONED_TYPE: &str = "reply_abandoned";

static PENDING: Mutex<Option<HashMap<String, UnixStream>>> = Mutex::new(None);

/// Whether `message` asks for an inline reply, and under which id.
pub(crate) fn awaited_request_id(message: &Value) -> Option<String> {
    if message.get(AWAIT_REPLY_FIELD).and_then(Value::as_bool) != Some(true) {
        return None;
    }
    message
        .get("request_id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

/// Park `stream` until the app replies to `request_id`.
pub(crate) fn register(request_id: &str, stream: UnixStream) {
    if let Ok(mut pending) = PENDING.lock() {
        pending
            .get_or_insert_with(HashMap::new)
            .insert(request_id.to_string(), stream);
    }
}

/// The client hung up: forget its stream. Returns whether it was still
/// waiting (i.e. the app never answered).
pub(crate) fn abandon(request_id: &str) -> bool {
    PENDING
        .lock()
        .ok()
        .and_then(|mut pending| pending.as_mut().and_then(|map| map.remove(request_id)))
        .is_some()
}

/// Answer `request_id` with one JSON line. `false` when nobody is waiting
/// any more (the client timed out), so the caller can log it and move on.
pub fn reply(request_id: &str, payload: &Value) -> bool {
    let stream = PENDING
        .lock()
        .ok()
        .and_then(|mut pending| pending.as_mut().and_then(|map| map.remove(request_id)));
    let Some(mut stream) = stream else {
        return false;
    };
    let line = serde_json::to_string(payload).unwrap_or_else(|_| "{}".to_string());
    // The client reads one line and hangs up; dropping this clone of the
    // stream is enough (the accept thread's reader sees the hang-up).
    writeln!(stream, "{line}")
        .and_then(|_| stream.flush())
        .is_ok()
}

/// Whether a reply for `request_id` is still possible.
#[cfg(test)]
pub fn is_pending(request_id: &str) -> bool {
    PENDING
        .lock()
        .ok()
        .and_then(|pending| pending.as_ref().map(|map| map.contains_key(request_id)))
        .unwrap_or(false)
}

/// Client side: send `payload` (given a `request_id` and the await flag if it
/// has none) and wait up to `timeout` for the app's one-line answer.
/// `Ok(None)` when the app did not answer in time or hung up without one;
/// `Err` when AIW is not listening at all.
pub fn request(path: &Path, mut payload: Value, timeout: Duration) -> Result<Option<Value>> {
    let object = payload
        .as_object_mut()
        .context("request payload must be a JSON object")?;
    if !object
        .get("request_id")
        .and_then(Value::as_str)
        .is_some_and(|id| !id.is_empty())
    {
        object.insert(
            "request_id".to_string(),
            Value::String(uuid::Uuid::new_v4().to_string()),
        );
    }
    object.insert(AWAIT_REPLY_FIELD.to_string(), Value::Bool(true));
    let line = serde_json::to_string(&payload)?;

    let mut stream = UnixStream::connect(path)
        .with_context(|| format!("AIW socket not found at {}", path.display()))?;
    writeln!(stream, "{line}").context("failed to write to the AIW socket")?;
    stream.flush().context("failed to flush the AIW socket")?;
    stream
        .set_read_timeout(Some(timeout))
        .context("failed to set the reply timeout")?;

    let mut reader = std::io::BufReader::new(stream);
    let mut answer = String::new();
    match reader.read_line(&mut answer) {
        Ok(0) => Ok(None),
        Ok(_) => Ok(serde_json::from_str(answer.trim()).ok()),
        Err(err)
            if matches!(
                err.kind(),
                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
            ) =>
        {
            Ok(None)
        }
        Err(err) => Err(err).context("failed to read the AIW reply"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_flagged_messages_with_an_id_are_awaited() {
        assert_eq!(
            awaited_request_id(&json!({"await_reply": true, "request_id": "r1"})).as_deref(),
            Some("r1")
        );
        assert!(awaited_request_id(&json!({"request_id": "r1"})).is_none());
        assert!(awaited_request_id(&json!({"await_reply": true})).is_none());
        assert!(awaited_request_id(&json!({"await_reply": "yes", "request_id": "r1"})).is_none());
    }

    #[test]
    fn a_request_is_answered_on_the_same_connection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("aiw.sock");
        let guard = crate::ipc::start(&path).unwrap();

        let client = std::thread::spawn({
            let path = path.clone();
            move || {
                request(
                    &path,
                    json!({"type": "permission_request", "request_id": "req-1"}),
                    Duration::from_secs(5),
                )
            }
        });

        let message = guard.rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(message["type"], "permission_request");
        assert_eq!(message["request_id"], "req-1");
        // The stream is parked until the app answers.
        for _ in 0..50 {
            if is_pending("req-1") {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(is_pending("req-1"));
        assert!(reply("req-1", &json!({"behavior": "allow"})));
        assert!(!is_pending("req-1"));

        let answer = client.join().unwrap().unwrap().expect("an answer");
        assert_eq!(answer["behavior"], "allow");
        // A second reply has nobody to go to.
        assert!(!reply("req-1", &json!({"behavior": "deny"})));
    }

    #[test]
    fn a_client_that_times_out_is_reported_as_abandoned() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("aiw.sock");
        let guard = crate::ipc::start(&path).unwrap();

        let answer = request(
            &path,
            json!({"type": "followup_take", "request_id": "req-2"}),
            Duration::from_millis(200),
        )
        .unwrap();
        assert!(answer.is_none(), "no reply within the timeout");

        let first = guard.rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(first["type"], "followup_take");
        let second = guard.rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(second["type"], ABANDONED_TYPE);
        assert_eq!(second["request_id"], "req-2");
        assert!(!is_pending("req-2"));
        assert!(!reply("req-2", &json!({})));
    }

    #[test]
    fn a_request_without_a_listener_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.sock");
        assert!(request(&path, json!({"type": "x"}), Duration::from_millis(50)).is_err());
    }
}
