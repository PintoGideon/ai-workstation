+++
title = "Core Concepts"
description = "Projects, features, sessions, and permission modes."
weight = 30
+++

## Projects, features, and sessions

- A **project** points to a directory that you want to work in.
- A **feature** represents one task or branch within that project.
- A **session** is an agent, terminal, editor, TODO list, or custom command
  attached to a feature.

For git projects, the first feature can use the repository directory
directly. Additional features use worktrees under `.worktrees/`, allowing
their agents to work concurrently without changing one another's files.
Press `F` to fork a feature or `B` to create several features at once.

## Permission modes

AIW asks you to choose how much autonomy an agent receives for each feature:

| Mode | Behavior |
| --- | --- |
| **Vibeless** | Shows supported file edits for approval before they are applied. Available for Claude Code. |
| **Vibe** | Allows edits without AIW's per-edit review gate while retaining the agent's normal permission controls. |
| **SuperVibe** | Skips agent permission prompts. AIW shows a warning before enabling it. |

Codex and Pi do not support AIW's Vibeless edit-review hooks. Pi also does
not receive AIW permission-mode flags. See
[Choosing a Harness](@/docs/harnesses.md) for the full breakdown of what
else varies by harness.
