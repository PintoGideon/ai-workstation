//! FORK: feature groups, notes and layout presets (DESIGN.md §19.4). Pure
//! data and rules; the App side lives in `src/app/fork_organize.rs`, the
//! tables in `src/db/organize.rs`.
//!
//! - A **group** is a named, coloured set of features inside one project.
//!   It is the user's own filing, unrelated to a fan-out group (§19.3): a
//!   fan-out sibling set can sit inside a user group.
//! - A **note** is free text on a feature or a session.
//! - A **layout preset** (`layout_presets` in `aiw.json` or the global
//!   extension config) is a list of extra sessions opened when a feature
//!   starts: more agents, terminals (optionally running a command), Neovim
//!   and named custom sessions.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::project::AgentKind;

// ── groups ───────────────────────────────────────────────────────

/// The colours a group can take, in the order `c` cycles through them.
/// Named ANSI colours, so every theme and colour depth renders them.
pub const GROUP_COLORS: [&str; 6] = ["cyan", "green", "yellow", "magenta", "blue", "red"];

/// The longest group name accepted.
pub const MAX_GROUP_NAME: usize = 40;

/// A user group of features within a project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureGroup {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub color: String,
    pub position: i64,
    pub collapsed: bool,
}

/// The colour a new group gets: the first palette colour no group of the
/// project uses yet, else the palette in turn.
pub fn default_color(used: &[&str]) -> &'static str {
    GROUP_COLORS
        .iter()
        .find(|c| !used.contains(c))
        .copied()
        .unwrap_or(GROUP_COLORS[used.len() % GROUP_COLORS.len()])
}

/// The colour after `current` in the palette (an unknown name starts over).
pub fn next_color(current: &str) -> &'static str {
    match GROUP_COLORS.iter().position(|c| *c == current) {
        Some(i) => GROUP_COLORS[(i + 1) % GROUP_COLORS.len()],
        None => GROUP_COLORS[0],
    }
}

/// Why `name` cannot name a group among `others` (the project's other
/// group names), if it cannot.
pub fn group_name_error(name: &str, others: &[&str]) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        return Some("Group name cannot be empty".into());
    }
    if name.chars().count() > MAX_GROUP_NAME {
        return Some(format!(
            "Group name is longer than {MAX_GROUP_NAME} characters"
        ));
    }
    if others.iter().any(|o| o.trim().eq_ignore_ascii_case(name)) {
        return Some(format!("A group named '{name}' already exists"));
    }
    None
}

/// What ordering needs to know about one feature.
#[derive(Debug, Clone)]
pub struct FeatureKey {
    pub index: usize,
    pub is_worktree: bool,
    pub created_at: DateTime<Utc>,
    /// `(group position, group id)` when the feature is in a group.
    pub group: Option<(i64, String)>,
}

/// The display order of a project's features: ungrouped features first, in
/// the dashboard's usual order (the repo-dir feature, then creation order),
/// then each group by position, its members in the same usual order.
pub fn order_features(keys: &[FeatureKey]) -> Vec<usize> {
    let mut sorted: Vec<&FeatureKey> = keys.iter().collect();
    sorted.sort_by(|a, b| {
        let group_a = a.group.as_ref().map(|(p, id)| (*p, id.as_str()));
        let group_b = b.group.as_ref().map(|(p, id)| (*p, id.as_str()));
        // `None` (ungrouped) sorts before every `Some`.
        group_a
            .cmp(&group_b)
            .then_with(|| a.is_worktree.cmp(&b.is_worktree))
            .then_with(|| a.created_at.cmp(&b.created_at))
            .then_with(|| a.index.cmp(&b.index))
    });
    sorted.into_iter().map(|k| k.index).collect()
}

/// Where a feature sits in the sidebar once groups are applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureSlot {
    /// Not in a group.
    Ungrouped,
    /// The first feature of an expanded group: the group header is drawn
    /// above it.
    GroupHead,
    /// Any later feature of an expanded group.
    GroupMember,
    /// The first feature of a collapsed group: its row stands for the whole
    /// group.
    CollapsedGroup,
    /// A later feature of a collapsed group: not shown.
    Hidden,
}

/// Each feature of `order` (display order) with its slot, given each
/// feature's group id (`None` = ungrouped) and whether that group is
/// collapsed.
pub fn feature_slots(
    order: &[usize],
    group_of: impl Fn(usize) -> Option<(String, bool)>,
) -> Vec<(usize, FeatureSlot)> {
    let mut seen: Vec<String> = Vec::new();
    order
        .iter()
        .map(|&fi| {
            let slot = match group_of(fi) {
                None => FeatureSlot::Ungrouped,
                Some((id, collapsed)) => {
                    let first = !seen.contains(&id);
                    if first {
                        seen.push(id);
                    }
                    match (first, collapsed) {
                        (true, false) => FeatureSlot::GroupHead,
                        (false, false) => FeatureSlot::GroupMember,
                        (true, true) => FeatureSlot::CollapsedGroup,
                        (false, true) => FeatureSlot::Hidden,
                    }
                }
            };
            (fi, slot)
        })
        .collect()
}

// ── notes ────────────────────────────────────────────────────────

/// What a note is attached to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NoteTarget {
    Feature(String),
    Session(String),
}

impl NoteTarget {
    pub fn kind(&self) -> &'static str {
        match self {
            NoteTarget::Feature(_) => "feature",
            NoteTarget::Session(_) => "session",
        }
    }

    pub fn id(&self) -> &str {
        match self {
            NoteTarget::Feature(id) | NoteTarget::Session(id) => id,
        }
    }

    pub fn from_parts(kind: &str, id: String) -> Option<Self> {
        match kind {
            "feature" => Some(NoteTarget::Feature(id)),
            "session" => Some(NoteTarget::Session(id)),
            _ => None,
        }
    }
}

/// The note to store for `text`: trailing whitespace trimmed per line and
/// blank lines at either end dropped; `None` when nothing is left (saving
/// an empty note deletes it).
pub fn normalize_note(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    let start = lines.iter().position(|l| !l.trim().is_empty())?;
    let end = lines.iter().rposition(|l| !l.trim().is_empty())?;
    Some(lines[start..=end].join("\n"))
}

/// The note's first non-blank line, trimmed.
pub fn note_first_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).find(|l| !l.is_empty())
}

/// Whether the note matches a search query (already lowercased).
pub fn note_matches(text: &str, query_lower: &str) -> bool {
    !query_lower.is_empty() && text.to_lowercase().contains(query_lower)
}

/// The line of the note the search result shows: the first line holding
/// the query, else the first line.
pub fn note_match_line<'a>(text: &'a str, query_lower: &str) -> &'a str {
    text.lines()
        .map(str::trim)
        .find(|l| !query_lower.is_empty() && l.to_lowercase().contains(query_lower))
        .or_else(|| note_first_line(text))
        .unwrap_or("")
}

// ── layout presets ───────────────────────────────────────────────

/// A named set of extra sessions opened when a feature starts
/// (`layout_presets` in `aiw.json` or the global extension config).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct LayoutPreset {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub sessions: Vec<LayoutSessionSpec>,
}

/// One session of a layout preset, as written in the config. Strings only,
/// so a typo is reported when the preset is used instead of failing the
/// whole config file.
///
/// ```json
/// { "kind": "terminal", "label": "dev server", "command": "npm run dev" }
/// { "kind": "agent", "agent": "codex" }
/// { "kind": "nvim" }
/// { "kind": "custom", "name": "tests" }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct LayoutSessionSpec {
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

/// A layout session checked against the project: what to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayoutSession {
    Agent {
        agent: AgentKind,
        label: Option<String>,
    },
    Terminal {
        label: Option<String>,
        command: Option<String>,
    },
    Nvim {
        label: Option<String>,
    },
    Custom {
        name: String,
        label: Option<String>,
    },
}

fn clean(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

/// Check `preset` against the project's custom session names and allowed
/// harnesses. The error names the preset and the offending entry, so it can
/// be shown as is.
pub fn resolve_layout(
    preset: &LayoutPreset,
    custom_names: &[&str],
    allowed_agents: &[AgentKind],
) -> Result<Vec<LayoutSession>, String> {
    let who = format!("layout '{}'", preset.name);
    if preset.sessions.is_empty() {
        return Err(format!("{who} has no sessions"));
    }
    let mut out = Vec::with_capacity(preset.sessions.len());
    for (i, spec) in preset.sessions.iter().enumerate() {
        let at = format!("{who}, session {}", i + 1);
        let label = clean(&spec.label);
        let command = clean(&spec.command);
        let kind = spec.kind.trim().to_ascii_lowercase();
        if command.is_some() && kind != "terminal" {
            return Err(format!(
                "{at}: \"command\" only applies to a terminal (use a custom session for a {kind} command)"
            ));
        }
        let session = match kind.as_str() {
            "agent" => {
                let Some(token) = clean(&spec.agent) else {
                    return Err(format!(
                        "{at}: an agent session needs \"agent\" (claude, codex, pi, gemini or cursor)"
                    ));
                };
                let Some(agent) = AgentKind::from_slug(&token.to_ascii_lowercase()) else {
                    return Err(format!("{at}: unknown agent '{token}'"));
                };
                if !allowed_agents.contains(&agent) {
                    return Err(format!(
                        "{at}: {} is not an enabled harness here",
                        agent.display_name()
                    ));
                }
                LayoutSession::Agent { agent, label }
            }
            "terminal" => LayoutSession::Terminal { label, command },
            "nvim" | "neovim" => LayoutSession::Nvim { label },
            "custom" => {
                let Some(name) = clean(&spec.name) else {
                    return Err(format!(
                        "{at}: a custom session needs \"name\" (one of custom_sessions)"
                    ));
                };
                if !custom_names.iter().any(|n| *n == name) {
                    let known = if custom_names.is_empty() {
                        "none are defined".to_string()
                    } else {
                        format!("known: {}", custom_names.join(", "))
                    };
                    return Err(format!(
                        "{at}: custom session '{name}' is not in custom_sessions ({known})"
                    ));
                }
                LayoutSession::Custom { name, label }
            }
            "" => return Err(format!("{at}: missing \"kind\"")),
            other => {
                return Err(format!(
                    "{at}: unknown kind '{other}' (agent, terminal, nvim or custom)"
                ));
            }
        };
        out.push(session);
    }
    Ok(out)
}

/// A short summary of a preset for the dialog: `claude · terminal(npm run
/// dev) · tests`.
pub fn layout_summary(preset: &LayoutPreset) -> String {
    preset
        .sessions
        .iter()
        .map(|s| {
            let kind = s.kind.trim().to_ascii_lowercase();
            match kind.as_str() {
                "agent" => clean(&s.agent).unwrap_or_else(|| "agent".into()),
                "terminal" => match clean(&s.command) {
                    Some(cmd) => format!("terminal({cmd})"),
                    None => clean(&s.label).unwrap_or_else(|| "terminal".into()),
                },
                "custom" => clean(&s.name).unwrap_or_else(|| "custom".into()),
                other => other.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_800_000_000 + secs, 0).unwrap()
    }

    fn key(index: usize, wt: bool, t: i64, group: Option<(i64, &str)>) -> FeatureKey {
        FeatureKey {
            index,
            is_worktree: wt,
            created_at: at(t),
            group: group.map(|(p, id)| (p, id.to_string())),
        }
    }

    #[test]
    fn ungrouped_features_come_first_then_groups_by_position() {
        let keys = vec![
            key(0, true, 10, Some((1, "b"))),
            key(1, false, 50, None),
            key(2, true, 5, None),
            key(3, true, 1, Some((0, "a"))),
            key(4, true, 20, Some((1, "b"))),
            key(5, true, 3, Some((0, "a"))),
        ];
        // repo-dir feature (1) before worktrees, then creation order; group
        // a (position 0) before group b (position 1).
        assert_eq!(order_features(&keys), vec![1, 2, 3, 5, 0, 4]);
    }

    #[test]
    fn without_groups_the_order_is_the_old_one() {
        let keys = vec![
            key(0, true, 30, None),
            key(1, true, 10, None),
            key(2, false, 99, None),
        ];
        assert_eq!(order_features(&keys), vec![2, 1, 0]);
    }

    #[test]
    fn slots_mark_the_head_of_each_group_and_hide_collapsed_members() {
        let order = vec![1, 2, 3, 5, 0, 4];
        let groups = |fi: usize| match fi {
            3 | 5 => Some(("a".to_string(), false)),
            0 | 4 => Some(("b".to_string(), true)),
            _ => None,
        };
        let slots = feature_slots(&order, groups);
        assert_eq!(
            slots,
            vec![
                (1, FeatureSlot::Ungrouped),
                (2, FeatureSlot::Ungrouped),
                (3, FeatureSlot::GroupHead),
                (5, FeatureSlot::GroupMember),
                (0, FeatureSlot::CollapsedGroup),
                (4, FeatureSlot::Hidden),
            ]
        );
    }

    #[test]
    fn group_names_are_trimmed_unique_and_bounded() {
        assert_eq!(
            group_name_error("  ", &[]).as_deref(),
            Some("Group name cannot be empty")
        );
        assert!(group_name_error("Backend", &["backend"]).is_some());
        assert!(group_name_error(&"x".repeat(41), &[]).is_some());
        assert_eq!(group_name_error(" Frontend ", &["Backend"]), None);
    }

    #[test]
    fn colours_cycle_and_new_groups_take_an_unused_one() {
        assert_eq!(next_color("cyan"), "green");
        assert_eq!(next_color("red"), "cyan");
        assert_eq!(next_color("nope"), "cyan");
        assert_eq!(default_color(&[]), "cyan");
        assert_eq!(default_color(&["cyan", "green"]), "yellow");
        assert_eq!(default_color(&GROUP_COLORS), "cyan");
    }

    #[test]
    fn notes_are_trimmed_and_an_empty_note_is_none() {
        assert_eq!(normalize_note("  \n\n  "), None);
        assert_eq!(
            normalize_note("\n  waiting on API  \n second line \n\n").as_deref(),
            Some("  waiting on API\n second line")
        );
        assert_eq!(
            note_first_line("\n\n  hello world \nnext"),
            Some("hello world")
        );
        assert!(note_matches("Blocked on the Stripe webhook", "stripe"));
        assert!(!note_matches("anything", ""));
        assert_eq!(
            note_match_line("first\nthe stripe key\nlast", "stripe"),
            "the stripe key"
        );
        assert_eq!(note_match_line("first\nsecond", "zzz"), "first");
    }

    fn spec(kind: &str) -> LayoutSessionSpec {
        LayoutSessionSpec {
            kind: kind.into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_layout_resolves_every_kind() {
        let preset = LayoutPreset {
            name: "dev".into(),
            description: None,
            sessions: vec![
                LayoutSessionSpec {
                    agent: Some("Codex".into()),
                    ..spec("agent")
                },
                LayoutSessionSpec {
                    label: Some("dev server".into()),
                    command: Some("npm run dev".into()),
                    ..spec("terminal")
                },
                spec("nvim"),
                LayoutSessionSpec {
                    name: Some("tests".into()),
                    ..spec("custom")
                },
            ],
        };
        let resolved =
            resolve_layout(&preset, &["tests"], &[AgentKind::Claude, AgentKind::Codex]).unwrap();
        assert_eq!(
            resolved,
            vec![
                LayoutSession::Agent {
                    agent: AgentKind::Codex,
                    label: None
                },
                LayoutSession::Terminal {
                    label: Some("dev server".into()),
                    command: Some("npm run dev".into())
                },
                LayoutSession::Nvim { label: None },
                LayoutSession::Custom {
                    name: "tests".into(),
                    label: None
                },
            ]
        );
        assert_eq!(
            layout_summary(&preset),
            "Codex · terminal(npm run dev) · nvim · tests"
        );
    }

    #[test]
    fn layout_errors_name_the_preset_and_the_entry() {
        let one = |s: LayoutSessionSpec| LayoutPreset {
            name: "dev".into(),
            description: None,
            sessions: vec![s],
        };
        let allowed = [AgentKind::Claude];
        let err =
            |p: LayoutPreset, custom: &[&str]| resolve_layout(&p, custom, &allowed).unwrap_err();

        let e = err(
            one(LayoutSessionSpec {
                name: Some("server".into()),
                ..spec("custom")
            }),
            &["tests", "lint"],
        );
        assert_eq!(
            e,
            "layout 'dev', session 1: custom session 'server' is not in custom_sessions (known: tests, lint)"
        );
        assert!(err(one(spec("custom")), &[]).contains("needs \"name\""));
        assert!(
            err(
                one(LayoutSessionSpec {
                    name: Some("x".into()),
                    ..spec("custom")
                }),
                &[]
            )
            .contains("none are defined")
        );
        assert!(err(one(spec("agent")), &[]).contains("needs \"agent\""));
        assert!(
            err(
                one(LayoutSessionSpec {
                    agent: Some("codex".into()),
                    ..spec("agent")
                }),
                &[]
            )
            .contains("not an enabled harness")
        );
        assert!(
            err(
                one(LayoutSessionSpec {
                    agent: Some("gpt".into()),
                    ..spec("agent")
                }),
                &[]
            )
            .contains("unknown agent 'gpt'")
        );
        assert!(err(one(spec("browser")), &[]).contains("unknown kind 'browser'"));
        assert!(err(one(spec("")), &[]).contains("missing \"kind\""));
        assert!(
            err(
                one(LayoutSessionSpec {
                    command: Some("make".into()),
                    ..spec("nvim")
                }),
                &[]
            )
            .contains("only applies to a terminal")
        );
        let empty = LayoutPreset {
            name: "dev".into(),
            description: None,
            sessions: vec![],
        };
        assert_eq!(err(empty, &[]), "layout 'dev' has no sessions");
    }

    #[test]
    fn a_preset_with_unknown_strings_still_parses() {
        let json = r#"{ "name": "x", "sessions": [ { "kind": "rocket", "agent": "gpt-9" } ] }"#;
        let preset: LayoutPreset = serde_json::from_str(json).unwrap();
        assert_eq!(preset.sessions[0].kind, "rocket");
        let round = serde_json::to_value(&preset).unwrap();
        assert!(round["sessions"][0].get("label").is_none());
    }
}
