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
// Final Review diff viewer (app/review.rs, handlers/diff_review.rs) — Claude only
// ---------------------------------------------------------------------------

/// `review.walkthrough` — on-demand plain-language walkthrough of a file's
/// diff during Final Review.
///
/// Placeholders: `{{file_path}}`, `{{patch}}`.
pub const REVIEW_WALKTHROUGH: &str = r#"You are helping a reviewer understand a code change during final review. Concisely explain what this diff does and why it likely matters. Answer in short markdown: a sentence or two of summary, then a few bullet points for the notable changes. Do not restate the diff line by line.

File: {{file_path}}

```diff
{{patch}}
```"#;

/// `review.co_review` — AI co-reviewer first pass over a single file.
/// Response contract: one `<line>|<comment>` per line.
///
/// Placeholders: `{{file_path}}`, `{{annotated_body}}`.
pub const REVIEW_CO_REVIEW: &str = r#"You are an AI co-reviewer doing a first pass on a code change so a human reviewer can adjudicate your findings. Review the diff below for bugs, correctness issues, missing edge cases, and clear quality problems. Be selective — only flag things worth a human's attention; skip nits and style unless they matter.

Each line is prefixed with its line number in the new file; `+` marks an added line, `-` a removed line (removed lines have no number).

Output ONLY your findings, one per line, each formatted EXACTLY as `<line>|<comment>` where `<line>` is the new-file line number the comment is about (pick an added or context line, never a removed `-` line). Keep each comment to one or two sentences. If you find nothing worth raising, output nothing at all.

File: {{file_path}}

```
{{annotated_body}}
```"#;

/// `review.changeset_overview` — whole-changeset triage overview + risk
/// markers.
///
/// Placeholders: `{{files_block}}`.
pub const REVIEW_CHANGESET_OVERVIEW: &str = r#"You are helping a reviewer triage a full changeset before a final review. Given the per-file diffs below, write a short markdown overview: a couple of sentences on what the change does overall, then a bulleted list of the areas it touches, then a short "Risk factors" list flagging anything that deserves extra attention (large surface area, files with no obvious test coverage, cross-cutting or structural changes, anything that looks unusually risky). Be concise — this is a triage aid, not a full review.

{{files_block}}"#;

/// `review.diff_explain` — config-wizard / hook diff-review explanation.
///
/// Placeholders: `{{file_path}}`, `{{old_snippet}}`, `{{new_snippet}}`.
pub const REVIEW_DIFF_EXPLAIN: &str = r#"Explain these code changes concisely. What is being changed and why?

File: {{file_path}}

Old:
```
{{old_snippet}}
```

New:
```
{{new_snippet}}
```"#;

// ---------------------------------------------------------------------------
// Session summary (summary.rs)
// ---------------------------------------------------------------------------

/// `session.summary` — one-line summary of a session's tmux pane content.
///
/// Placeholders: `{{harness_name}}`, `{{max_chars}}`, `{{recent_lines}}`.
pub const SESSION_SUMMARY: &str = r#"Summarize this {{harness_name}} session in one line (max {{max_chars}} chars). Focus on what was done or what's blocking. Be concise and specific. Example: 'Refactored auth module, waiting on test fix'

Session output:
{{recent_lines}}"#;

// ---------------------------------------------------------------------------
// Batched review of an oversized diff (review_batch.rs)
// ---------------------------------------------------------------------------

/// `review.batch` — review one bounded slice of a diff that was too large to
/// send in a single prompt. The slice holds one or more whole files; other
/// parts of the change are reviewed in their own prompts and the findings are
/// combined afterwards by `review.synthesis`.
///
/// Placeholders: `{{skill_directive}}`, `{{recurring_findings}}`,
/// `{{file_list}}`, `{{annotated_diff}}`, `{{finding_heading_prefix}}`.
pub const REVIEW_BATCH: &str = r#"{{skill_directive}}You are reviewing part of a larger pull request. The diff is too big to review at once, so it has been split; you are seeing one slice of it now. Review only the lines shown for correctness bugs and clear quality problems. Do NOT flag something as missing just because it is not in this slice — imports, helpers, types, tests, and callers may live in another slice you cannot see. Check especially for issues matching the team's known recurring findings below, if any. Skip praise and style nitpicks.

Files in this slice: {{file_list}}

{{recurring_findings}}Diff:

{{annotated_diff}}

---

Output ONLY findings, no summary and no prose outside them. If you find nothing worth raising in this slice, output nothing at all.

{{finding_heading_prefix}}<path>|<side>|<line>
<finding text, 1-3 sentences>

{{finding_heading_prefix}}General
<a finding with no single file:line anchor>

`<side>` must be `RIGHT` for a current-file line or `LEFT` for a removed base-file line. Copy the path, side, and one-based line number exactly from that row's bracketed coordinate label; never count patch rows or infer a line number from a hunk offset.
"#;

/// `review.hunk_split` — review one hunk group of a single file whose own diff
/// was still too large after file-level batching. The rest of the file is not
/// shown.
///
/// Placeholders: `{{file_path}}`, `{{hunk_label}}`, `{{annotated_diff}}`,
/// `{{finding_heading_prefix}}`.
pub const REVIEW_HUNK_SPLIT: &str = r#"You are reviewing {{hunk_label}} of `{{file_path}}`. This file's change was too large to review whole, so you are seeing only these hunks — the rest of the file, its imports, and its other hunks are not shown. Only raise an issue you are confident about from the lines here; when a concern depends on code you cannot see (a type definition, an import, an earlier guard), say so instead of asserting a bug. Skip praise and style nitpicks.

Diff:

{{annotated_diff}}

---

Output ONLY findings, no summary. If you find nothing worth raising in these hunks, output nothing at all.

{{finding_heading_prefix}}<path>|<side>|<line>
<finding text, 1-3 sentences>

`<side>` must be `RIGHT` for a current-file line or `LEFT` for a removed base-file line. Copy the path, side, and one-based line number exactly from that row's bracketed coordinate label.
"#;

/// `review.synthesis` — combine the per-slice findings from a batched review
/// into one coherent review with a summary. `{{uncovered_note}}` is empty
/// unless some slice could not be reviewed even after splitting.
///
/// Placeholders: `{{batch_findings}}`, `{{uncovered_note}}`,
/// `{{finding_heading_prefix}}`.
pub const REVIEW_SYNTHESIS: &str = r#"A large pull request was reviewed in slices because its diff was too big for one pass. Below are the raw findings from every slice, in order. Combine them into one review: merge findings that describe the same issue, drop any that a later slice's context clearly resolves, and keep every distinct issue with its original file, side, and line intact. Do not invent findings that are not supported by the slice output below.

{{uncovered_note}}Slice findings:

{{batch_findings}}

---

Output ONLY the summary and findings in this exact format (no prose outside it). Always include the summary, even when there are no findings; one to three sentences on the main themes or risk:

## Summary
<overall review summary>

{{finding_heading_prefix}}<path>|<side>|<line>
<finding text, 1-3 sentences>

{{finding_heading_prefix}}General
<a finding with no single file:line anchor>
"#;

/// `review.findings_summary` — condense one slice's findings when the
/// `review.synthesis` prompt itself overflows and its inputs must be shrunk
/// before the final merge.
///
/// Placeholders: `{{batch_label}}`, `{{findings}}`, `{{max_chars}}`.
pub const REVIEW_FINDINGS_SUMMARY: &str = r#"Condense the code-review findings below (from {{batch_label}}) to at most {{max_chars}} characters. Keep every distinct issue: preserve each finding's file path, side, one-based line number, and a one-sentence description. Drop only elaboration, rationale, and repetition — never a whole finding. Keep the same `<path>|<side>|<line>` heading shape the input uses.

Findings:

{{findings}}"#;
