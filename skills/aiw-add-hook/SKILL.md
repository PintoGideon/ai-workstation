---
name: aiw:add-hook
description: >
  Add a lifecycle hook to this AIW workspace. Use when the user
  wants to run a script automatically when a feature starts
  (on_start), stops (on_stop), or when a new git worktree is
  created for this project (on_worktree_created). Hooks can
  optionally prompt the user to choose from a list of options
  before the script runs.
allowed-tools: Bash(cat *) Bash(mkdir *) Edit(aiw.json) Bash(test *) Bash(rm .aiw/config.json)
argument-hint: "[on_start | on_stop | on_worktree_created] [script path]"
---

## Current config

!`cat aiw.json 2>/dev/null || cat .aiw/config.json 2>/dev/null || echo "{}"`

## Task

Add or update a hook in `lifecycle_hooks` in `aiw.json`.
Create the file if it doesn't exist.

## Hook events

| Event | When it fires |
|---|---|
| `on_start` | Feature session is started or resumed in AIW |
| `on_stop` | Feature session is stopped from AIW |
| `on_worktree_created` | A new git worktree is created for this project |

## Simple hook (plain script path)

```json
{
  "lifecycle_hooks": {
    "on_start": "~/scripts/setup.sh",
    "on_stop": "~/scripts/teardown.sh"
  }
}
```

The `~/` prefix is expanded by AIW. Scripts run with the feature
workdir as the working directory.

## Prompt hook (user picks an option before the script runs)

```json
{
  "lifecycle_hooks": {
    "on_worktree_created": {
      "script": "~/scripts/init-worktree.sh",
      "prompt": {
        "title": "Which environment?",
        "options": ["staging", "production"]
      }
    }
  }
}
```

The user's choice is passed to the script as the environment
variable `$AIW_HOOK_CHOICE`. Only `on_worktree_created` shows
the prompt in the AIW UI before running; `on_start` and
`on_stop` run silently.

## Config path (read the legacy file too)

Project config lives at `aiw.json` in the repo root. Older checkouts
keep it at `.aiw/config.json`, and the block above falls back to that
file when `aiw.json` is absent — so what you see is the config that is
actually in effect, not an empty `{}`.

Always **write** to `aiw.json`. If the config you read came from the
legacy path, carry every existing key over into `aiw.json` and then
delete the legacy file:

```bash
test -f aiw.json && rm .aiw/config.json
```

AIW prefers `aiw.json` whenever it exists, so leaving both behind would
let a stale `.aiw/config.json` sit there looking authoritative while
changing nothing.

## Scope

- **Project** (this repo only): `aiw.json` — edit this file
- **Global** (all projects): `~/.config/ai_workstation/config.json` under
  the `"extension"` key

Project hooks take priority over global hooks for the same event.
If the user hasn't specified scope, default to project scope.

## Steps

1. Read the current config (shown above — it may have come
   from the legacy `.aiw/config.json`)
2. Add or replace the relevant hook event in `lifecycle_hooks`
3. Write the updated JSON back to `aiw.json` — preserve other
   hooks and sessions — then delete `.aiw/config.json` if the
   config came from there
4. Tell the user which event will trigger the script and how to
   test it (e.g. stop and restart the feature from AIW)
