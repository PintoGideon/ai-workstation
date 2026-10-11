+++
title = "Project Config Reference"
description = "What a project's aiw.json can declare, and how it merges with global config."
weight = 92
+++

A project's `aiw.json` (repo root — see [Configuration](@/docs/configuration.md))
is meant to be committed: it's how a team shares presets, sessions, and hooks
across everyone working in the repo, layered
on top of each person's own global `~/.config/ai_workstation/config.json`. This page
is the field reference for what it can hold.

| Key | Holds | Edit via | Merge with global |
| --- | --- | --- | --- |
| `custom_sessions` | Dev servers or other persistent commands offered in the `s` session picker | Config wizard | Project appends; a name collision lets the project entry win |
| `feature_presets` | Named create-feature templates (agent, mode, branch prefix, review, Chrome) | Config wizard | Same append/override-by-name rule |
| `lifecycle_hooks` | `on_start` / `on_stop` / `on_worktree_created` scripts, optionally prompting the user with options first | Config wizard | Each of the three hooks independently overrides its global counterpart when set |
| `keybindings` | Remap a dashboard action to a different key | Config wizard | Project overrides global per action |
| `allowed_agents` | Restrict which harnesses this project offers | Config wizard | Project replaces the whole list when set |
| `prompt_templates` | Reusable [prompt-library](@/docs/prompts-and-todos.md#reuse-prompts) templates | Prompt library (`L`) export | Merged by name: project wins on collision |

The config wizard (`Ctrl+Space`, then `c`) covers everything marked
"Config wizard" above — see
[Configuration](@/docs/configuration.md#built-in-customization-skills) for
how to open it and for the `aiw:configure` family of skills that can edit
these on your behalf. Everything marked "Hand-edit" has no wizard step yet;
edit `aiw.json` directly.

A field with no entry in the project's `aiw.json` simply inherits the
global value — an empty project config never overrides anything.
