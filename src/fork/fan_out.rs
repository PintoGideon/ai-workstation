//! FORK: one prompt, several agents — the pure pieces (DESIGN.md §17,
//! §19.3).
//!
//! The New Feature dialog can tick more than one harness. With one shared
//! worktree, the feature gets a window per agent (`Claude 1`, `Codex 1`);
//! with one worktree each, sibling features `<branch>-<agent>` are created
//! from the same base and tagged with a shared fan-out group, so the sidebar
//! draws them under one header and `v` tiles them.
//!
//! The task prompt is never typed blind into a starting pane. Each agent
//! session gets a [`PendingPrompt`] that is delivered once its pane is
//! ready: shortly after the harness's `SessionStart` hook reports in, or —
//! when no hook comes (an untrusted Codex hook file, an older harness) —
//! after a fallback delay once the pane runs the agent rather than a shell.
//! Delivery pastes the prompt unsent; with "send when ready" it is followed
//! by one Enter a moment later.

use std::time::{Duration, Instant};

use crate::project::{AgentKind, VibeMode};

/// After `SessionStart`, how long to let the agent's TUI finish drawing
/// before the paste (bracketed paste is enabled a beat after the hook).
pub const READY_SETTLE: Duration = Duration::from_millis(1500);

/// After the paste, how long to wait before the Enter of "send when ready",
/// so the TUI has taken the whole paste in first.
pub const ENTER_DELAY: Duration = Duration::from_millis(500);

/// Default seconds to wait for a `SessionStart` hook before falling back to
/// "the pane runs the agent" (config `fan_out_ready_fallback_secs`).
pub const DEFAULT_FALLBACK_SECS: u64 = 15;

/// How a multi-agent feature is laid out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FanOutLayout {
    /// One feature, one checkout, a window per agent.
    Shared,
    /// Sibling features, one worktree each, from the same base.
    #[default]
    WorktreeEach,
}

impl FanOutLayout {
    pub const ALL: [FanOutLayout; 2] = [FanOutLayout::WorktreeEach, FanOutLayout::Shared];

    /// The automation / preset spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            FanOutLayout::Shared => "shared",
            FanOutLayout::WorktreeEach => "worktree_each",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "shared" | "one_worktree" | "same" => Some(FanOutLayout::Shared),
            "worktree_each" | "each" | "worktrees" => Some(FanOutLayout::WorktreeEach),
            _ => None,
        }
    }

    /// The dialog's wording.
    pub fn label(self) -> &'static str {
        match self {
            FanOutLayout::Shared => "one worktree, shared",
            FanOutLayout::WorktreeEach => "one worktree each",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            FanOutLayout::Shared => {
                "agents share the files: good for questions, review and planning"
            }
            FanOutLayout::WorktreeEach => {
                "sibling features <branch>-<agent> from the same base: for code changes"
            }
        }
    }
}

impl serde::Serialize for FanOutLayout {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for FanOutLayout {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        FanOutLayout::parse(&raw).ok_or_else(|| {
            serde::de::Error::custom(format!(
                "unknown fan_out '{raw}' (use \"shared\" or \"worktree_each\")"
            ))
        })
    }
}

/// The short lowercase name used in branch names and group headers.
pub fn agent_slug(agent: &AgentKind) -> &'static str {
    match agent {
        AgentKind::Claude => "claude",
        AgentKind::Codex => "codex",
        AgentKind::Pi => "pi",
        AgentKind::Gemini => "gemini",
        AgentKind::Cursor => "cursor",
    }
}

/// The branch of the sibling that runs `agent`. A harness ticked twice gets
/// a number from its second copy on (`-claude`, `-claude-2`).
pub fn sibling_branch(branch: &str, agent: &AgentKind, copy: usize) -> String {
    let base = branch.trim().trim_end_matches('/');
    if copy <= 1 {
        format!("{base}-{}", agent_slug(agent))
    } else {
        format!("{base}-{}-{copy}", agent_slug(agent))
    }
}

/// `(agent, copy number)` for each ticked agent, in order: the copy number
/// counts how many times that harness appeared so far (1-based).
pub fn numbered(agents: &[AgentKind]) -> Vec<(AgentKind, usize)> {
    let mut seen: Vec<(AgentKind, usize)> = Vec::new();
    agents
        .iter()
        .map(|agent| {
            let n = match seen.iter_mut().find(|(a, _)| a == agent) {
                Some((_, n)) => {
                    *n += 1;
                    *n
                }
                None => {
                    seen.push((agent.clone(), 1));
                    1
                }
            };
            (agent.clone(), n)
        })
        .collect()
}

/// The sibling branches for `agents`, in order.
pub fn sibling_branches(branch: &str, agents: &[AgentKind]) -> Vec<String> {
    numbered(agents)
        .iter()
        .map(|(agent, n)| sibling_branch(branch, agent, *n))
        .collect()
}

/// The window labels of a shared fan-out: `Claude 1`, `Codex 1`,
/// `Claude 2`, … — unique within the feature.
pub fn shared_labels(agents: &[AgentKind]) -> Vec<String> {
    numbered(agents)
        .iter()
        .map(|(agent, n)| format!("{} {n}", agent.display_name()))
        .collect()
}

/// The mode every agent of the set runs in, plus a note when it had to
/// change: Codex refuses Vibeless (per-edit approval), so a set with Codex
/// falls back to Vibe.
pub fn fan_out_mode(agents: &[AgentKind], mode: &VibeMode) -> (VibeMode, Option<String>) {
    if matches!(mode, VibeMode::Vibeless) && agents.contains(&AgentKind::Codex) {
        return (
            VibeMode::Vibe,
            Some("Codex has no Vibeless per-edit approval, so every agent runs in Vibe".into()),
        );
    }
    (mode.clone(), None)
}

/// The sidebar header of a fan-out group: `⇶ auth-fix · claude / codex`.
pub fn group_header(base: &str, agents: &[AgentKind]) -> String {
    let names: Vec<&str> = agents.iter().map(agent_slug).collect();
    format!("⇶ {base} · {}", names.join(" / "))
}

/// Remove trailing blank lines and surrounding whitespace that would turn
/// into stray Enters, keeping inner newlines.
pub fn normalize_prompt(text: &str) -> String {
    text.trim_matches(|c: char| c == '\n' || c == '\r' || c == ' ' || c == '\t')
        .replace("\r\n", "\n")
        .replace('\r', "\n")
}

/// A prompt waiting for its agent's pane.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingPrompt {
    pub text: String,
    /// Press Enter after the paste.
    pub send: bool,
    /// Select the session and focus its pane on delivery (the primary agent
    /// of a dialog launch).
    pub focus: bool,
    /// When it was queued (or loaded at start-up).
    pub queued_at: Instant,
    /// Set by the `SessionStart` hook: deliver at this instant.
    pub ready_at: Option<Instant>,
}

impl PendingPrompt {
    pub fn new(text: String, send: bool, focus: bool, now: Instant) -> Self {
        Self {
            text,
            send,
            focus,
            queued_at: now,
            ready_at: None,
        }
    }

    /// The hook said the agent is up.
    pub fn mark_ready(&mut self, now: Instant) {
        let at = now + READY_SETTLE;
        self.ready_at = Some(self.ready_at.map_or(at, |old| old.min(at)));
    }

    /// What to do with it now.
    pub fn due(&self, now: Instant, fallback: Duration) -> Due {
        if self.ready_at.is_some_and(|at| now >= at) {
            return Due::Ready;
        }
        if self.ready_at.is_none() && now.duration_since(self.queued_at) >= fallback {
            return Due::IfAgentRuns;
        }
        Due::Wait
    }
}

/// Whether a pending prompt can go now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Due {
    /// Not yet.
    Wait,
    /// The hook reported ready and the settle time passed.
    Ready,
    /// No hook came in time: deliver if the pane runs the agent.
    IfAgentRuns,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sibling_branches_get_the_agent_and_a_copy_number() {
        let agents = [AgentKind::Claude, AgentKind::Codex, AgentKind::Claude];
        assert_eq!(
            sibling_branches("auth-fix", &agents),
            ["auth-fix-claude", "auth-fix-codex", "auth-fix-claude-2"]
        );
        assert_eq!(
            sibling_branch("feat/x/", &AgentKind::Gemini, 1),
            "feat/x-gemini"
        );
    }

    #[test]
    fn shared_labels_are_unique_within_the_feature() {
        let agents = [AgentKind::Claude, AgentKind::Codex, AgentKind::Claude];
        assert_eq!(shared_labels(&agents), ["Claude 1", "Codex 1", "Claude 2"]);
    }

    #[test]
    fn codex_in_the_set_turns_vibeless_into_vibe_with_a_note() {
        let (mode, note) =
            fan_out_mode(&[AgentKind::Claude, AgentKind::Codex], &VibeMode::Vibeless);
        assert_eq!(mode, VibeMode::Vibe);
        assert!(note.unwrap().contains("Vibe"));
        let (mode, note) =
            fan_out_mode(&[AgentKind::Claude, AgentKind::Codex], &VibeMode::SuperVibe);
        assert_eq!(mode, VibeMode::SuperVibe);
        assert!(note.is_none());
        let (mode, note) = fan_out_mode(&[AgentKind::Claude], &VibeMode::Vibeless);
        assert_eq!(mode, VibeMode::Vibeless);
        assert!(note.is_none());
    }

    #[test]
    fn the_group_header_names_every_agent() {
        assert_eq!(
            group_header("auth-fix", &[AgentKind::Claude, AgentKind::Codex]),
            "⇶ auth-fix · claude / codex"
        );
    }

    #[test]
    fn layouts_round_trip_through_their_spelling() {
        for layout in FanOutLayout::ALL {
            assert_eq!(FanOutLayout::parse(layout.as_str()), Some(layout));
            let json = serde_json::to_string(&layout).unwrap();
            assert_eq!(serde_json::from_str::<FanOutLayout>(&json).unwrap(), layout);
        }
        assert_eq!(
            FanOutLayout::parse("worktree-each"),
            Some(FanOutLayout::WorktreeEach)
        );
        assert!(serde_json::from_str::<FanOutLayout>("\"both\"").is_err());
    }

    #[test]
    fn prompts_lose_trailing_newlines_but_keep_inner_ones() {
        assert_eq!(normalize_prompt("\n  do x\r\nthen y\n\n"), "do x\nthen y");
    }

    #[test]
    fn a_pending_prompt_waits_for_the_hook_then_the_settle_time() {
        let t0 = Instant::now();
        let fallback = Duration::from_secs(15);
        let mut p = PendingPrompt::new("x".into(), false, true, t0);
        assert_eq!(p.due(t0, fallback), Due::Wait);
        p.mark_ready(t0 + Duration::from_secs(1));
        assert_eq!(p.due(t0 + Duration::from_secs(2), fallback), Due::Wait);
        assert_eq!(
            p.due(t0 + Duration::from_secs(1) + READY_SETTLE, fallback),
            Due::Ready
        );
        // A hook that came keeps the prompt off the fallback path.
        assert_eq!(p.due(t0 + Duration::from_secs(60), fallback), Due::Ready);
    }

    #[test]
    fn without_a_hook_the_fallback_asks_whether_the_agent_runs() {
        let t0 = Instant::now();
        let p = PendingPrompt::new("x".into(), true, false, t0);
        let fallback = Duration::from_secs(15);
        assert_eq!(p.due(t0 + Duration::from_secs(14), fallback), Due::Wait);
        assert_eq!(
            p.due(t0 + Duration::from_secs(15), fallback),
            Due::IfAgentRuns
        );
    }

    #[test]
    fn a_second_ready_never_postpones_delivery() {
        let t0 = Instant::now();
        let mut p = PendingPrompt::new("x".into(), false, false, t0);
        p.mark_ready(t0);
        p.mark_ready(t0 + Duration::from_secs(5));
        assert_eq!(p.ready_at, Some(t0 + READY_SETTLE));
    }
}
