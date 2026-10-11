//! Built-in default template text for every registered headless prompt.
//!
//! Each constant is the *editable surface* of one prompt ID: the instruction
//! prose that today lives inline at the call site, rewritten with visible
//! `{{placeholder}}` tokens where dynamic context is spliced in at run time
//! (see [`crate::prompts::resolve_prompt`]). The call sites in `app/review/`,
//! `handlers/diff_review.rs` and `summary.rs` build the context map; these
//! strings are the templates.
//!
//! Placeholders are documented per prompt in
//! `docs/backlog/editable-prompts-call-site-inventory.md`. There is no
//! validation: an override that drops a required token, or adds an unknown
//! one, is rendered verbatim (a deliberate product decision).

// ---------------------------------------------------------------------------
// Session summary (summary.rs)
// ---------------------------------------------------------------------------

/// `session.summary` — one-line summary of a session's tmux pane content.
///
/// Placeholders: `{{harness_name}}`, `{{max_chars}}`, `{{recent_lines}}`.
pub const SESSION_SUMMARY: &str = r#"Summarize this {{harness_name}} session in one line (max {{max_chars}} chars). Focus on what was done or what's blocking. Be concise and specific. Example: 'Refactored auth module, waiting on test fix'

Session output:
{{recent_lines}}"#;
