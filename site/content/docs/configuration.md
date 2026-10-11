+++
title = "Configuration"
description = "Global and per-project config, resource guards, and theming."
weight = 90
+++

AIW creates its global configuration at `~/.config/ai_workstation/config.json`. A
project can add `aiw.json` at its repository root; project settings override
matching global settings.

From an embedded session, press `Ctrl+Space`, then `c` to open the
configuration wizard. It covers the common customizations, including:

- feature presets;
- custom session commands;
- lifecycle hooks;
- workspace-specific agent restrictions; and
- key remapping.

See [Project Config Reference](@/docs/project-config.md) for every key
`aiw.json` can hold and how project settings merge with global ones.

Use `T` on the dashboard to choose an AIW theme. If your terminal does not
have a Nerd Font, enable ASCII fallbacks:

```json
{
  "nerd_font": false
}
```

AIW stores its projects, features, saved prompts, and other application
state in `~/.config/ai_workstation/aiw.db`.

## Resource guards

These settings govern AIW's agent-count and memory guards:

```json
{
  "max_concurrent_agents": 4,
  "low_memory_warn_mb": 1536,
  "kill_editor_on_stop": true,
  "dormant_idle_minutes": 60,
  "dormant_last_accessed_hours": 4
}
```

`0` disables either warning. A feature is dormant only when both of its
thresholds are past.

This setting governs how long a needs-attention state stays on the
dashboard:

```json
{
  "waiting_stale_minutes": 30
}
```

A question nobody has answered for this long stops being news, so AIW drops
it back to plain idle. `0` keeps states up until the agent produces output
again.

## Built-in customization skills

AIW injects a small set of skills into Claude Code and Codex
feature workspaces so the agent can safely customize the project-local AIW
config for you: `aiw:configure` explains the current setup, `aiw:add-session`
adds a dev server or other persistent command, `aiw:add-hook` automates
feature lifecycle events, `aiw:add-preset` creates a reusable feature setup,
and `aiw:add-prompt` adds a shared prompt template. You can invoke a skill by
name or simply describe the customization you want.
