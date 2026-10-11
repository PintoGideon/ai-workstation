+++
title = "Choosing a Harness"
description = "What Claude Code, Codex, Gemini CLI, Cursor agent and Pi each support in AIW."
weight = 32
+++

AIW treats Claude Code, Codex, Gemini CLI, Cursor agent and Pi as first-class
agents, but they don't all expose the same hooks and CLI surface, so a few
AIW features vary by harness. This page collects those differences in one
place.

| | Claude Code | Codex | Gemini CLI | Cursor agent | Pi |
| --- | --- | --- | --- | --- | --- |
| Binary | `claude` | `codex` | `gemini` | `agent` | `pi` |
| [Permission modes](@/docs/core-concepts.md#permission-modes) | Vibeless, Vibe, SuperVibe | Vibe, SuperVibe — Vibeless is refused | all three, via `--approval-mode` | Vibe, SuperVibe (`--force`) | selectable, no effect — Pi receives no mode flags |
| [Needs-you and finished badges](@/docs/attention-and-limits.md#see-which-agents-need-you) | question, completed, waiting | waiting only — one hook fires at turn end | all three | working and completed; questions through the screen | none — Pi has no hooks |
| Answer permissions from the dashboard | yes | yes | no — its hooks only observe | yes, shell and MCP calls | no |
| Vibeless edit approval | yes | refused | no | no | no |
| [Context and cost on the row](@/docs/attention-and-limits.md#track-context-and-usage) | yes, from its status line | yes, from its rollout file | tokens only | no | no |
| Queued follow-ups | yes | yes | yes | yes | no |
| Subagent count | yes | yes | no | yes | no |
| Launch context | appended system prompt | developer instructions | session-start hook | session-start hook | no |
| Resume a saved session (`S`) | yes | yes | `--resume <id>` at launch | `--resume <id>` at launch | no |
| Where AIW's hooks go | a per-session settings file passed with `--settings` | `.codex/hooks.json` and a notify hook per launch | `.gemini/settings.json` | `.cursor/hooks.json` | nowhere |

A few notes worth expanding on:

- **Vibeless** relies on AIW's per-edit approval hook. Claude Code supports
  it; Codex does not, so AIW refuses Vibeless for a Codex feature rather than
  silently downgrading it. Pi receives no mode flags at all, so any mode can
  be selected but none changes Pi's behaviour.
- **Badges** depend entirely on what each harness's hooks report. Codex's
  single end-of-turn hook can't distinguish a question from a finished turn,
  so every stopped Codex session shows as waiting. Pi has no hook mechanism,
  so its sessions carry no state.
- **One-time setup.** Codex asks you to trust AIW's hooks once per worktree
  with its own hooks command. Cursor runs project hooks only in a trusted
  workspace. Gemini cannot grant permissions from a hook, so add safe
  commands to its own allowed-tools setting to stop being asked.
- AIW never edits a harness's global configuration. Claude's hooks travel in
  a `--settings` file; Codex gets its notify hook per launch rather than in
  `~/.codex/config.toml`.
