---
name: aiw:configure
description: >
  Show and explain the current AIW workspace configuration.
  Use when the user asks what AIW settings are available,
  wants to understand what is currently configured, or needs
  to know the difference between project and global config scope.
  Routes to aiw:add-session, aiw:add-hook, or aiw:add-preset for
  targeted changes.
allowed-tools: Bash(cat *) Bash(test *)
disable-model-invocation: true
---

## Current project config (aiw.json)

!`cat aiw.json 2>/dev/null || cat .aiw/config.json 2>/dev/null || echo "(none — file does not exist yet)"`

## Current global config (~/.config/ai_workstation/config.json)

!`cat ~/.config/ai_workstation/config.json 2>/dev/null | python3 -c "import sys,json; d=json.load(sys.stdin); print(json.dumps(d.get('extension',{}), indent=2))" 2>/dev/null || echo "(none)"`

## Config scope

| Scope | File | Key |
|---|---|---|
| This project only | `aiw.json` | top-level |
| All projects | `~/.config/ai_workstation/config.json` | under `"extension"` |

Project config wins over global for the same key/name.

## What you can configure

### Custom sessions — `custom_sessions`

Persistent tmux windows that appear in the AIW session picker
alongside the agent and terminal. Examples: dev server, docker
compose, test watcher, database shell.

Use `/aiw:add-session` to add one.

### Lifecycle hooks — `lifecycle_hooks`

Scripts that run automatically at key moments:

- `on_start` — when a feature is started or resumed
- `on_stop` — when a feature is stopped
- `on_worktree_created` — when a new git worktree is created
  (can prompt user to choose an option first)

Use `/aiw:add-hook` to add one.

### Feature presets — `feature_presets`

Named templates that pre-fill agent, mode, branch prefix, review,
and Chrome settings when creating a new feature.

Use `/aiw:add-preset` to add one.

### Keybindings — `keybindings`

Override default AIW key mappings per action. Project overrides
global per-key. Example: `{ "quit": "Q" }`.

### Allowed agents — `allowed_agents`

Restrict which agent harnesses are available for this project.
Values: `"claude"`, `"codex"`, `"pi"`, `"gemini"`, `"cursor"`. Omit to allow all.

Example: `{ "allowed_agents": ["claude", "codex"] }`

### View auto-refresh — `view_auto_refresh`

Opt into periodic embedded-view repairs for Claude/tmux rendering
bugs. Defaults to `false`; manual refresh and normal event-driven
updates still work when this is off.

## What would you like to do?

Based on what the user is asking for, invoke the appropriate skill:

- Adding a session, server, or background process →
  `/aiw:add-session`
- Setting up a startup, shutdown, or worktree script →
  `/aiw:add-hook`
- Creating a feature template or preset → `/aiw:add-preset`
- Anything else — explain using the schema above and edit
  `aiw.json` directly
