+++
title = "Core Concepts"
description = "Projects, features, worktrees, sessions, and permission modes."
weight = 30
+++

## Projects, features, and sessions

- A **project** points to a directory that you want to work in.
- A **feature** represents one task or branch within that project. It owns a
  tmux session on AIW's private tmux server.
- A **session** is an agent, terminal, editor, TODO list, or custom command
  attached to a feature.

For git projects, the first feature can use the repository directory
directly. Other features get a **worktree**: a separate checkout on its own
branch, created outside the repository under AIW's data directory, so parallel
agents never touch each other's files. Files that git ignores but a fresh
checkout needs, such as `.env`, are copied in; a repository can list them in
a `.worktreeinclude` file. Press `F` to fork a feature or `B` to create
several features at once.

## The cockpit

`aiw` builds a small tmux layout for itself: the sidebar on the left, the
selected agent's real tmux window on the right. tmux draws the agent, AIW
draws the sidebar. `Ctrl+Q` flips the keyboard between the two. Closing the
terminal stops nothing; the next `aiw` attaches again. See
[Keybindings](@/docs/keybindings.md).

## Permission modes

AIW asks you to choose how much autonomy an agent receives for each feature,
and maps each level to the harness's own flags:

| Mode | Behavior | Claude Code | Codex | Gemini CLI | Cursor agent |
| --- | --- | --- | --- | --- | --- |
| **Vibeless** | Every file edit is held until you approve it on the dashboard | `--permission-mode default` plus AIW's edit-approval hook | refused | `--approval-mode default` | default |
| **Vibe** | Edits are accepted; other permissions still ask | `--permission-mode acceptEdits` | `--sandbox workspace-write --ask-for-approval on-request` | `--approval-mode auto_edit` | default |
| **SuperVibe** | No permission prompts at all; AIW warns before enabling it | `--dangerously-skip-permissions` | `--sandbox danger-full-access --ask-for-approval never` | `--approval-mode yolo` | `--force` |

Claude sessions can also get the `--chrome` flag for browser automation —
the Chrome row in the new-feature dialog. Pi receives no mode flags, so the
mode you pick for a Pi feature does not change Pi's behaviour. See
[Choosing a Harness](@/docs/harnesses.md) for everything else that varies
by harness.
