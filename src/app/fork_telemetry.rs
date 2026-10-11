//! FORK: per-session context and cost on the dashboard (P4, DESIGN.md §16).
//!
//! `App::telemetry` is keyed by feature session id. Claude's numbers arrive
//! over IPC from `aiw status-line`; Codex's and Gemini's are read on the
//! 5-second sweep by a background thread ([`App::telemetry_tick`] /
//! [`App::poll_telemetry_bg`]) that folds each rollout / chat file
//! incrementally, so the main loop and the frame never read a file for them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use serde_json::Value;

use super::App;
use crate::context_tracking::ContextBand;
use crate::fork::line_fold::LineFoldCache;
use crate::fork::telemetry::{
    CodexRolloutScan, GeminiChatScan, STATUS_LINE_TYPE, SessionTelemetry, codex_rollout_line,
    gemini_chat_line, gemini_chats_dir, gemini_home, gemini_newest_session, telemetry_from_message,
};
use crate::project::SessionKind;
use crate::token_tracking::TokenUsageProvider;

/// The incremental readers, handed to the sweep thread and back.
#[derive(Default)]
pub struct TelemetryScans {
    codex: LineFoldCache<CodexRolloutScan>,
    gemini: LineFoldCache<GeminiChatScan>,
}

/// One session the sweep should read: where its file is.
struct TelemetryJob {
    session_id: String,
    kind: SessionKind,
    workdir: PathBuf,
    codex_thread_id: Option<String>,
}

/// The sweep's result: the sessions that reported something.
pub struct TelemetrySweep {
    scans: TelemetryScans,
    updates: Vec<(String, SessionTelemetry)>,
}

/// Runtime state of the sweep, on `App`.
#[derive(Default)]
pub struct TelemetryRuntime {
    scans: Option<TelemetryScans>,
    bg: Option<Receiver<TelemetrySweep>>,
}

fn run_jobs(mut scans: TelemetryScans, jobs: Vec<TelemetryJob>, home: &Path) -> TelemetrySweep {
    let gemini = gemini_home(home);
    let mut updates = Vec::new();
    for job in jobs {
        let telemetry = match job.kind {
            SessionKind::Codex => job
                .codex_thread_id
                .as_deref()
                .and_then(|id| crate::codex::indexed_session_in(home, &job.workdir, id))
                .map(|session| session.rollout_path)
                .and_then(|path| scans.codex.fold(&path, codex_rollout_line))
                .and_then(|scan| scan.telemetry()),
            SessionKind::Gemini => gemini_chats_dir(&gemini, &job.workdir)
                .and_then(|chats| gemini_newest_session(&chats))
                .and_then(|path| scans.gemini.fold(&path, gemini_chat_line))
                .and_then(|scan| scan.telemetry()),
            _ => None,
        };
        if let Some(telemetry) = telemetry {
            updates.push((job.session_id, telemetry));
        }
    }
    TelemetrySweep { scans, updates }
}

impl App {
    /// A `status_line` message from `aiw status-line`. Returns `false` when
    /// `raw` is not one.
    pub(crate) fn handle_status_line_message(&mut self, raw: &Value) -> bool {
        if raw.get("type").and_then(Value::as_str) != Some(STATUS_LINE_TYPE) {
            return false;
        }
        let text = |key: &str| {
            raw.get(key)
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|v| !v.is_empty())
        };
        let session_id = text("aiw_feature_session_id").or_else(|| {
            // An older launch without the id: the feature's Claude window.
            let aiw_session = text("aiw_tmux_session").or_else(|| text("aiw_session"));
            let cwd = text("cwd").unwrap_or_default();
            let (pi, fi) = self.fork_feature_for(aiw_session.as_deref(), &cwd)?;
            self.store.projects[pi].features[fi]
                .sessions
                .iter()
                .find(|s| s.kind == SessionKind::Claude)
                .map(|s| s.id.clone())
        });
        let Some(session_id) = session_id else {
            self.log_debug("telemetry", "status_line matched no session".into());
            return true;
        };
        let telemetry = telemetry_from_message(raw);
        if let Some(limits) = telemetry.rate_limits.as_ref() {
            self.usage.set_claude_rate_limits(limits);
        }
        self.telemetry.insert(session_id, telemetry);
        true
    }

    /// Main-loop step on the 5-second sweep: read Codex rollouts and Gemini
    /// chats in the background, unless the previous sweep is still running.
    pub fn telemetry_tick(&mut self) {
        if self.fork_telemetry.bg.is_some() {
            return;
        }
        let jobs: Vec<TelemetryJob> = self
            .store
            .projects
            .iter()
            .flat_map(|p| p.features.iter())
            .filter(|f| f.status != crate::project::ProjectStatus::Stopped)
            .flat_map(|f| {
                f.sessions
                    .iter()
                    .filter(|s| matches!(s.kind, SessionKind::Codex | SessionKind::Gemini))
                    .filter(|s| !s.stopped)
                    .map(move |s| TelemetryJob {
                        session_id: s.id.clone(),
                        kind: s.kind.clone(),
                        workdir: f.workdir.clone(),
                        codex_thread_id: s
                            .token_usage_source
                            .as_ref()
                            .filter(|src| src.provider == TokenUsageProvider::Codex)
                            .map(|src| src.id.clone()),
                    })
            })
            .collect();
        if jobs.is_empty() {
            return;
        }
        let Some(home) = dirs::home_dir() else {
            return;
        };
        let scans = self.fork_telemetry.scans.take().unwrap_or_default();
        let (tx, rx) = std::sync::mpsc::channel();
        self.fork_telemetry.bg = Some(rx);
        std::thread::spawn(move || {
            let _ = tx.send(run_jobs(scans, jobs, &home));
        });
    }

    /// Apply a finished sweep. Returns whether anything changed.
    pub fn poll_telemetry_bg(&mut self) -> bool {
        let Some(rx) = self.fork_telemetry.bg.as_ref() else {
            return false;
        };
        let sweep = match rx.try_recv() {
            Ok(sweep) => sweep,
            Err(std::sync::mpsc::TryRecvError::Empty) => return false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.fork_telemetry.bg = None;
                return false;
            }
        };
        self.fork_telemetry.bg = None;
        self.fork_telemetry.scans = Some(sweep.scans);
        let mut changed = false;
        for (session_id, telemetry) in sweep.updates {
            let same = self.telemetry.get(&session_id).is_some_and(|known| {
                known.model == telemetry.model
                    && known.context_pct == telemetry.context_pct
                    && known.context_used_tokens == telemetry.context_used_tokens
                    && known.total_tokens == telemetry.total_tokens
            });
            if !same {
                self.telemetry.insert(session_id, telemetry);
                changed = true;
            }
        }
        changed
    }

    /// The band for a context percentage, from the configured thresholds.
    pub fn context_band_for(&self, pct: u8) -> ContextBand {
        if pct >= self.config.context_critical_percent {
            ContextBand::Critical
        } else if pct >= self.config.context_warning_percent {
            ContextBand::Warning
        } else {
            ContextBand::Normal
        }
    }

    /// The session's context use: what the harness reported, else the
    /// collector's snapshot.
    pub fn session_context_percent(&self, session_id: &str) -> Option<(u8, ContextBand)> {
        if let Some(pct) = self.telemetry.get(session_id).and_then(|t| t.context_pct) {
            return Some((pct, self.context_band_for(pct)));
        }
        self.context_states
            .get(session_id)
            .and_then(|state| state.snapshot.as_ref())
            .map(|snapshot| (snapshot.percentage.get(), snapshot.band))
    }

    /// The row's context label: `42% ctx`, or the token count when no
    /// window is known (Gemini).
    pub fn session_context_label(&self, session_id: &str) -> Option<String> {
        if let Some((pct, _)) = self.session_context_percent(session_id) {
            return Some(format!("{pct}% ctx"));
        }
        self.telemetry
            .get(session_id)
            .and_then(|t| t.context_label())
    }

    /// Part of `App::redraw_signature`: what the hook-fed state (DESIGN.md
    /// §16) draws on the rows — context use, subagent counts, queued
    /// follow-ups, pending permissions — so an IPC update is a frame.
    pub(crate) fn fork_hook_state_signature(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        let mut sessions: Vec<&String> = self.telemetry.keys().collect();
        sessions.sort();
        for id in sessions {
            let t = &self.telemetry[id];
            id.hash(&mut hasher);
            t.context_pct.hash(&mut hasher);
            t.context_used_tokens.hash(&mut hasher);
            t.model.hash(&mut hasher);
        }
        let mut counts: Vec<(&String, &usize)> = self.subagent_counts.iter().collect();
        counts.sort();
        counts.hash(&mut hasher);
        // FORK: §19.1 row events.
        let mut hooks: Vec<_> = self.hook_events.iter().collect();
        hooks.sort_by(|a, b| a.0.cmp(b.0));
        hooks.hash(&mut hasher);
        let mut queues: Vec<(&String, usize)> =
            self.followups.iter().map(|(id, q)| (id, q.len())).collect();
        queues.sort();
        queues.hash(&mut hasher);
        for pending in &self.pending_permissions {
            pending.request_id.hash(&mut hasher);
        }
        hasher.finish()
    }

    /// Claude's status-line payloads for the context collector, keyed by
    /// session id.
    pub(crate) fn claude_runtime_payloads(&self) -> HashMap<String, Value> {
        self.telemetry
            .iter()
            .filter_map(|(id, t)| Some((id.clone(), t.claude_runtime_payload(None)?)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fork::badges::BadgeKind;
    use serde_json::json;

    fn message(session_id: &str, pct: u64) -> Value {
        json!({
            "type": "status_line", "session_id": "c-1", "aiw_session": "aiw-hx-12",
            "aiw_feature_session_id": session_id, "model": "Opus 4", "context_used_pct": pct,
            "context_window": 200000, "cost_usd": 0.84, "lines_added": 1, "lines_removed": 0,
            "rate_limits": {"five_hour": {"used_percentage": 12.5, "resets_at": 1760000000},
                            "seven_day": {"used_percentage": 30}}
        })
    }

    #[test]
    fn a_status_line_sets_the_rows_context_and_the_badge_label() {
        let mut app = crate::app::fork_workspace::workspace_app(&[SessionKind::Claude]);
        let id = app.store.projects[0].features[0].sessions[0].id.clone();
        assert!(app.handle_status_line_message(&message(&id, 42)));
        assert_eq!(
            app.session_context_percent(&id),
            Some((42, ContextBand::Normal))
        );
        assert_eq!(app.session_context_label(&id).as_deref(), Some("42% ctx"));
        app.thinking_features.insert("aiw-hx-12".into());
        let badge = app.session_badge_at(0, 0, 0).unwrap();
        assert_eq!(badge.kind, BadgeKind::Working);
        assert_eq!(badge.label(chrono::Utc::now()), "working 0:00 · 42% ctx");
        // The thresholds colour it.
        assert!(app.handle_status_line_message(&message(&id, 75)));
        assert_eq!(
            app.session_context_percent(&id).unwrap().1,
            ContextBand::Warning
        );
        assert!(app.handle_status_line_message(&message(&id, 90)));
        assert_eq!(
            app.session_context_percent(&id).unwrap().1,
            ContextBand::Critical
        );
        // Claude's limits reach the header.
        let usage = app.usage.get_data();
        assert_eq!(usage.claude.five_hour_pct, Some(12.5));
        assert_eq!(usage.claude.seven_day_pct, Some(30.0));
        assert!(usage.claude.five_hour_resets.is_some());
        assert!(!app.handle_status_line_message(&json!({"type": "stop"})));
    }

    #[test]
    fn a_status_line_without_the_session_id_lands_on_the_features_claude_window() {
        let mut app =
            crate::app::fork_workspace::workspace_app(&[SessionKind::Codex, SessionKind::Claude]);
        let claude = app.store.projects[0].features[0].sessions[1].id.clone();
        let mut raw = message("x", 10);
        raw.as_object_mut()
            .unwrap()
            .remove("aiw_feature_session_id");
        raw["cwd"] = json!("/tmp");
        assert!(app.handle_status_line_message(&raw));
        assert!(app.telemetry.contains_key(&claude));
    }

    #[test]
    fn the_sweep_reads_codex_rollouts_and_gemini_chats_from_a_home() {
        let home = tempfile::tempdir().unwrap();
        let workdir = home.path().join("repo");
        std::fs::create_dir_all(&workdir).unwrap();
        // Gemini: the chat file under the project hash.
        let chats = home
            .path()
            .join(".gemini/tmp")
            .join(crate::fork::telemetry::gemini_project_hash(&workdir))
            .join("chats");
        std::fs::create_dir_all(&chats).unwrap();
        std::fs::write(
            chats.join("session-2026-10-10T10-00-abcdefgh.jsonl"),
            "{\"sessionId\":\"s\"}\n{\"type\":\"gemini\",\"model\":\"gemini-2.5-pro\",\"tokens\":{\"total\":5000}}\n",
        )
        .unwrap();
        let jobs = vec![
            TelemetryJob {
                session_id: "g1".into(),
                kind: SessionKind::Gemini,
                workdir: workdir.clone(),
                codex_thread_id: None,
            },
            TelemetryJob {
                session_id: "c1".into(),
                kind: SessionKind::Codex,
                workdir: workdir.clone(),
                codex_thread_id: Some("missing".into()),
            },
        ];
        let sweep = run_jobs(TelemetryScans::default(), jobs, home.path());
        assert_eq!(sweep.updates.len(), 1);
        assert_eq!(sweep.updates[0].0, "g1");
        assert_eq!(
            sweep.updates[0].1.context_label().as_deref(),
            Some("5.0k tokens")
        );
    }
}
