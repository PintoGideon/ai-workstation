//! PR comment-review model and normalization (feature-specific).
//!
//! The generic `gh` access lives in [`crate::github`]; this module turns those
//! raw GitHub payloads into a single triage-ready [`PrReview`] and owns the
//! token-saving transforms (bot-boilerplate stripping, one-line snippets,
//! thread-resolution merge). See `docs/backlog/pr-comment-review-plan.md`.

mod actions;
mod domain;
mod fetch;
pub(crate) mod github_access;
mod integration;
mod investigation;
mod reply;
mod review_tab;
pub(crate) mod revisions;
pub(crate) mod state;

#[cfg(test)]
mod tests;
use actions::new_fix_confirm;
pub(crate) use actions::with_reply_draft_handoff;
pub(crate) use domain::AI_ATTRIBUTION_FOOTER;
#[cfg(test)]
pub(crate) use domain::AI_REVIEW_ATTRIBUTION_FOOTER;
pub(crate) use domain::AIW_ATTRIBUTION_FOOTER;
pub(crate) use domain::TRIAGE_SESSION_LABEL;
pub(crate) use domain::fix_session_index;
pub(crate) use domain::pr_triage_session_index;
#[cfg(test)]
pub(crate) use domain::pr_triage_session_index_named;
use domain::{
    BATCH_COMBINED_COMMENT_WARN, BATCH_COMBINED_TOKEN_WARN, SNIPPET_LEN, append_reply_attribution,
    file_already_touched, pr_triage_session_index_named_for_harness,
};
pub use domain::{
    CommentKind, FixTarget, FixTargetPickRow, MarkAction, PrComment, PrInvestigationStatus,
    PrInvestigationTurn, PrReview, PrSortMode, ReplyDraftProvenance, ReplyDraftRequest,
    ReplyGenerationMetadata, ReplyKind, ReplyTarget, TriageState, combined_fix_prompt,
    estimate_tokens, reply_effective_agent_drafted,
};
pub(crate) use fetch::PrFetchFailure;
pub use fetch::{fetch_and_normalize, strip_bot_boilerplate};
pub use investigation::InvestigationOutcome;
pub(crate) use investigation::investigation_findings_for_prompt;
pub(crate) use investigation::{INVESTIGATION_CONTEXT_MAX_LEN, investigation_prompt_from_meta};

#[cfg(test)]
pub use fetch::normalize;

pub(crate) mod runtime;
