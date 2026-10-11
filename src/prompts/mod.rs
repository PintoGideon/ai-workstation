//! Central registry of every headless AI prompt AIW sends.
//!
//! Historically each headless call site (`app/review/`,
//! `app/ai_review.rs`, `app/pr_review/`,
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
    /// Review questions: answer a reviewer's question about the code under
    /// Final Review, a manual PR review, or the AI PR pane
    /// (`app/review_questions/`, read-only tools).
    ReviewQuestion,
    /// Review questions: turn an answer into an inline or general review
    /// comment draft (`app/review_questions/`, no repository tools).
    ReviewQuestionDraft,
    /// Final Review: plain-language walkthrough of a file's diff (Claude).
    ReviewWalkthrough,
    /// Final Review: AI co-reviewer first pass over a file (Claude).
    ReviewCoReview,
    /// Final Review: whole-changeset triage overview + risk markers (Claude).
    ReviewChangesetOverview,
    /// Config-wizard / hook diff-review explanation (Claude).
    ReviewDiffExplain,
    /// The AI PR review pane (`app/ai_review.rs`).
    PrReviewAiReview,
    /// Review-memory lookback bootstrap (`app/pr_review.rs`, Claude).
    ReviewMemoryBootstrap,
    /// Review-memory compaction (`app/pr_review.rs`, Claude).
    ReviewMemoryCompact,
    /// Review-memory AI summary: turns one PR review comment into a
    /// recall-shaped finding (`app/pr_review/memory.rs`, the "add to memory"
    /// dialog's `s` action, user-picked harness, restricted no-tools mode).
    ReviewMemoryAiSummary,
    /// One-line session summary from tmux pane content (`summary.rs`).
    SessionSummary,
    /// Batched review: one bounded slice of an oversized diff
    /// (`review_batch.rs`).
    ReviewBatch,
    /// Batched review: one hunk group of a file too large to review whole.
    ReviewHunkSplit,
    /// Batched review: combine every slice's findings into one review.
    ReviewSynthesis,
    /// Batched review: shrink one slice's findings when synthesis overflows.
    ReviewFindingsSummary,
}

impl PromptId {
    /// Every registered prompt, in registry (and manager-list) order.
    pub const ALL: [PromptId; 15] = [
        PromptId::ReviewQuestion,
        PromptId::ReviewQuestionDraft,
        PromptId::ReviewWalkthrough,
        PromptId::ReviewCoReview,
        PromptId::ReviewChangesetOverview,
        PromptId::ReviewDiffExplain,
        PromptId::PrReviewAiReview,
        PromptId::ReviewMemoryBootstrap,
        PromptId::ReviewMemoryCompact,
        PromptId::ReviewMemoryAiSummary,
        PromptId::SessionSummary,
        PromptId::ReviewBatch,
        PromptId::ReviewHunkSplit,
        PromptId::ReviewSynthesis,
        PromptId::ReviewFindingsSummary,
    ];

    /// The stable string key. Used verbatim as the SQLite `prompt_id` and the
    /// `.aiw/prompts/` file stem — do not change for an existing prompt.
    pub fn as_str(self) -> &'static str {
        match self {
            PromptId::ReviewQuestion => "review.question",
            PromptId::ReviewQuestionDraft => "review.question_draft",
            PromptId::ReviewWalkthrough => "review.walkthrough",
            PromptId::ReviewCoReview => "review.co_review",
            PromptId::ReviewChangesetOverview => "review.changeset_overview",
            PromptId::ReviewDiffExplain => "review.diff_explain",
            PromptId::PrReviewAiReview => "pr_review.ai_review",
            PromptId::ReviewMemoryBootstrap => "review_memory.bootstrap",
            PromptId::ReviewMemoryCompact => "review_memory.compact",
            PromptId::ReviewMemoryAiSummary => "review_memory.ai_summary",
            PromptId::SessionSummary => "session.summary",
            PromptId::ReviewBatch => "review.batch",
            PromptId::ReviewHunkSplit => "review.hunk_split",
            PromptId::ReviewSynthesis => "review.synthesis",
            PromptId::ReviewFindingsSummary => "review.findings_summary",
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

static SPECS: [PromptSpec; 15] = [
    PromptSpec {
        id: PromptId::ReviewQuestion,
        title: "Review: ask AI",
        summary: "Answers reviewer questions with read-only repository discovery.",
        placeholders: &[
            "review_identity",
            "repository_path",
            "review_revision",
            "context_version",
            "selection",
            "diff",
            "earlier_turns",
            "question",
        ],
        default_template: defaults::REVIEW_QUESTION,
        harness_variants: NO_HARNESS_VARIANTS,
    },
    PromptSpec {
        id: PromptId::ReviewQuestionDraft,
        title: "Review: draft comment from answer",
        summary: "Turns an answer into editable local review-comment text.",
        placeholders: &[
            "review_identity",
            "repository_path",
            "review_revision",
            "context_version",
            "selection",
            "diff",
            "question",
            "answer",
        ],
        default_template: defaults::REVIEW_QUESTION_DRAFT,
        harness_variants: NO_HARNESS_VARIANTS,
    },
    PromptSpec {
        id: PromptId::ReviewWalkthrough,
        title: "Final Review: file walkthrough",
        summary: "Plain-language explanation of one file's diff during final review.",
        placeholders: &["file_path", "patch"],
        default_template: defaults::REVIEW_WALKTHROUGH,
        harness_variants: NO_HARNESS_VARIANTS,
    },
    PromptSpec {
        id: PromptId::ReviewCoReview,
        title: "Final Review: AI co-review",
        summary: "First-pass findings over one file, one `<line>|<comment>` per line.",
        placeholders: &["file_path", "annotated_body"],
        default_template: defaults::REVIEW_CO_REVIEW,
        harness_variants: NO_HARNESS_VARIANTS,
    },
    PromptSpec {
        id: PromptId::ReviewChangesetOverview,
        title: "Final Review: changeset overview",
        summary: "Whole-changeset triage overview and risk-factor list.",
        placeholders: &["files_block"],
        default_template: defaults::REVIEW_CHANGESET_OVERVIEW,
        harness_variants: NO_HARNESS_VARIANTS,
    },
    PromptSpec {
        id: PromptId::ReviewDiffExplain,
        title: "Diff review: explanation",
        summary: "Explains a code change shown in the config-wizard / hook diff review.",
        placeholders: &["file_path", "old_snippet", "new_snippet"],
        default_template: defaults::REVIEW_DIFF_EXPLAIN,
        harness_variants: NO_HARNESS_VARIANTS,
    },
    PromptSpec {
        id: PromptId::PrReviewAiReview,
        title: "PR review: AI review",
        summary: "Reviews a PR diff for bugs and quality issues in a fixed output format.",
        placeholders: &[
            "skill_directive",
            "recurring_findings",
            "annotated_diff",
            "finding_heading_prefix",
        ],
        default_template: defaults::PR_REVIEW_AI_REVIEW,
        harness_variants: NO_HARNESS_VARIANTS,
    },
    PromptSpec {
        id: PromptId::ReviewMemoryBootstrap,
        title: "Review memory: bootstrap",
        summary: "Distills recurring review findings from a project's PR history.",
        placeholders: &["pr_history"],
        default_template: defaults::REVIEW_MEMORY_BOOTSTRAP,
        harness_variants: NO_HARNESS_VARIANTS,
    },
    PromptSpec {
        id: PromptId::ReviewMemoryCompact,
        title: "Review memory: compact",
        summary: "Merges near-duplicate findings and prunes the review-memory doc.",
        placeholders: &["doc_contents"],
        default_template: defaults::REVIEW_MEMORY_COMPACT,
        harness_variants: NO_HARNESS_VARIANTS,
    },
    PromptSpec {
        id: PromptId::ReviewMemoryAiSummary,
        title: "Review memory: AI summary",
        summary: "Turns one PR review comment into a recall-shaped review-memory finding.",
        placeholders: &["finding_context"],
        default_template: defaults::REVIEW_MEMORY_AI_SUMMARY,
        harness_variants: NO_HARNESS_VARIANTS,
    },
    PromptSpec {
        id: PromptId::SessionSummary,
        title: "Session summary",
        summary: "One-line summary of a session's recent tmux output.",
        placeholders: &["harness_name", "max_chars", "recent_lines"],
        default_template: defaults::SESSION_SUMMARY,
        harness_variants: NO_HARNESS_VARIANTS,
    },
    PromptSpec {
        id: PromptId::ReviewBatch,
        title: "Batched review: diff slice",
        summary: "Reviews one bounded slice of a diff too large to review at once.",
        placeholders: &[
            "skill_directive",
            "recurring_findings",
            "file_list",
            "annotated_diff",
            "finding_heading_prefix",
        ],
        default_template: defaults::REVIEW_BATCH,
        harness_variants: NO_HARNESS_VARIANTS,
    },
    PromptSpec {
        id: PromptId::ReviewHunkSplit,
        title: "Batched review: hunk slice",
        summary: "Reviews one hunk group of a file too large to review whole.",
        placeholders: &[
            "file_path",
            "hunk_label",
            "annotated_diff",
            "finding_heading_prefix",
        ],
        default_template: defaults::REVIEW_HUNK_SPLIT,
        harness_variants: NO_HARNESS_VARIANTS,
    },
    PromptSpec {
        id: PromptId::ReviewSynthesis,
        title: "Batched review: synthesis",
        summary: "Combines every slice's findings into one review with a summary.",
        placeholders: &["batch_findings", "uncovered_note", "finding_heading_prefix"],
        default_template: defaults::REVIEW_SYNTHESIS,
        harness_variants: NO_HARNESS_VARIANTS,
    },
    PromptSpec {
        id: PromptId::ReviewFindingsSummary,
        title: "Batched review: findings summary",
        summary: "Shrinks one slice's findings when the synthesis prompt overflows.",
        placeholders: &["batch_label", "findings", "max_chars"],
        default_template: defaults::REVIEW_FINDINGS_SUMMARY,
        harness_variants: NO_HARNESS_VARIANTS,
    },
];

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
    fn batched_review_prompt_keys_are_stable() {
        // These are written to override rows and `aiw.json`; a rename orphans
        // committed overrides.
        for (id, key) in [
            (PromptId::ReviewBatch, "review.batch"),
            (PromptId::ReviewHunkSplit, "review.hunk_split"),
            (PromptId::ReviewSynthesis, "review.synthesis"),
            (PromptId::ReviewFindingsSummary, "review.findings_summary"),
        ] {
            assert_eq!(id.as_str(), key);
            assert_eq!(PromptId::from_key(key), Some(id));
            assert!(!spec(id).default_template.trim().is_empty());
        }
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
