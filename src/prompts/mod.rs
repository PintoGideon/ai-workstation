//! Central registry of every headless AI prompt AIW sends.
//!
//! Historically each headless call site (`app/review/`,
//! `handlers/diff_review.rs`, `summary.rs`) assembled its prompt inline. This
//! module is the single place those templates live, so they can be listed,
//! displayed, and overridden.
//!
//! - This file: stable [`PromptId`]s, the built-in default template text
//!   (in [`defaults`]), the placeholder names each expects, and room for
//!   optional per-harness default variants.
//! - [`resolve`]: pick the effective template (feature → project → global →
//!   built-in) and interpolate the context with no validation.
//!
//! A full map of prompt IDs → placeholder sets → call sites lives in
//! `docs/backlog/editable-prompts-call-site-inventory.md`.

mod defaults;
pub mod project;
mod resolve;

#[cfg(test)]
pub use resolve::resolve_prompt;
pub use resolve::{
    PromptContext, PromptLayers, PromptSource, render_template, resolve_template_layered,
};

use crate::project::AgentKind;

/// Stable identifier for one headless prompt. The [`PromptId::as_str`] form is
/// the key used for SQLite override rows and `.aiw/prompts/` file names, so it
/// must never change for an existing prompt (renaming orphans committed
/// overrides).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PromptId {
    /// One-line session summary from tmux pane content (`summary.rs`).
    SessionSummary,
}

impl PromptId {
    /// Every registered prompt, in registry (and manager-list) order.
    pub const ALL: [PromptId; 1] = [PromptId::SessionSummary];

    /// The stable string key. Used verbatim as the SQLite `prompt_id` and the
    /// `.aiw/prompts/` file stem — do not change for an existing prompt.
    pub fn as_str(self) -> &'static str {
        match self {
            PromptId::SessionSummary => "session.summary",
        }
    }

    /// Resolve a stable key back to its [`PromptId`]. `None` for an unknown
    /// key — e.g. an orphaned override file left behind by a renamed prompt.
    pub fn from_key(key: &str) -> Option<PromptId> {
        PromptId::ALL.into_iter().find(|id| id.as_str() == key)
    }

    /// The registry entry for this prompt.
    pub fn spec(self) -> &'static PromptSpec {
        spec(self)
    }
}

/// One registry entry: the built-in default template and the metadata the
/// manager overlay and scope resolver need.
pub struct PromptSpec {
    pub id: PromptId,
    /// Short human label for the manager list.
    pub title: &'static str,
    /// One-line description of when this prompt runs.
    pub summary: &'static str,
    /// The placeholder names the built-in template splices in. Informational:
    /// the interpolator is driven by the caller's context map, not this list,
    /// and neither is validated against the other (a deliberate decision — an
    /// override may drop or add tokens freely).
    pub placeholders: &'static [&'static str],
    /// Built-in default, used when no override applies.
    pub default_template: &'static str,
    /// Optional per-harness replacements for [`Self::default_template`]. Empty
    /// for every prompt today; the field exists so a harness-specific default
    /// can be added later without touching call sites.
    pub harness_variants: &'static [(AgentKind, &'static str)],
}

impl PromptSpec {
    /// The built-in default for `harness`: its per-harness variant when one is
    /// registered, otherwise the shared [`Self::default_template`].
    pub fn default_template_for(&self, harness: &AgentKind) -> &'static str {
        self.harness_variants
            .iter()
            .find(|(kind, _)| kind == harness)
            .map(|(_, template)| *template)
            .unwrap_or(self.default_template)
    }
}

/// Every registry entry. Indexed by [`spec`]; iterate for the manager list.
#[allow(dead_code)] // exercised only by unit tests
pub fn all_specs() -> &'static [PromptSpec] {
    &SPECS
}

/// The registry entry for `id`. Infallible: [`SPECS`] is exhaustive over
/// [`PromptId`], enforced by [`tests::every_prompt_id_has_a_spec`].
pub fn spec(id: PromptId) -> &'static PromptSpec {
    SPECS
        .iter()
        .find(|spec| spec.id == id)
        .expect("SPECS is exhaustive over PromptId")
}

const NO_HARNESS_VARIANTS: &[(AgentKind, &str)] = &[];

static SPECS: [PromptSpec; 1] = [PromptSpec {
    id: PromptId::SessionSummary,
    title: "Session summary",
    summary: "One-line summary of a session's recent tmux output.",
    placeholders: &["harness_name", "max_chars", "recent_lines"],
    default_template: defaults::SESSION_SUMMARY,
    harness_variants: NO_HARNESS_VARIANTS,
}];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_prompt_id_has_a_spec_and_round_trips() {
        for id in PromptId::ALL {
            let spec = spec(id);
            assert_eq!(spec.id, id, "spec() returned the wrong entry");
            assert!(!spec.default_template.trim().is_empty());
            assert!(!spec.title.is_empty());
            assert!(!spec.summary.is_empty());
            assert_eq!(PromptId::from_key(id.as_str()), Some(id));
        }
    }

    #[test]
    fn specs_and_all_are_the_same_length_and_order() {
        assert_eq!(SPECS.len(), PromptId::ALL.len());
        for (spec, id) in SPECS.iter().zip(PromptId::ALL) {
            assert_eq!(spec.id, id);
        }
    }

    #[test]
    fn prompt_keys_are_unique() {
        let mut keys: Vec<&str> = PromptId::ALL.iter().map(|id| id.as_str()).collect();
        keys.sort_unstable();
        let before = keys.len();
        keys.dedup();
        assert_eq!(before, keys.len(), "duplicate prompt key");
    }

    #[test]
    fn from_key_rejects_unknown_keys() {
        assert_eq!(PromptId::from_key("nope.not.a.prompt"), None);
        assert_eq!(PromptId::from_key(""), None);
    }

    #[test]
    fn declared_placeholders_appear_in_the_builtin_template() {
        // The built-in defaults must be self-consistent even though runtime
        // interpolation never validates overrides against this list.
        for spec in all_specs() {
            for name in spec.placeholders {
                let token = format!("{{{{{name}}}}}");
                assert!(
                    spec.default_template.contains(&token),
                    "{}: declared placeholder {token} missing from its default template",
                    spec.id.as_str()
                );
            }
        }
    }

    #[test]
    fn default_template_for_falls_back_to_the_shared_template() {
        for spec in all_specs() {
            for harness in AgentKind::ALL {
                assert_eq!(
                    spec.default_template_for(&harness),
                    spec.default_template,
                    "{}: no harness variants are registered yet",
                    spec.id.as_str()
                );
            }
        }
    }
}
