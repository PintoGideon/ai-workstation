//! Harness-specific context-window telemetry collectors.

use crate::context_tracking::{
    ContextProvenance, ContextResetEvent, ContextResetMetadata, ContextResetReason,
    ContextUsageSample,
};
use crate::project::SessionKind;
use crate::token_tracking::SessionTokenUsage;
use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const CLAUDE_DEFAULT_CONTEXT_LIMIT: u64 = 900_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextUnavailableReason {
    UnsupportedSessionKind,
    MissingSession,
    MissingTelemetry,
    MalformedTelemetry,
}

/// Collector failures are data, not status-sync errors. Callers can retain a
/// previous snapshot and classify it as stale without aborting the sync pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContextCollectionResult {
    Collected(ContextUsageSample),
    ResetPending {
        conversation_id: Option<String>,
        event: ContextResetEvent,
    },
    Unavailable(ContextUnavailableReason),
}

pub struct ContextCollectionTarget<'a> {
    pub session_kind: &'a SessionKind,
    pub workdir: &'a Path,
    pub conversation_id: Option<&'a str>,
    /// Conversation IDs already assigned to sibling sessions of the same
    /// harness in this feature. Used only during first-time discovery.
    pub excluded_conversation_ids: &'a [String],
    pub session_created_at: DateTime<Utc>,
    pub provider_id: Option<&'a str>,
    pub model_id: Option<&'a str>,
    /// Optional direct runtime JSON, such as Claude status-line input or Pi's
    /// `get_session_stats` response. File collectors remain the fallback.
    pub runtime_payload: Option<&'a Value>,
    pub fallback_usage: Option<&'a SessionTokenUsage>,
    pub fallback_context_limit: Option<u64>,
    pub collected_at: DateTime<Utc>,
}

pub trait ContextCollector {
    fn collect(&mut self, target: ContextCollectionTarget<'_>) -> ContextCollectionResult;
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ModelLimitKey {
    harness: &'static str,
    provider: String,
    model: String,
}

pub struct SessionContextCollector {
    home_dir: Option<PathBuf>,
    model_limits: HashMap<ModelLimitKey, Option<u64>>,
    allow_model_commands: bool,
    // FORK: incremental transcript reads; the sweep re-read whole files every 5 s.
    claude_scans: crate::fork::line_fold::LineFoldCache<ClaudeScan>,
    codex_scans: crate::fork::line_fold::LineFoldCache<CodexScan>,
}

impl Default for SessionContextCollector {
    fn default() -> Self {
        Self {
            home_dir: dirs::home_dir(),
            model_limits: HashMap::new(),
            allow_model_commands: true,
            claude_scans: Default::default(), // FORK
            codex_scans: Default::default(),
        }
    }
}

impl SessionContextCollector {
    #[cfg(test)]
    pub(crate) fn with_home(home: &Path) -> Self {
        Self {
            home_dir: Some(home.to_path_buf()),
            model_limits: HashMap::new(),
            allow_model_commands: false,
            claude_scans: Default::default(), // FORK
            codex_scans: Default::default(),
        }
    }

    fn collect_claude(&mut self, target: &ContextCollectionTarget<'_>) -> ContextCollectionResult {
        if let Some(payload) = target.runtime_payload {
            match claude_runtime_sample(payload, target) {
                Some(sample) => return ContextCollectionResult::Collected(sample),
                None if claude_runtime_is_post_reset(payload) => {
                    return ContextCollectionResult::ResetPending {
                        conversation_id: payload
                            .get("session_id")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                            .or_else(|| target.conversation_id.map(str::to_string)),
                        event: ContextResetEvent {
                            reason: ContextResetReason::Compaction,
                            detected_at: target.collected_at,
                        },
                    };
                }
                None => {}
            }
        }

        let Some(home) = self.home_dir.as_deref() else {
            return self.fallback(target, FallbackFormula::Claude);
        };
        let project_dir = home
            .join(".claude")
            .join("projects")
            .join(encode_claude_path(target.workdir));
        let transcript = target
            .conversation_id
            .map(|id| project_dir.join(format!("{id}.jsonl")))
            .filter(|path| path.is_file())
            .or_else(|| newest_file_with_extension(&project_dir, "jsonl"));
        let Some(transcript) = transcript else {
            return self.fallback(target, FallbackFormula::Claude);
        };

        // FORK: incremental read of the transcript (same result as a full read).
        let scan = self.claude_scans.fold(&transcript, claude_scan_line);
        match scan.and_then(|scan| claude_scan_result(scan, target)) {
            Some(result) => result,
            None => self.fallback_or(
                target,
                FallbackFormula::Claude,
                ContextUnavailableReason::MalformedTelemetry,
            ),
        }
    }

    fn collect_codex(&mut self, target: &ContextCollectionTarget<'_>) -> ContextCollectionResult {
        if let Some(payload) = target.runtime_payload
            && let Some(sample) = codex_runtime_sample(payload, target)
        {
            return ContextCollectionResult::Collected(sample);
        }

        let Some(home) = self.home_dir.as_deref() else {
            return self.fallback(target, FallbackFormula::Codex);
        };
        let rollout = target
            .conversation_id
            .and_then(|id| crate::codex::indexed_session_in(home, target.workdir, id))
            .map(|session| session.rollout_path)
            .or_else(|| match target.conversation_id {
                Some(_) => None,
                None => crate::codex::indexed_sessions_in(home, target.workdir)
                    .and_then(|sessions| sessions.into_iter().next())
                    .map(|session| session.rollout_path),
            })
            .or_else(|| find_codex_rollout(home, target.workdir, target.conversation_id));
        let Some(rollout) = rollout else {
            return self.fallback(target, FallbackFormula::Codex);
        };

        // FORK: incremental read of the rollout (same result as a full read).
        let scan = self.codex_scans.fold(&rollout, codex_scan_line);
        match scan.and_then(|scan| codex_scan_result(scan, target)) {
            Some(result) => result,
            None => self.fallback_or(
                target,
                FallbackFormula::Codex,
                ContextUnavailableReason::MalformedTelemetry,
            ),
        }
    }

    fn collect_pi(&mut self, target: &ContextCollectionTarget<'_>) -> ContextCollectionResult {
        if let Some(payload) = target.runtime_payload {
            match pi_runtime_sample(payload, target) {
                Some(sample) => return ContextCollectionResult::Collected(sample),
                None if pi_runtime_is_post_reset(payload) => {
                    return ContextCollectionResult::ResetPending {
                        conversation_id: payload
                            .pointer("/data/sessionId")
                            .or_else(|| payload.get("sessionId"))
                            .and_then(Value::as_str)
                            .map(str::to_string)
                            .or_else(|| target.conversation_id.map(str::to_string)),
                        event: ContextResetEvent {
                            reason: ContextResetReason::Compaction,
                            detected_at: target.collected_at,
                        },
                    };
                }
                None => {}
            }
        }

        let Some(home) = self.home_dir.as_deref() else {
            return ContextCollectionResult::Unavailable(ContextUnavailableReason::MissingSession);
        };
        let Some(path) = find_pi_session(
            home,
            target.workdir,
            target.conversation_id,
            target.excluded_conversation_ids,
            target.session_created_at,
        ) else {
            return ContextCollectionResult::Unavailable(ContextUnavailableReason::MissingSession);
        };
        let Some(record) = parse_pi_session(&path, target) else {
            return ContextCollectionResult::Unavailable(
                ContextUnavailableReason::MalformedTelemetry,
            );
        };
        if record.awaiting_post_reset {
            return ContextCollectionResult::ResetPending {
                conversation_id: record
                    .conversation_id
                    .or_else(|| target.conversation_id.map(str::to_string)),
                event: record.last_reset.unwrap_or(ContextResetEvent {
                    reason: ContextResetReason::Compaction,
                    detected_at: target.collected_at,
                }),
            };
        }
        let Some(used_tokens) = record.used_tokens else {
            return ContextCollectionResult::Unavailable(
                ContextUnavailableReason::MissingTelemetry,
            );
        };
        let provider = record.provider_id.as_deref().or(target.provider_id);
        let model = record.model_id.as_deref().or(target.model_id);
        let context_limit = target.fallback_context_limit.or_else(|| {
            provider
                .zip(model)
                .and_then(|(provider, model)| self.pi_model_limit(provider, model))
        });

        ContextCollectionResult::Collected(make_sample(
            used_tokens,
            context_limit,
            ContextProvenance::Estimated,
            record.sampled_at.unwrap_or(target.collected_at),
            target.collected_at,
            record
                .conversation_id
                .or_else(|| target.conversation_id.map(str::to_string)),
            record.last_reset,
        ))
    }

    fn fallback(
        &self,
        target: &ContextCollectionTarget<'_>,
        formula: FallbackFormula,
    ) -> ContextCollectionResult {
        self.fallback_or(
            target,
            formula,
            if target.conversation_id.is_some() {
                ContextUnavailableReason::MissingTelemetry
            } else {
                ContextUnavailableReason::MissingSession
            },
        )
    }

    fn fallback_or(
        &self,
        target: &ContextCollectionTarget<'_>,
        formula: FallbackFormula,
        unavailable: ContextUnavailableReason,
    ) -> ContextCollectionResult {
        let Some(usage) = target.fallback_usage else {
            return ContextCollectionResult::Unavailable(unavailable);
        };
        let used_tokens = match formula {
            FallbackFormula::Claude => usage
                .input_tokens
                .saturating_add(usage.cache_read_tokens)
                .saturating_add(usage.cache_write_tokens),
            // Codex cached input is a subset of input and must not be added
            // twice. Existing AIW usage is cumulative, hence Estimated.
            FallbackFormula::Codex => usage.input_tokens,
        };
        let context_limit = target.fallback_context_limit.or(match formula {
            FallbackFormula::Claude => Some(CLAUDE_DEFAULT_CONTEXT_LIMIT),
            _ => None,
        });

        ContextCollectionResult::Collected(make_sample(
            used_tokens,
            context_limit,
            ContextProvenance::Estimated,
            target.collected_at,
            target.collected_at,
            target.conversation_id.map(str::to_string),
            None,
        ))
    }

    fn pi_model_limit(&mut self, provider: &str, model: &str) -> Option<u64> {
        let key = ModelLimitKey {
            harness: "pi",
            provider: provider.to_string(),
            model: model.to_string(),
        };
        if let Some(limit) = self.model_limits.get(&key) {
            return *limit;
        }

        let limit = self
            .home_dir
            .as_deref()
            .and_then(|home| read_pi_custom_model_limit(home, provider, model))
            .or_else(|| {
                self.allow_model_commands
                    .then(|| query_pi_model_limit(provider, model))?
            });
        self.model_limits.insert(key, limit);
        limit
    }
}

impl ContextCollector for SessionContextCollector {
    fn collect(&mut self, target: ContextCollectionTarget<'_>) -> ContextCollectionResult {
        match target.session_kind {
            SessionKind::Claude => self.collect_claude(&target),
            SessionKind::Codex => self.collect_codex(&target),
            SessionKind::Pi => self.collect_pi(&target),
            _ => ContextCollectionResult::Unavailable(
                ContextUnavailableReason::UnsupportedSessionKind,
            ),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum FallbackFormula {
    Claude,
    Codex,
}

#[derive(Default)]
struct ArtifactContextRecord {
    used_tokens: Option<u64>,
    provider_id: Option<String>,
    model_id: Option<String>,
    conversation_id: Option<String>,
    sampled_at: Option<DateTime<Utc>>,
    last_reset: Option<ContextResetEvent>,
    awaiting_post_reset: bool,
}

fn make_sample(
    used_tokens: u64,
    context_limit: Option<u64>,
    provenance: ContextProvenance,
    sampled_at: DateTime<Utc>,
    checked_at: DateTime<Utc>,
    conversation_id: Option<String>,
    last_reset: Option<ContextResetEvent>,
) -> ContextUsageSample {
    ContextUsageSample {
        used_tokens,
        context_limit,
        provenance,
        sampled_at,
        checked_at,
        reset: ContextResetMetadata {
            generation: 0,
            conversation_id,
            last_reset,
        },
    }
}

fn claude_runtime_sample(
    payload: &Value,
    target: &ContextCollectionTarget<'_>,
) -> Option<ContextUsageSample> {
    let context = payload.get("context_window")?;
    // Claude reports `current_usage: null` before the first response and
    // immediately after compaction. `total_input_tokens` may simultaneously
    // be zero, but that is an unknown sample rather than fresh 0% telemetry.
    let current = context
        .get("current_usage")
        .filter(|usage| !usage.is_null())?;
    let limit = json_u64(context, "context_window_size")?;
    let used = json_u64(context, "total_input_tokens").or_else(|| {
        Some(
            json_u64(current, "input_tokens")?
                .saturating_add(json_u64(current, "cache_read_input_tokens").unwrap_or(0))
                .saturating_add(json_u64(current, "cache_creation_input_tokens").unwrap_or(0)),
        )
    })?;
    let conversation_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| target.conversation_id.map(str::to_string));

    Some(make_sample(
        used,
        Some(limit),
        ContextProvenance::Direct,
        target.collected_at,
        target.collected_at,
        conversation_id,
        None,
    ))
}

fn claude_runtime_is_post_reset(payload: &Value) -> bool {
    payload
        .pointer("/context_window/current_usage")
        .is_some_and(Value::is_null)
}

/// FORK: what a Claude transcript says, independent of when it is read:
/// timestamps stay `None` where a line had none (filled with the collection
/// time only when the result is built), so it can be folded incrementally.
#[derive(Debug, Clone, Default)]
struct ClaudeScan {
    latest: Option<(u64, Option<DateTime<Utc>>, Option<String>)>,
    last_reset: Option<(ContextResetReason, Option<DateTime<Utc>>)>,
}

fn claude_scan_line(scan: &mut ClaudeScan, line: &str) {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return;
    };
    let timestamp = json_timestamp(&value, "timestamp");
    let subtype = value.get("subtype").and_then(Value::as_str);
    if matches!(subtype, Some("compact_boundary" | "microcompact_boundary")) {
        let reason = if subtype == Some("microcompact_boundary") {
            ContextResetReason::Summarization
        } else {
            ContextResetReason::Compaction
        };
        scan.last_reset = Some((reason, timestamp));
    }
    if value.get("type").and_then(Value::as_str) != Some("assistant") {
        return;
    }
    let Some(usage) = value.pointer("/message/usage") else {
        return;
    };
    let Some(input) = json_u64(usage, "input_tokens") else {
        return;
    };
    let used = input
        .saturating_add(json_u64(usage, "cache_read_input_tokens").unwrap_or(0))
        .saturating_add(json_u64(usage, "cache_creation_input_tokens").unwrap_or(0));
    let conversation = value
        .get("sessionId")
        .or_else(|| value.get("session_id"))
        .and_then(Value::as_str)
        .map(str::to_string);
    scan.latest = Some((used, timestamp, conversation));
}

#[cfg(test)]
fn parse_claude_transcript(
    path: &Path,
    target: &ContextCollectionTarget<'_>,
) -> Option<ContextCollectionResult> {
    let content = fs::read_to_string(path).ok()?;
    let mut scan = ClaudeScan::default();
    for line in content.lines() {
        claude_scan_line(&mut scan, line);
    }
    claude_scan_result(scan, target)
}

fn claude_scan_result(
    scan: ClaudeScan,
    target: &ContextCollectionTarget<'_>,
) -> Option<ContextCollectionResult> {
    let latest = scan
        .latest
        .map(|(used, at, conversation)| (used, at.unwrap_or(target.collected_at), conversation));
    let last_reset = scan.last_reset.map(|(reason, at)| ContextResetEvent {
        reason,
        detected_at: at.unwrap_or(target.collected_at),
    });

    if let Some(event) = last_reset.as_ref()
        && latest
            .as_ref()
            .is_none_or(|(_, sampled_at, _)| *sampled_at <= event.detected_at)
    {
        return Some(ContextCollectionResult::ResetPending {
            conversation_id: latest
                .and_then(|(_, _, conversation_id)| conversation_id)
                .or_else(|| target.conversation_id.map(str::to_string)),
            event: event.clone(),
        });
    }
    let (used, sampled_at, conversation_id) = latest?;
    Some(ContextCollectionResult::Collected(make_sample(
        used,
        target
            .fallback_context_limit
            .or(Some(CLAUDE_DEFAULT_CONTEXT_LIMIT)),
        ContextProvenance::Estimated,
        sampled_at,
        target.collected_at,
        conversation_id.or_else(|| target.conversation_id.map(str::to_string)),
        last_reset,
    )))
}

fn codex_runtime_sample(
    payload: &Value,
    target: &ContextCollectionTarget<'_>,
) -> Option<ContextUsageSample> {
    let token_usage = payload
        .pointer("/tokenUsage")
        .or_else(|| payload.pointer("/params/tokenUsage"))?;
    let last = token_usage.get("last")?;
    let used = json_u64(last, "totalTokens")
        .or_else(|| json_u64(last, "total_tokens"))
        .or_else(|| {
            Some(
                json_u64(last, "inputTokens")?
                    .saturating_add(json_u64(last, "outputTokens").unwrap_or(0)),
            )
        })?;
    let limit = json_u64(token_usage, "modelContextWindow")
        .or_else(|| json_u64(token_usage, "model_context_window"))?;
    let conversation_id = payload
        .get("threadId")
        .or_else(|| payload.get("thread_id"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| target.conversation_id.map(str::to_string));

    Some(make_sample(
        used,
        Some(limit),
        ContextProvenance::Direct,
        target.collected_at,
        target.collected_at,
        conversation_id,
        None,
    ))
}

/// FORK: what a Codex rollout says, independent of when it is read (see
/// [`ClaudeScan`]).
#[derive(Debug, Clone, Default)]
struct CodexScan {
    /// The last `session_meta` id; the target's id applies when there is none.
    meta_id: Option<String>,
    latest: Option<(u64, u64, Option<DateTime<Utc>>)>,
    /// `Some(timestamp)` once a compaction was seen.
    last_reset: Option<Option<DateTime<Utc>>>,
}

fn codex_scan_line(scan: &mut CodexScan, line: &str) {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return;
    };
    let event_type = value.get("type").and_then(Value::as_str);
    let payload_type = value.pointer("/payload/type").and_then(Value::as_str);
    let timestamp = json_timestamp(&value, "timestamp");
    if event_type == Some("session_meta")
        && let Some(id) = value
            .pointer("/payload/id")
            .or_else(|| value.pointer("/payload/session_id"))
            .and_then(Value::as_str)
    {
        scan.meta_id = Some(id.to_string());
    }
    if event_type == Some("compacted") || payload_type == Some("context_compacted") {
        scan.last_reset = Some(timestamp);
    }
    if event_type != Some("event_msg") || payload_type != Some("token_count") {
        return;
    }
    let Some(info) = value.pointer("/payload/info") else {
        return;
    };
    let Some(last) = info.get("last_token_usage") else {
        return;
    };
    let Some(used) = json_u64(last, "total_tokens").or_else(|| {
        Some(
            json_u64(last, "input_tokens")?
                .saturating_add(json_u64(last, "output_tokens").unwrap_or(0)),
        )
    }) else {
        return;
    };
    let Some(limit) = json_u64(info, "model_context_window") else {
        return;
    };
    scan.latest = Some((used, limit, timestamp));
}

#[cfg(test)]
fn parse_codex_rollout(
    path: &Path,
    target: &ContextCollectionTarget<'_>,
) -> Option<ContextCollectionResult> {
    let content = fs::read_to_string(path).ok()?;
    let mut scan = CodexScan::default();
    for line in content.lines() {
        codex_scan_line(&mut scan, line);
    }
    codex_scan_result(scan, target)
}

fn codex_scan_result(
    scan: CodexScan,
    target: &ContextCollectionTarget<'_>,
) -> Option<ContextCollectionResult> {
    let conversation_id = scan
        .meta_id
        .or_else(|| target.conversation_id.map(str::to_string));
    let latest = scan
        .latest
        .map(|(used, limit, at)| (used, limit, at.unwrap_or(target.collected_at)));
    let last_reset = scan.last_reset.map(|at| ContextResetEvent {
        reason: ContextResetReason::Compaction,
        detected_at: at.unwrap_or(target.collected_at),
    });

    if let Some(event) = last_reset.as_ref()
        && latest
            .as_ref()
            .is_none_or(|(_, _, sampled_at)| *sampled_at <= event.detected_at)
    {
        return Some(ContextCollectionResult::ResetPending {
            conversation_id,
            event: event.clone(),
        });
    }
    let (used, limit, sampled_at) = latest?;
    Some(ContextCollectionResult::Collected(make_sample(
        used,
        Some(limit),
        ContextProvenance::Direct,
        sampled_at,
        target.collected_at,
        conversation_id,
        last_reset,
    )))
}

fn pi_runtime_sample(
    payload: &Value,
    target: &ContextCollectionTarget<'_>,
) -> Option<ContextUsageSample> {
    let data = payload.get("data").unwrap_or(payload);
    let context = data.get("contextUsage")?;
    let used = context.get("tokens")?.as_u64()?;
    let limit = json_u64(context, "contextWindow")?;
    let conversation_id = data
        .get("sessionId")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| target.conversation_id.map(str::to_string));

    Some(make_sample(
        used,
        Some(limit),
        ContextProvenance::Direct,
        target.collected_at,
        target.collected_at,
        conversation_id,
        None,
    ))
}

fn pi_runtime_is_post_reset(payload: &Value) -> bool {
    let data = payload.get("data").unwrap_or(payload);
    data.pointer("/contextUsage/tokens")
        .is_some_and(Value::is_null)
}

fn parse_pi_session(
    path: &Path,
    target: &ContextCollectionTarget<'_>,
) -> Option<ArtifactContextRecord> {
    let content = fs::read_to_string(path).ok()?;
    let mut record = ArtifactContextRecord::default();
    let mut saw_header = false;
    let mut usage_after_reset = false;
    let mut saw_reset = false;

    for line in content.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match value.get("type").and_then(Value::as_str) {
            Some("session") => {
                if value.get("cwd").and_then(Value::as_str)
                    != Some(target.workdir.to_string_lossy().as_ref())
                {
                    return None;
                }
                record.conversation_id =
                    value.get("id").and_then(Value::as_str).map(str::to_string);
                saw_header = true;
            }
            Some("compaction") => {
                saw_reset = true;
                usage_after_reset = false;
                record.last_reset = Some(ContextResetEvent {
                    reason: ContextResetReason::Compaction,
                    detected_at: json_timestamp(&value, "timestamp").unwrap_or(target.collected_at),
                });
            }
            Some("branch_summary") => {
                saw_reset = true;
                usage_after_reset = false;
                record.last_reset = Some(ContextResetEvent {
                    reason: ContextResetReason::Summarization,
                    detected_at: json_timestamp(&value, "timestamp").unwrap_or(target.collected_at),
                });
            }
            Some("message") => {
                let Some(message) = value.get("message") else {
                    continue;
                };
                if message.get("role").and_then(Value::as_str) != Some("assistant")
                    || matches!(
                        message.get("stopReason").and_then(Value::as_str),
                        Some("aborted" | "error")
                    )
                {
                    continue;
                }
                let Some(usage) = message.get("usage") else {
                    continue;
                };
                let Some(used) = json_u64(usage, "totalTokens")
                    .filter(|total| *total > 0)
                    .or_else(|| {
                        Some(
                            json_u64(usage, "input")?
                                .saturating_add(json_u64(usage, "output").unwrap_or(0))
                                .saturating_add(json_u64(usage, "cacheRead").unwrap_or(0))
                                .saturating_add(json_u64(usage, "cacheWrite").unwrap_or(0)),
                        )
                    })
                else {
                    continue;
                };
                record.used_tokens = Some(used);
                record.provider_id = message
                    .get("provider")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                record.model_id = message
                    .get("model")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                record.sampled_at = message
                    .get("timestamp")
                    .and_then(Value::as_i64)
                    .and_then(millis_timestamp)
                    .or_else(|| json_timestamp(&value, "timestamp"));
                if saw_reset {
                    usage_after_reset = true;
                }
            }
            _ => {}
        }
    }
    if !saw_header {
        return None;
    }
    record.awaiting_post_reset = saw_reset && !usage_after_reset;
    Some(record)
}

fn find_pi_session(
    home: &Path,
    workdir: &Path,
    id: Option<&str>,
    excluded_ids: &[String],
    created_at: DateTime<Utc>,
) -> Option<PathBuf> {
    let root = home.join(".pi").join("agent").join("sessions");
    let mut selected: Option<(PathBuf, u64, i64)> = None;
    for path in walk_files_with_extension(&root, "jsonl") {
        // FORK: read up to the header line only, not the whole session file.
        let Some(lines) = crate::fork::line_fold::file_lines(&path) else {
            continue;
        };
        let Some(header) = lines
            .map_while(Result::ok)
            .filter_map(|line| serde_json::from_str::<Value>(&line).ok())
            .find(|value| value.get("type").and_then(Value::as_str) == Some("session"))
        else {
            continue;
        };
        if header.get("cwd").and_then(Value::as_str) != Some(workdir.to_string_lossy().as_ref()) {
            continue;
        }
        let session_id = header.get("id").and_then(Value::as_str);
        if id.is_some_and(|wanted| session_id != Some(wanted)) {
            continue;
        }
        if id.is_none()
            && session_id.is_some_and(|candidate| excluded_ids.iter().any(|id| id == candidate))
        {
            continue;
        }
        let modified = modified_millis(&path).unwrap_or(0);
        let session_millis = json_timestamp(&header, "timestamp")
            .map(|timestamp| timestamp.timestamp_millis())
            .unwrap_or(modified);
        let distance = session_millis.abs_diff(created_at.timestamp_millis());
        if selected
            .as_ref()
            .is_none_or(|(_, current_distance, current_modified)| {
                distance < *current_distance
                    || (distance == *current_distance && modified > *current_modified)
            })
        {
            selected = Some((path, distance, modified));
        }
    }
    selected.map(|(path, _, _)| path)
}

fn find_codex_rollout(home: &Path, workdir: &Path, id: Option<&str>) -> Option<PathBuf> {
    let root = home.join(".codex").join("sessions");
    let mut selected: Option<(PathBuf, i64)> = None;
    for path in walk_files_with_extension(&root, "jsonl") {
        // FORK: read up to the `session_meta` line only, not the whole rollout.
        let Some(lines) = crate::fork::line_fold::file_lines(&path) else {
            continue;
        };
        let mut matched = false;
        for line in lines.map_while(Result::ok) {
            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            if value.get("type").and_then(Value::as_str) != Some("session_meta") {
                continue;
            }
            let session_id = value
                .pointer("/payload/id")
                .or_else(|| value.pointer("/payload/session_id"))
                .and_then(Value::as_str);
            let cwd = value.pointer("/payload/cwd").and_then(Value::as_str);
            matched = cwd == Some(workdir.to_string_lossy().as_ref())
                && id.is_none_or(|wanted| session_id == Some(wanted));
            break;
        }
        if !matched {
            continue;
        }
        let modified = modified_millis(&path).unwrap_or(0);
        if selected
            .as_ref()
            .is_none_or(|(_, current)| modified > *current)
        {
            selected = Some((path, modified));
        }
    }
    selected.map(|(path, _)| path)
}

fn read_pi_custom_model_limit(home: &Path, provider: &str, model: &str) -> Option<u64> {
    let value = read_json(&home.join(".pi").join("agent").join("models.json")).ok()?;
    let models = value
        .pointer(&format!(
            "/providers/{}/models",
            escape_json_pointer(provider)
        ))?
        .as_array()?;
    models.iter().find_map(|entry| {
        if entry.get("id").and_then(Value::as_str) == Some(model) {
            entry.get("contextWindow").and_then(value_u64)
        } else {
            None
        }
    })
}

fn query_pi_model_limit(provider: &str, model: &str) -> Option<u64> {
    let query = format!("{provider}/{model}");
    let output = Command::new("pi")
        .args(["--list-models", &query])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8(output.stdout).ok()?;
    stdout.lines().skip(1).find_map(|line| {
        let columns = line.split_whitespace().collect::<Vec<_>>();
        if columns.first().copied() == Some(provider) && columns.get(1).copied() == Some(model) {
            columns.get(2).and_then(|value| parse_token_count(value))
        } else {
            None
        }
    })
}

fn parse_token_count(value: &str) -> Option<u64> {
    let (number, multiplier) = match value.as_bytes().last().copied()? {
        b'K' | b'k' => (&value[..value.len() - 1], 1_000f64),
        b'M' | b'm' => (&value[..value.len() - 1], 1_000_000f64),
        _ => (value, 1f64),
    };
    let parsed = number.parse::<f64>().ok()?;
    (parsed.is_finite() && parsed > 0.0).then_some((parsed * multiplier).round() as u64)
}

fn read_json(path: &Path) -> Result<Value, ()> {
    fs::read_to_string(path)
        .map_err(|_| ())
        .and_then(|content| serde_json::from_str(&content).map_err(|_| ()))
}

fn json_u64(value: &Value, field: &str) -> Option<u64> {
    value.get(field).and_then(value_u64)
}

fn value_u64(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        let number = value.as_f64()?;
        (number.is_finite() && number >= 0.0 && number.fract() == 0.0).then_some(number as u64)
    })
}

fn json_timestamp(value: &Value, field: &str) -> Option<DateTime<Utc>> {
    let raw = value.get(field)?.as_str()?;
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|timestamp| timestamp.with_timezone(&Utc))
}

fn millis_timestamp(millis: i64) -> Option<DateTime<Utc>> {
    Utc.timestamp_millis_opt(millis).single()
}

fn modified_millis(path: &Path) -> Option<i64> {
    fs::metadata(path)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as i64)
}

fn newest_file_with_extension(root: &Path, extension: &str) -> Option<PathBuf> {
    walk_files_with_extension(root, extension)
        .into_iter()
        .filter_map(|path| modified_millis(&path).map(|modified| (path, modified)))
        .max_by_key(|(_, modified)| *modified)
        .map(|(path, _)| path)
}

fn walk_files_with_extension(root: &Path, extension: &str) -> Vec<PathBuf> {
    if !root.is_dir() {
        return Vec::new();
    }
    let mut files = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some(extension) {
                files.push(path);
            }
        }
    }
    files
}

fn encode_claude_path(path: &Path) -> String {
    path.to_string_lossy()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect()
}

fn escape_json_pointer(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context_tracking::{ContextBand, ContextThresholds, calculate_context_snapshot};
    use crate::token_tracking::{TokenUsageProvider, TokenUsageSource};
    use std::fs;
    use tempfile::TempDir;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 23, 12, 0, 0)
            .single()
            .unwrap()
    }

    fn target<'a>(
        kind: &'a SessionKind,
        workdir: &'a Path,
        conversation_id: Option<&'a str>,
    ) -> ContextCollectionTarget<'a> {
        ContextCollectionTarget {
            session_kind: kind,
            workdir,
            conversation_id,
            excluded_conversation_ids: &[],
            session_created_at: now(),
            provider_id: None,
            model_id: None,
            runtime_payload: None,
            fallback_usage: None,
            fallback_context_limit: None,
            collected_at: now(),
        }
    }

    fn collector(roots: &TempDir) -> SessionContextCollector {
        SessionContextCollector::with_home(roots.path())
    }

    fn write(path: &Path, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    #[test]
    fn claude_status_line_is_direct_and_transcript_fallback_is_estimated() {
        let roots = TempDir::new().unwrap();
        let workdir = roots.path().join("repo");
        let kind = SessionKind::Claude;
        let payload = serde_json::json!({
            "session_id": "claude-1",
            "context_window": {
                "total_input_tokens": 140_000,
                "context_window_size": 200_000,
                "current_usage": {
                    "input_tokens": 10_000,
                    "cache_read_input_tokens": 130_000,
                    "cache_creation_input_tokens": 0
                }
            }
        });
        let mut direct_target = target(&kind, &workdir, Some("claude-1"));
        direct_target.runtime_payload = Some(&payload);
        let ContextCollectionResult::Collected(sample) = collector(&roots).collect(direct_target)
        else {
            panic!("expected direct Claude sample");
        };
        assert_eq!(sample.provenance, ContextProvenance::Direct);
        assert_eq!(sample.used_tokens, 140_000);
        assert_eq!(
            calculate_context_snapshot(sample, ContextThresholds::default())
                .unwrap()
                .band,
            ContextBand::Warning
        );

        let post_reset_payload = serde_json::json!({
            "session_id": "claude-1",
            "context_window": {
                "total_input_tokens": 0,
                "context_window_size": 200_000,
                "current_usage": null
            }
        });
        let mut post_reset_target = target(&kind, &workdir, Some("claude-1"));
        post_reset_target.runtime_payload = Some(&post_reset_payload);
        assert!(matches!(
            collector(&roots).collect(post_reset_target),
            ContextCollectionResult::ResetPending {
                conversation_id: Some(id),
                event: ContextResetEvent {
                    reason: ContextResetReason::Compaction,
                    ..
                },
            } if id == "claude-1"
        ));

        let transcript = roots
            .path()
            .join(".claude/projects")
            .join(encode_claude_path(&workdir))
            .join("claude-1.jsonl");
        write(
            &transcript,
            concat!(
                "not json\n",
                "{\"type\":\"assistant\",\"timestamp\":\"2026-08-23T11:59:00Z\",\"sessionId\":\"claude-1\",\"message\":{\"model\":\"claude-sonnet-5\",\"usage\":{\"input_tokens\":2,\"cache_read_input_tokens\":100000,\"cache_creation_input_tokens\":1000}}}\n"
            ),
        );
        let ContextCollectionResult::Collected(sample) =
            collector(&roots).collect(target(&kind, &workdir, Some("claude-1")))
        else {
            panic!("expected transcript fallback");
        };
        assert_eq!(sample.used_tokens, 101_002);
        assert_eq!(sample.context_limit, Some(900_000));
        assert_eq!(sample.provenance, ContextProvenance::Estimated);
    }

    #[test]
    fn codex_rollout_uses_last_usage_and_direct_window() {
        let roots = TempDir::new().unwrap();
        let workdir = roots.path().join("repo");
        let rollout = roots
            .path()
            .join(".codex/sessions/2026/08/23/rollout.jsonl");
        write(
            &rollout,
            &format!(
                "{{\"timestamp\":\"2026-08-23T11:00:00Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"codex-1\",\"cwd\":{}}}}}\n\
                 {{malformed\n\
                 {{\"timestamp\":\"2026-08-23T11:59:00Z\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"info\":{{\"total_token_usage\":{{\"total_tokens\":900000}},\"last_token_usage\":{{\"input_tokens\":179000,\"cached_input_tokens\":170000,\"output_tokens\":1000,\"total_tokens\":180000}},\"model_context_window\":200000}}}}}}\n",
                serde_json::to_string(workdir.to_string_lossy().as_ref()).unwrap()
            ),
        );
        let kind = SessionKind::Codex;
        let ContextCollectionResult::Collected(sample) =
            collector(&roots).collect(target(&kind, &workdir, Some("codex-1")))
        else {
            panic!("expected Codex sample");
        };
        assert_eq!(sample.used_tokens, 180_000);
        assert_eq!(sample.context_limit, Some(200_000));
        assert_eq!(sample.provenance, ContextProvenance::Direct);
        assert_eq!(
            calculate_context_snapshot(sample, ContextThresholds::default())
                .unwrap()
                .band,
            ContextBand::Critical
        );
    }

    #[test]
    fn pi_session_uses_latest_usage_and_custom_model_limit() {
        let roots = TempDir::new().unwrap();
        let workdir = roots.path().join("repo");
        let session = roots
            .path()
            .join(".pi/agent/sessions/--repo--/session.jsonl");
        write(
            &session,
            &format!(
                "{{\"type\":\"session\",\"version\":3,\"id\":\"pi-1\",\"timestamp\":\"2026-08-23T11:00:00Z\",\"cwd\":{}}}\n\
                 {{\"type\":\"message\",\"id\":\"a1\",\"parentId\":null,\"timestamp\":\"2026-08-23T11:59:00Z\",\"message\":{{\"role\":\"assistant\",\"provider\":\"anthropic\",\"model\":\"claude-test\",\"timestamp\":1787486340000,\"stopReason\":\"stop\",\"usage\":{{\"input\":70000,\"output\":1000,\"cacheRead\":0,\"cacheWrite\":0,\"totalTokens\":71000}}}}}}\n",
                serde_json::to_string(workdir.to_string_lossy().as_ref()).unwrap()
            ),
        );
        write(
            &roots.path().join(".pi/agent/models.json"),
            &serde_json::json!({
                "providers": {"anthropic": {"models": [{"id": "claude-test", "contextWindow": 100_000}]}}
            })
            .to_string(),
        );
        let kind = SessionKind::Pi;
        let ContextCollectionResult::Collected(sample) =
            collector(&roots).collect(target(&kind, &workdir, Some("pi-1")))
        else {
            panic!("expected Pi sample");
        };
        assert_eq!(sample.used_tokens, 71_000);
        assert_eq!(sample.context_limit, Some(100_000));
        assert_eq!(sample.provenance, ContextProvenance::Estimated);
    }

    #[test]
    fn malformed_or_unsupported_telemetry_is_recoverable() {
        let roots = TempDir::new().unwrap();
        let workdir = roots.path().join("repo");
        let kind = SessionKind::Terminal;
        assert_eq!(
            collector(&roots).collect(target(&kind, &workdir, None)),
            ContextCollectionResult::Unavailable(ContextUnavailableReason::UnsupportedSessionKind)
        );

        let kind = SessionKind::Claude;
        let transcript = roots
            .path()
            .join(".claude/projects")
            .join(encode_claude_path(&workdir))
            .join("broken.jsonl");
        write(&transcript, "{not-json\n");
        assert_eq!(
            collector(&roots).collect(target(&kind, &workdir, Some("broken"))),
            ContextCollectionResult::Unavailable(ContextUnavailableReason::MalformedTelemetry)
        );
    }

    #[test]
    fn cumulative_fallback_is_explicitly_estimated() {
        let roots = TempDir::new().unwrap();
        let workdir = roots.path().join("repo");
        let kind = SessionKind::Claude;
        let usage = SessionTokenUsage {
            source: TokenUsageSource {
                provider: TokenUsageProvider::Claude,
                id: "claude-1".to_string(),
            },
            input_tokens: 50_000,
            output_tokens: 1_000,
            cache_read_tokens: 20_000,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            total_tokens: 71_000,
        };
        let mut request = target(&kind, &workdir, Some("claude-1"));
        request.fallback_usage = Some(&usage);
        let ContextCollectionResult::Collected(sample) = collector(&roots).collect(request) else {
            panic!("expected fallback sample");
        };
        assert_eq!(sample.used_tokens, 70_000);
        assert_eq!(sample.provenance, ContextProvenance::Estimated);
    }

    #[test]
    fn explicit_pi_compaction_waits_for_a_post_reset_sample() {
        let roots = TempDir::new().unwrap();
        let workdir = roots.path().join("repo");
        let session = roots
            .path()
            .join(".pi/agent/sessions/--repo--/session.jsonl");
        write(
            &session,
            &format!(
                "{{\"type\":\"session\",\"version\":3,\"id\":\"pi-1\",\"timestamp\":\"2026-08-23T11:00:00Z\",\"cwd\":{}}}\n\
                 {{\"type\":\"message\",\"id\":\"a1\",\"parentId\":null,\"message\":{{\"role\":\"assistant\",\"provider\":\"x\",\"model\":\"y\",\"timestamp\":1,\"stopReason\":\"stop\",\"usage\":{{\"input\":90000,\"output\":0,\"cacheRead\":0,\"cacheWrite\":0,\"totalTokens\":90000}}}}}}\n\
                 {{\"type\":\"compaction\",\"id\":\"c1\",\"parentId\":\"a1\",\"timestamp\":\"2026-08-23T11:59:00Z\",\"summary\":\"summary\",\"firstKeptEntryId\":\"a1\",\"tokensBefore\":90000}}\n",
                serde_json::to_string(workdir.to_string_lossy().as_ref()).unwrap()
            ),
        );
        let kind = SessionKind::Pi;
        assert!(matches!(
            collector(&roots).collect(target(&kind, &workdir, Some("pi-1"))),
            ContextCollectionResult::ResetPending {
                conversation_id: Some(id),
                event: ContextResetEvent {
                    reason: ContextResetReason::Compaction,
                    ..
                },
            } if id == "pi-1"
        ));
    }

    #[test]
    fn trailing_claude_and_codex_compactions_hide_pre_reset_usage() {
        let roots = TempDir::new().unwrap();
        let workdir = roots.path().join("repo");
        let claude_path = roots
            .path()
            .join(".claude/projects")
            .join(encode_claude_path(&workdir))
            .join("claude-reset.jsonl");
        write(
            &claude_path,
            concat!(
                "{\"type\":\"assistant\",\"timestamp\":\"2026-08-23T11:00:00Z\",\"sessionId\":\"claude-reset\",\"message\":{\"usage\":{\"input_tokens\":180000}}}\n",
                "{\"type\":\"system\",\"subtype\":\"compact_boundary\",\"timestamp\":\"2026-08-23T11:01:00Z\"}\n"
            ),
        );
        assert!(matches!(
            collector(&roots).collect(target(&SessionKind::Claude, &workdir, Some("claude-reset"))),
            ContextCollectionResult::ResetPending { .. }
        ));

        let codex_path = roots.path().join(".codex/sessions/reset.jsonl");
        write(
            &codex_path,
            &format!(
                "{{\"timestamp\":\"2026-08-23T11:00:00Z\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"codex-reset\",\"cwd\":{}}}}}\n\
                 {{\"timestamp\":\"2026-08-23T11:00:30Z\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"info\":{{\"last_token_usage\":{{\"total_tokens\":180000}},\"model_context_window\":200000}}}}}}\n\
                 {{\"timestamp\":\"2026-08-23T11:01:00Z\",\"type\":\"compacted\",\"payload\":{{}}}}\n",
                serde_json::to_string(workdir.to_string_lossy().as_ref()).unwrap()
            ),
        );
        assert!(matches!(
            collector(&roots).collect(target(&SessionKind::Codex, &workdir, Some("codex-reset"))),
            ContextCollectionResult::ResetPending { .. }
        ));
    }
}

// FORK: the incremental transcript reads must give what a whole-file read gives.
#[cfg(test)]
mod fork_incremental_tests {
    use super::*;
    use std::io::Write;

    fn target(collected_at: DateTime<Utc>) -> ContextCollectionTarget<'static> {
        ContextCollectionTarget {
            session_kind: &SessionKind::Claude,
            workdir: Path::new("/tmp/aiw-incremental"),
            conversation_id: Some("target-conversation"),
            excluded_conversation_ids: &[],
            session_created_at: collected_at,
            provider_id: None,
            model_id: None,
            runtime_payload: None,
            fallback_usage: None,
            fallback_context_limit: None,
            collected_at,
        }
    }

    fn append_and_compare<S: Clone + Default>(
        lines: &[&str],
        fold: fn(&mut S, &str),
        result: fn(S, &ContextCollectionTarget<'_>) -> Option<ContextCollectionResult>,
        whole: fn(&Path, &ContextCollectionTarget<'_>) -> Option<ContextCollectionResult>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        let mut file = std::fs::File::create(&path).unwrap();
        let mut cache = crate::fork::line_fold::LineFoldCache::<S>::default();
        let mut sweep = Utc.with_ymd_and_hms(2026, 10, 10, 12, 0, 0).unwrap();
        for line in lines {
            // Each line lands in two writes, so some sweeps see half a line.
            let (head, tail) = line.split_at(line.len() / 2);
            for part in [head.to_string(), format!("{tail}\n")] {
                file.write_all(part.as_bytes()).unwrap();
                file.flush().unwrap();
                sweep += chrono::Duration::seconds(5);
                let target = target(sweep);
                let incremental = cache.fold(&path, fold).and_then(|s| result(s, &target));
                assert_eq!(incremental, whole(&path, &target), "after {part:?}");
            }
        }
    }

    /// `cargo +1.99.0 test --release --locked sweep_cost_bench -- --ignored --nocapture`
    #[test]
    #[ignore = "timing benchmark"]
    fn sweep_cost_bench() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        let line = r#"{"type":"assistant","sessionId":"s1","timestamp":"2026-10-10T11:00:01Z","message":{"model":"claude-x","content":[{"type":"text","text":"lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua"}],"usage":{"input_tokens":10,"cache_read_input_tokens":5}}}"#;
        let mut content = String::new();
        while content.len() < 10 * 1024 * 1024 {
            content.push_str(line);
            content.push('\n');
        }
        std::fs::write(&path, &content).unwrap();
        let target = target(Utc::now());
        let started = std::time::Instant::now();
        let whole = parse_claude_transcript(&path, &target);
        let whole_time = started.elapsed();
        let mut cache = crate::fork::line_fold::LineFoldCache::<ClaudeScan>::default();
        let _ = cache.fold(&path, claude_scan_line);
        let started = std::time::Instant::now();
        let again = cache
            .fold(&path, claude_scan_line)
            .and_then(|scan| claude_scan_result(scan, &target));
        let again_time = started.elapsed();
        assert_eq!(whole, again);
        println!(
            "10 MB Claude transcript, one sweep: whole-file parse {:.1} ms, incremental (unchanged) {:.3} ms",
            whole_time.as_secs_f64() * 1e3,
            again_time.as_secs_f64() * 1e3
        );
    }

    #[test]
    fn claude_transcript_incremental_matches_whole_file() {
        append_and_compare(
            &[
                r#"{"type":"user","timestamp":"2026-10-10T11:00:00Z"}"#,
                r#"{"type":"assistant","sessionId":"s1","timestamp":"2026-10-10T11:00:01Z","message":{"usage":{"input_tokens":10,"cache_read_input_tokens":5}}}"#,
                r#"{"type":"assistant","message":{"usage":{"input_tokens":20,"cache_creation_input_tokens":7}}}"#,
                r#"{"type":"system","subtype":"compact_boundary","timestamp":"2026-10-10T11:05:00Z"}"#,
                r#"not json"#,
                r#"{"type":"assistant","session_id":"s2","timestamp":"2026-10-10T11:06:00Z","message":{"usage":{"input_tokens":3}}}"#,
                r#"{"type":"system","subtype":"microcompact_boundary"}"#,
            ],
            claude_scan_line,
            claude_scan_result,
            parse_claude_transcript,
        );
    }

    #[test]
    fn codex_rollout_incremental_matches_whole_file() {
        append_and_compare(
            &[
                r#"{"type":"session_meta","payload":{"id":"codex-1"}}"#,
                r#"{"type":"event_msg","timestamp":"2026-10-10T11:00:00Z","payload":{"type":"token_count","info":{"last_token_usage":{"total_tokens":100},"model_context_window":1000}}}"#,
                r#"{"type":"compacted","timestamp":"2026-10-10T11:01:00Z"}"#,
                r#"{"type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":40,"output_tokens":2},"model_context_window":1000}}}"#,
                r#"{"type":"session_meta","payload":{"session_id":"codex-2"}}"#,
                r#"{"type":"event_msg","payload":{"type":"context_compacted"}}"#,
            ],
            codex_scan_line,
            codex_scan_result,
            parse_codex_rollout,
        );
    }
}
