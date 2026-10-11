//! FORK: the session info block's state (DESIGN.md §18).
//!
//! The block under the cockpit sidebar describes the selected session: its
//! title (the harness's own summary, else its first prompt), harness and
//! model, context, cost, age, branch, worktree, queued follow-ups and the
//! last permission decision. Everything it shows is already in the app or
//! comes from the harness's own files, read on the 5-second sweep by a
//! background thread ([`App::session_info_tick`] /
//! [`App::poll_session_info_bg`]) that folds each transcript incrementally
//! and caches the result per feature session id. Drawing reads the cache
//! and the store only.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;

use chrono::{DateTime, Utc};

use super::{App, Selection};
use crate::context_tracking::ContextBand;
use crate::fork::line_fold::LineFoldCache;
use crate::fork::permissions::Behavior;
use crate::fork::session_info::{
    TranscriptScan, age_label, claude_transcript_line, claude_transcript_path, codex_rollout_line,
    cost_label, gemini_chat_line, one_line, shorten_worktree,
};
use crate::fork::telemetry::{gemini_chats_dir, gemini_home, gemini_newest_session};
use crate::project::SessionKind;
use crate::token_tracking::TokenUsageProvider;

/// What the sweep learned about one session from its harness's files.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TranscriptInfo {
    /// Summary or first prompt, on one line.
    pub title: Option<String>,
    pub model: Option<String>,
    /// The AIW task prompt (`latest-prompt.txt`), the fallback title.
    pub task_prompt: Option<String>,
}

/// The incremental readers, handed to the sweep thread and back.
#[derive(Default)]
pub struct InfoScans {
    claude: LineFoldCache<TranscriptScan>,
    codex: LineFoldCache<TranscriptScan>,
    gemini: LineFoldCache<TranscriptScan>,
}

/// One session the sweep should read.
struct InfoJob {
    session_id: String,
    kind: SessionKind,
    workdir: PathBuf,
    /// Claude's session id (also where Gemini's resume id is kept).
    harness_session_id: Option<String>,
    codex_thread_id: Option<String>,
}

pub struct InfoSweep {
    scans: InfoScans,
    updates: Vec<(String, TranscriptInfo)>,
}

/// The last answer given to a session's permission request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionDecision {
    pub tool: String,
    pub behavior: Behavior,
    pub at: DateTime<Utc>,
}

impl PermissionDecision {
    /// `✓ allowed Bash`, `✗ denied Edit`, `↩ passed Bash`.
    pub fn label(&self) -> String {
        let (glyph, word) = match self.behavior {
            Behavior::Allow => ("✓", "allowed"),
            Behavior::Deny => ("✗", "denied"),
            Behavior::Pass => ("↩", "passed"),
        };
        format!("{glyph} {word} {}", self.tool)
    }
}

/// Runtime state, on `App`.
#[derive(Default)]
pub struct SessionInfoRuntime {
    scans: Option<InfoScans>,
    bg: Option<Receiver<InfoSweep>>,
    /// Per feature session id, from the last sweep.
    pub cache: HashMap<String, TranscriptInfo>,
    /// Per feature session id, the last permission decision.
    pub decisions: HashMap<String, PermissionDecision>,
}

/// Everything the block draws, assembled from the app's state. Pure data:
/// the renderer only lays it out.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionInfo {
    pub label: String,
    /// The session title, or the fallback (task prompt / feature name).
    pub title: String,
    pub harness: String,
    pub model: Option<String>,
    pub context: Option<(u8, ContextBand)>,
    pub cost: Option<String>,
    pub age: String,
    pub branch: String,
    pub worktree: Option<String>,
    pub queued: usize,
    pub decision: Option<String>,
}

/// The harness's name as the block says it.
fn harness_name(kind: &SessionKind) -> &'static str {
    match kind {
        SessionKind::Claude => "Claude Code",
        SessionKind::Codex => "Codex",
        SessionKind::Pi => "Pi",
        SessionKind::Gemini => "Gemini CLI",
        SessionKind::Cursor => "Cursor agent",
        SessionKind::Terminal => "terminal",
        SessionKind::Nvim => "Neovim",
        SessionKind::Vscode => "VS Code",
        SessionKind::Custom => "custom",
        SessionKind::Todos => "TODOs",
    }
}

fn read_task_prompt(workdir: &Path) -> Option<String> {
    ["claude", "codex"].iter().find_map(|dir| {
        std::fs::read_to_string(workdir.join(format!(".{dir}")).join("latest-prompt.txt"))
            .ok()
            .map(|text| one_line(&text))
            .filter(|text| !text.is_empty())
    })
}

fn run_jobs(mut scans: InfoScans, jobs: Vec<InfoJob>, home: &Path) -> InfoSweep {
    let gemini = gemini_home(home);
    let mut updates = Vec::new();
    for job in jobs {
        let scan = match job.kind {
            SessionKind::Claude => job
                .harness_session_id
                .as_deref()
                .map(|id| claude_transcript_path(home, &job.workdir, id))
                .filter(|path| path.is_file())
                .and_then(|path| scans.claude.fold(&path, claude_transcript_line)),
            SessionKind::Codex => job
                .codex_thread_id
                .as_deref()
                .and_then(|id| crate::codex::indexed_session_in(home, &job.workdir, id))
                .map(|session| session.rollout_path)
                .and_then(|path| scans.codex.fold(&path, codex_rollout_line)),
            SessionKind::Gemini => gemini_chats_dir(&gemini, &job.workdir)
                .and_then(|chats| gemini_newest_session(&chats))
                .and_then(|path| scans.gemini.fold(&path, gemini_chat_line)),
            _ => None,
        };
        let info = TranscriptInfo {
            title: scan.as_ref().and_then(TranscriptScan::title),
            model: scan.and_then(|s| s.model),
            task_prompt: read_task_prompt(&job.workdir),
        };
        updates.push((job.session_id, info));
    }
    InfoSweep { scans, updates }
}

impl App {
    /// Main-loop step on the 5-second sweep: read the running agent
    /// sessions' transcripts in the background, unless the previous sweep
    /// is still running or the block is never drawn (no cockpit).
    pub fn session_info_tick(&mut self) {
        if !self.cockpit_active() || self.fork_session_info.bg.is_some() {
            return;
        }
        let jobs: Vec<InfoJob> = self
            .store
            .projects
            .iter()
            .flat_map(|p| p.features.iter())
            .filter(|f| f.status != crate::project::ProjectStatus::Stopped)
            .flat_map(|f| {
                f.sessions
                    .iter()
                    .filter(|s| s.kind.is_agent_harness() && !s.stopped)
                    .map(move |s| InfoJob {
                        session_id: s.id.clone(),
                        kind: s.kind.clone(),
                        workdir: f.workdir.clone(),
                        harness_session_id: s.claude_session_id.clone(),
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
        let scans = self.fork_session_info.scans.take().unwrap_or_default();
        let (tx, rx) = std::sync::mpsc::channel();
        self.fork_session_info.bg = Some(rx);
        std::thread::Builder::new()
            .name("aiw-session-info".into())
            .spawn(move || {
                let _ = tx.send(run_jobs(scans, jobs, &home));
            })
            .ok();
    }

    /// Apply a finished sweep. Returns whether anything changed.
    pub fn poll_session_info_bg(&mut self) -> bool {
        let Some(rx) = self.fork_session_info.bg.as_ref() else {
            return false;
        };
        let sweep = match rx.try_recv() {
            Ok(sweep) => sweep,
            Err(std::sync::mpsc::TryRecvError::Empty) => return false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.fork_session_info.bg = None;
                return false;
            }
        };
        self.fork_session_info.bg = None;
        self.fork_session_info.scans = Some(sweep.scans);
        let mut changed = false;
        for (session_id, info) in sweep.updates {
            if self.fork_session_info.cache.get(&session_id) != Some(&info) {
                self.fork_session_info.cache.insert(session_id, info);
                changed = true;
            }
        }
        changed
    }

    /// Remember a permission answer for the block (`answer_permission`).
    pub(crate) fn record_permission_decision(
        &mut self,
        feature_session_id: Option<&str>,
        tool: &str,
        behavior: Behavior,
    ) {
        if let Some(id) = feature_session_id {
            self.fork_session_info.decisions.insert(
                id.to_string(),
                PermissionDecision {
                    tool: tool.to_string(),
                    behavior,
                    at: Utc::now(),
                },
            );
        }
    }

    /// The session the block describes: the selected session row, else the
    /// agent the pane shows for a feature or project row.
    pub fn session_info_target(&self) -> Option<(usize, usize, usize)> {
        match self.selection {
            Selection::Session(pi, fi, si) => {
                let session = self
                    .store
                    .projects
                    .get(pi)?
                    .features
                    .get(fi)?
                    .sessions
                    .get(si)?;
                session.kind.is_agent_harness().then_some((pi, fi, si))
            }
            Selection::Feature(..) | Selection::Project(_) => {
                let live = self.fork_workspace.live.as_ref()?;
                self.store
                    .projects
                    .iter()
                    .enumerate()
                    .find_map(|(pi, project)| {
                        project
                            .features
                            .iter()
                            .enumerate()
                            .find_map(|(fi, feature)| {
                                feature
                                    .sessions
                                    .iter()
                                    .position(|s| s.id == live.session_id)
                                    .map(|si| (pi, fi, si))
                            })
                    })
            }
        }
    }

    /// The block's content for the session at `(pi, fi, si)`: store and
    /// cache lookups only.
    pub fn session_info_for(
        &self,
        pi: usize,
        fi: usize,
        si: usize,
        now: DateTime<Utc>,
    ) -> Option<SessionInfo> {
        let project = self.store.projects.get(pi)?;
        let feature = project.features.get(fi)?;
        let session = feature.sessions.get(si)?;
        let cached = self.fork_session_info.cache.get(&session.id);
        let telemetry = self.telemetry.get(&session.id);
        let title = cached
            .and_then(|c| c.title.clone())
            .or_else(|| cached.and_then(|c| c.task_prompt.clone()))
            .unwrap_or_else(|| {
                feature
                    .nickname
                    .clone()
                    .unwrap_or_else(|| feature.name.clone())
            });
        let model = telemetry
            .and_then(|t| t.model.clone())
            .or_else(|| cached.and_then(|c| c.model.clone()));
        let cost = telemetry.and_then(|t| t.cost_usd).map(cost_label);
        let worktree = feature.is_worktree.then(|| {
            shorten_worktree(
                &feature.workdir,
                dirs::home_dir().as_deref(),
                usize::from(self.viewport_cols.saturating_sub(6)),
            )
        });
        Some(SessionInfo {
            label: session.label.clone(),
            title,
            harness: harness_name(&session.kind).to_string(),
            model,
            context: self.session_context_percent(&session.id),
            cost,
            age: age_label(now.signed_duration_since(session.created_at).num_seconds()),
            branch: feature.branch.clone(),
            worktree,
            queued: self.queued_followup_count(&session.id),
            decision: self
                .fork_session_info
                .decisions
                .get(&session.id)
                .map(PermissionDecision::label),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::fork_workspace::workspace_app;
    use crate::fork::cockpit::Link;
    use crate::fork::telemetry::{SessionTelemetry, TelemetrySource, parse_claude_status};

    fn app() -> App {
        let mut app = workspace_app(&[SessionKind::Claude, SessionKind::Codex]);
        app.cockpit_attach(
            Link::recording(),
            "/bin/aiw".into(),
            "/s/cockpit.sock".into(),
        );
        app.viewport_cols = 36;
        app
    }

    fn telemetry() -> SessionTelemetry {
        parse_claude_status(&serde_json::json!({
            "model": {"display_name": "Opus 4"},
            "context_window": {"used_percentage": 42},
            "cost": {"total_cost_usd": 0.8412}
        }))
    }

    #[test]
    fn the_block_falls_back_to_the_task_prompt_then_the_feature_name() {
        let mut app = app();
        let id = app.store.projects[0].features[0].sessions[0].id.clone();
        let now = Utc::now();
        let info = app.session_info_for(0, 0, 0, now).unwrap();
        assert_eq!(info.title, "hx-12");
        assert_eq!(info.harness, "Claude Code");
        assert_eq!(info.label, "Claude 1");
        assert_eq!(info.branch, "hx-12");
        assert_eq!(info.worktree, None, "not a worktree feature");
        assert_eq!(info.queued, 0);
        assert_eq!(info.decision, None);
        app.fork_session_info.cache.insert(
            id.clone(),
            TranscriptInfo {
                title: None,
                model: None,
                task_prompt: Some("Fix the uploader".into()),
            },
        );
        assert_eq!(
            app.session_info_for(0, 0, 0, now).unwrap().title,
            "Fix the uploader"
        );
        app.fork_session_info.cache.insert(
            id.clone(),
            TranscriptInfo {
                title: Some("SSO redirect loop fixed".into()),
                model: Some("claude-opus-4".into()),
                task_prompt: Some("Fix the uploader".into()),
            },
        );
        let info = app.session_info_for(0, 0, 0, now).unwrap();
        assert_eq!(info.title, "SSO redirect loop fixed");
        assert_eq!(info.model.as_deref(), Some("claude-opus-4"));
        // The status line wins for the model, and brings cost and context.
        app.telemetry.insert(id.clone(), telemetry());
        let info = app.session_info_for(0, 0, 0, now).unwrap();
        assert_eq!(info.model.as_deref(), Some("Opus 4"));
        assert_eq!(info.cost.as_deref(), Some("$0.84"));
        assert_eq!(info.context.map(|(pct, _)| pct), Some(42));
        assert_eq!(app.telemetry[&id].source, TelemetrySource::StatusLine);
        app.record_permission_decision(Some(&id), "Bash", Behavior::Allow);
        app.queue_followup(&id, "next").unwrap();
        let info = app.session_info_for(0, 0, 0, now).unwrap();
        assert_eq!(info.decision.as_deref(), Some("✓ allowed Bash"));
        assert_eq!(info.queued, 1);
        app.store.projects[0].features[0].is_worktree = true;
        app.store.projects[0].features[0].workdir =
            PathBuf::from("/x/y/z/.local/share/ai_workstation/worktrees/RS/hx-12");
        assert_eq!(
            app.session_info_for(0, 0, 0, now)
                .unwrap()
                .worktree
                .as_deref(),
            Some("…/RS/hx-12")
        );
    }

    #[test]
    fn the_target_is_the_selected_session_or_the_pane_agent() {
        let mut app = app();
        app.selection = Selection::Session(0, 0, 1);
        assert_eq!(app.session_info_target(), Some((0, 0, 1)));
        app.selection = Selection::Feature(0, 0);
        assert_eq!(app.session_info_target(), None, "nothing committed yet");
        app.workspace_tick();
        std::thread::sleep(
            crate::app::fork_workspace::SELECTION_DEBOUNCE + std::time::Duration::from_millis(20),
        );
        app.workspace_tick();
        assert_eq!(app.session_info_target(), Some((0, 0, 0)));
        app.selection = Selection::Project(0);
        assert_eq!(app.session_info_target(), Some((0, 0, 0)));
        // A terminal row has no block.
        app.store.projects[0].features[0].add_session(SessionKind::Terminal);
        app.selection = Selection::Session(0, 0, 2);
        assert_eq!(app.session_info_target(), None);
    }

    #[test]
    fn the_sweep_reads_fixture_transcripts_under_a_private_home() {
        let home = tempfile::tempdir().unwrap();
        let workdir = home.path().join("repo");
        std::fs::create_dir_all(&workdir).unwrap();
        let transcript = claude_transcript_path(home.path(), &workdir, "c-1");
        std::fs::create_dir_all(transcript.parent().unwrap()).unwrap();
        std::fs::write(
            &transcript,
            concat!(
                r#"{"type":"user","message":{"role":"user","content":"Fix the login redirect loop"}}"#,
                "\n",
                r#"{"type":"assistant","message":{"role":"assistant","model":"claude-opus-4","content":[{"type":"text","text":"ok"}]}}"#,
                "\n"
            ),
        )
        .unwrap();
        std::fs::create_dir_all(workdir.join(".claude")).unwrap();
        std::fs::write(workdir.join(".claude/latest-prompt.txt"), "the task\nmore").unwrap();
        let jobs = vec![
            InfoJob {
                session_id: "s-claude".into(),
                kind: SessionKind::Claude,
                workdir: workdir.clone(),
                harness_session_id: Some("c-1".into()),
                codex_thread_id: None,
            },
            InfoJob {
                session_id: "s-unknown".into(),
                kind: SessionKind::Claude,
                workdir: workdir.clone(),
                harness_session_id: None,
                codex_thread_id: None,
            },
        ];
        let sweep = run_jobs(InfoScans::default(), jobs, home.path());
        assert_eq!(
            sweep.updates[0].1,
            TranscriptInfo {
                title: Some("Fix the login redirect loop".into()),
                model: Some("claude-opus-4".into()),
                task_prompt: Some("the task".into()),
            }
        );
        assert_eq!(
            sweep.updates[1].1,
            TranscriptInfo {
                title: None,
                model: None,
                task_prompt: Some("the task".into()),
            }
        );
        // A summary appended later is picked up by the incremental fold.
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&transcript)
            .unwrap();
        use std::io::Write;
        writeln!(
            file,
            r#"{{"type":"summary","summary":"Redirect loop fixed"}}"#
        )
        .unwrap();
        let again = run_jobs(
            sweep.scans,
            vec![InfoJob {
                session_id: "s-claude".into(),
                kind: SessionKind::Claude,
                workdir,
                harness_session_id: Some("c-1".into()),
                codex_thread_id: None,
            }],
            home.path(),
        );
        assert_eq!(
            again.updates[0].1.title.as_deref(),
            Some("Redirect loop fixed")
        );
    }

    #[test]
    fn the_sweep_only_runs_in_the_cockpit_and_once_at_a_time() {
        let mut app = workspace_app(&[SessionKind::Claude]);
        app.session_info_tick();
        assert!(app.fork_session_info.bg.is_none(), "no cockpit: no sweep");
        app.cockpit_attach(
            Link::recording(),
            "/bin/aiw".into(),
            "/s/cockpit.sock".into(),
        );
        app.session_info_tick();
        assert!(app.fork_session_info.bg.is_some());
        for _ in 0..50 {
            if app.poll_session_info_bg() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(app.fork_session_info.bg.is_none());
        let id = app.store.projects[0].features[0].sessions[0].id.clone();
        assert!(app.fork_session_info.cache.contains_key(&id));
    }
}
