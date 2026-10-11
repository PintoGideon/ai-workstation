+++
title = "Choosing a Harness"
description = "What Claude Code, Codex, and Pi each support in AIW."
weight = 32
+++

AIW treats Claude Code, Codex, and Pi as first-class agents, but
they don't all expose the same hooks and CLI surface, so a few AIW features
vary by harness. This page collects those differences in one place; each
row links back to the page that documents the feature in full.

| Capability | Claude Code | Codex | Pi |
| --- | --- | --- | --- |
| [Permission modes](@/docs/core-concepts.md#permission-modes) | Vibeless, Vibe, SuperVibe | Vibe, SuperVibe — Vibeless is blocked | Selectable, but has no effect — Pi does not receive AIW's permission-mode flags |
| [Attention-state fidelity](@/docs/attention-and-limits.md#see-which-agents-need-you) | Question, Completed, and Waiting | Waiting only — one lifecycle hook fires at turn-end with no way to tell a question from a completion | None — Pi has no lifecycle-hook mechanism |
| [Context-window tracking](@/docs/attention-and-limits.md#track-context-and-usage-in-the-sidebar) | Yes | Yes | Yes |
| [Usage/quota sidebar](@/docs/attention-and-limits.md#track-context-and-usage-in-the-sidebar) (5h / 7-day windows) | Yes | Yes | No usage API is currently known for this harness |
| Saved-session resume | Yes | Yes | Not available |
| [Prompt overrides](@/docs/custom-prompts.md) | Yes | Yes | Yes |

A few notes worth expanding on:

- **Vibeless** relies on AIW's per-edit review hooks. Claude Code
  supports them; Codex does not, so AIW blocks selecting Vibeless
  for a Codex feature outright rather than silently downgrading it. Pi
  supports none of AIW's permission-mode integration, so any mode can be
  selected but none changes Pi's own behavior.
- **Attention-state fidelity** depends entirely on what each harness's
  lifecycle hooks report. Codex's single end-of-turn hook can't distinguish
  a question from a finished turn, so every stopped Codex session shows as
  Waiting. Pi has no hook mechanism at all, so its sessions carry no
  attention state.
