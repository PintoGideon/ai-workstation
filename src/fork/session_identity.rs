//! FORK: the identity an agent session carries into its harness
//! (DESIGN.md §19.2).
//!
//! - **Names.** Every Claude session is launched with `--name <name>`, a
//!   stable slug of its feature and label (`auth-fix-claude-1`). The name is
//!   assigned once, stored in `aiw_session_names`, and kept when the session
//!   is renamed in AIW, so `claude --resume <name>`, `/list-agents`, agent
//!   view and (later) cross-session messages all see the same handle. Codex
//!   0.162 has no launch-time name flag, so Codex sessions get no name.
//! - **Forks.** A session can start from another session's conversation:
//!   Claude resumes the source with `--fork-session` (a new conversation id,
//!   the source untouched), Codex runs `codex … fork <id>`. The fork source
//!   is held in memory until the new session's first launch.

use std::collections::HashMap;

use crate::project::{AgentKind, slugify};

/// Longest name AIW gives a harness session. Claude shows the name in its
/// prompt bar and the terminal title; a short handle is easier to type.
pub const MAX_NAME_LEN: usize = 60;

/// The name a session would get before collision handling:
/// `<feature>-<label>`, slugged and cut to [`MAX_NAME_LEN`].
pub fn base_name(feature_name: &str, label: &str) -> String {
    let feature = slugify(feature_name);
    let label = slugify(label);
    let joined = match (feature.is_empty(), label.is_empty()) {
        (true, true) => "aiw-session".to_string(),
        (true, false) => label,
        (false, true) => feature,
        (false, false) => format!("{feature}-{label}"),
    };
    cut(&joined, MAX_NAME_LEN)
}

/// `s` cut to at most `max` characters, never ending on a dash.
fn cut(s: &str, max: usize) -> String {
    let cut: String = s.chars().take(max).collect();
    cut.trim_end_matches('-').to_string()
}

/// The first characters of a session id that are letters or digits, used to
/// keep two names apart.
fn short_id(session_id: &str) -> String {
    session_id
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(6)
        .collect::<String>()
        .to_lowercase()
}

/// Names assigned to feature sessions, mirroring `aiw_session_names`.
#[derive(Debug, Default, Clone)]
pub struct SessionNames {
    by_session: HashMap<String, String>,
    /// Assigned since the last save, in assignment order.
    unsaved: Vec<(String, String)>,
}

impl SessionNames {
    pub fn from_map(by_session: HashMap<String, String>) -> Self {
        Self {
            by_session,
            unsaved: Vec::new(),
        }
    }

    /// The session's name, if it has one.
    pub fn get(&self, session_id: &str) -> Option<&str> {
        self.by_session.get(session_id).map(String::as_str)
    }

    /// The session that carries `name`, if any.
    pub fn session_for_name(&self, name: &str) -> Option<&str> {
        self.by_session
            .iter()
            .find(|(_, n)| n.as_str() == name)
            .map(|(id, _)| id.as_str())
    }

    /// The session's name, assigning one on first use. A name another
    /// session already carries gets the session id's first characters
    /// appended, so names stay unique among AIW's sessions.
    pub fn assign(&mut self, session_id: &str, feature_name: &str, label: &str) -> String {
        if let Some(name) = self.by_session.get(session_id) {
            return name.clone();
        }
        let base = base_name(feature_name, label);
        let taken = |candidate: &str| {
            self.by_session
                .iter()
                .any(|(id, n)| id != session_id && n == candidate)
        };
        let name = if taken(&base) {
            let suffix = short_id(session_id);
            let room = MAX_NAME_LEN.saturating_sub(suffix.len() + 1);
            let mut candidate = format!("{}-{suffix}", cut(&base, room));
            let mut n = 2;
            while taken(&candidate) {
                candidate = format!("{}-{suffix}{n}", cut(&base, room.saturating_sub(2)));
                n += 1;
            }
            candidate
        } else {
            base
        };
        self.by_session.insert(session_id.to_string(), name.clone());
        self.unsaved.push((session_id.to_string(), name.clone()));
        name
    }

    /// Names assigned since the last call, for the caller to persist.
    pub fn take_unsaved(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.unsaved)
    }

    /// Put back names whose save failed, so the next save retries them.
    pub fn restore_unsaved(&mut self, mut names: Vec<(String, String)>) {
        names.append(&mut self.unsaved);
        self.unsaved = names;
    }

    /// Forget a deleted session's name. Returns whether it had one.
    pub fn forget(&mut self, session_id: &str) -> bool {
        self.unsaved.retain(|(id, _)| id != session_id);
        self.by_session.remove(session_id).is_some()
    }
}

/// Where a session's next launch starts its conversation from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForkSource {
    pub agent: AgentKind,
    /// The source conversation: Claude's session id, Codex's thread id.
    pub harness_session_id: String,
    /// The source session's label, for messages.
    pub source_label: String,
}

impl ForkSource {
    /// Whether this source can be forked by a session of `agent`.
    pub fn applies_to(&self, agent: &AgentKind) -> bool {
        &self.agent == agent
    }
}

/// Claude's resume id and extra flag for a fork: resume the source and ask
/// for a new conversation id. `args` gains `--fork-session`.
pub fn claude_fork(source: &ForkSource, args: &mut Vec<String>) -> String {
    if !args.iter().any(|a| a == "--fork-session") {
        args.push("--fork-session".to_string());
    }
    source.harness_session_id.clone()
}

/// Codex's arguments for a fork: the usual options, then `fork <id>`.
pub fn codex_fork_args(source: &ForkSource, mut args: Vec<String>) -> Vec<String> {
    args.push("fork".to_string());
    args.push(source.harness_session_id.clone());
    args
}

/// `args` with `--name <name>` added, unless a name is already there.
pub fn with_claude_name(mut args: Vec<String>, name: &str) -> Vec<String> {
    let named = args.iter().any(|a| a == "--name" || a == "-n");
    if !named && !name.is_empty() {
        args.push("--name".to_string());
        args.push(name.to_string());
    }
    args
}

/// The label of a session forked from one labelled `label`, unique among
/// `existing` labels: `Claude 1 fork`, then `Claude 1 fork 2`, …
pub fn fork_label(label: &str, existing: &[&str]) -> String {
    let base = format!("{label} fork");
    if !existing.contains(&base.as_str()) {
        return base;
    }
    (2..)
        .map(|n| format!("{base} {n}"))
        .find(|candidate| !existing.contains(&candidate.as_str()))
        .unwrap_or(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_names_slug_the_feature_and_the_label() {
        assert_eq!(base_name("auth-fix", "Claude 1"), "auth-fix-claude-1");
        assert_eq!(base_name("Auth Fix!", "Claude"), "auth-fix-claude");
        assert_eq!(base_name("", "Claude 1"), "claude-1");
        assert_eq!(base_name("feat", ""), "feat");
        assert_eq!(base_name("", ""), "aiw-session");
        let long = base_name(&"x".repeat(100), "Claude 1");
        assert!(long.len() <= MAX_NAME_LEN);
        assert!(!long.ends_with('-'));
    }

    #[test]
    fn names_are_assigned_once_and_kept_unique() {
        let mut names = SessionNames::default();
        let a = names.assign("sess-aaaa1111", "auth", "Claude 1");
        assert_eq!(a, "auth-claude-1");
        // Same session: same name, even with a new label (a rename in AIW).
        assert_eq!(names.assign("sess-aaaa1111", "auth", "Renamed"), a);
        // Another session with the same base gets the id suffix.
        let b = names.assign("sess-bbbb2222", "auth", "Claude 1");
        assert_eq!(b, "auth-claude-1-sessbb");
        assert_ne!(a, b);
        assert_eq!(names.session_for_name(&b), Some("sess-bbbb2222"));
        assert_eq!(names.take_unsaved().len(), 2);
        assert!(names.take_unsaved().is_empty());
        assert!(names.forget("sess-aaaa1111"));
        assert!(!names.forget("sess-aaaa1111"));
        assert_eq!(names.get("sess-aaaa1111"), None);
        assert_eq!(names.get("sess-bbbb2222"), Some(b.as_str()));
    }

    #[test]
    fn a_third_colliding_session_with_the_same_id_prefix_still_gets_a_unique_name() {
        let mut names = SessionNames::default();
        let a = names.assign("abcdef-1", "f", "Claude 1");
        let b = names.assign("abcdef-2", "f", "Claude 1");
        let c = names.assign("abcdef-3", "f", "Claude 1");
        assert_eq!(a, "f-claude-1");
        assert_eq!(b, "f-claude-1-abcdef");
        assert_eq!(c, "f-claude-1-abcdef2");
    }

    #[test]
    fn failed_saves_are_retried() {
        let mut names = SessionNames::default();
        names.assign("s1", "f", "Claude 1");
        let pending = names.take_unsaved();
        names.assign("s2", "f", "Claude 2");
        names.restore_unsaved(pending);
        let retried: Vec<_> = names.take_unsaved().into_iter().map(|(id, _)| id).collect();
        assert_eq!(retried, ["s1", "s2"]);
    }

    #[test]
    fn fork_arguments_for_each_harness() {
        let source = ForkSource {
            agent: AgentKind::Claude,
            harness_session_id: "uuid-1".into(),
            source_label: "Claude 1".into(),
        };
        let mut args = vec!["--chrome".to_string()];
        assert_eq!(claude_fork(&source, &mut args), "uuid-1");
        assert_eq!(args, ["--chrome", "--fork-session"]);
        claude_fork(&source, &mut args);
        assert_eq!(args.iter().filter(|a| *a == "--fork-session").count(), 1);
        assert!(source.applies_to(&AgentKind::Claude));
        assert!(!source.applies_to(&AgentKind::Codex));

        let codex = ForkSource {
            agent: AgentKind::Codex,
            harness_session_id: "thread-9".into(),
            source_label: "Codex 1".into(),
        };
        assert_eq!(
            codex_fork_args(&codex, vec!["-C".into(), "/w".into()]),
            ["-C", "/w", "fork", "thread-9"]
        );
    }

    #[test]
    fn claude_gets_one_name() {
        assert_eq!(with_claude_name(vec![], "a-b"), ["--name", "a-b"]);
        assert_eq!(
            with_claude_name(vec!["-n".into(), "mine".into()], "a-b"),
            ["-n", "mine"]
        );
        assert!(with_claude_name(vec![], "").is_empty());
    }

    #[test]
    fn fork_labels_count_up() {
        assert_eq!(fork_label("Claude 1", &[]), "Claude 1 fork");
        assert_eq!(
            fork_label("Claude 1", &["Claude 1", "Claude 1 fork"]),
            "Claude 1 fork 2"
        );
        assert_eq!(
            fork_label("Claude 1", &["Claude 1 fork", "Claude 1 fork 2"]),
            "Claude 1 fork 3"
        );
    }
}
