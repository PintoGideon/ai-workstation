---
name: aiw:add-preset
description: >
  Add a feature preset to this AIW workspace. Use when the user
  wants a named template that pre-configures agent harness, mode,
  branch prefix and Chrome settings for new
  features — so they can create a feature with a single selection
  instead of filling out every field each time.
allowed-tools: Bash(cat *) Bash(mkdir *) Edit(aiw.json) Bash(test *) Bash(rm .aiw/config.json)
argument-hint: "[preset name]"
---

## Current config

!`cat aiw.json 2>/dev/null || cat .aiw/config.json 2>/dev/null || echo "{}"`

## Task

Add or update an entry in the `feature_presets` array in
`aiw.json`. Create the file if it doesn't exist.

## FeaturePreset schema

```json
{
  "name": "Quick Fix",
  "branch_prefix": "fix/",
  "mode": "normal",
  "agent": "claude",
  "enable_chrome": false
}
```

| Field | Required | Valid values | Notes |
|---|---|---|---|
| `name` | yes | any string | Label shown in the preset picker |
| `branch_prefix` | no | any string | Prepended to the branch name the user types |
| `mode` | no | `"normal"`, `"vibeless"` | `vibeless` = diff review on every edit |
| `agent` | no | `"claude"`, `"codex"`, `"pi"`, `"gemini"`, `"cursor"` | Agent harness to use |
| `enable_chrome` | no | `true`, `false` | Allow agent to control a browser |

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

Project presets appear first in the picker; global presets are
appended unless the same name exists in the project config.
If the user hasn't specified scope, default to project scope.

## allowed_agents interaction

If `allowed_agents` is set in config, only presets whose `agent`
is in that list will be shown. Make sure the preset's agent is
allowed, or omit `allowed_agents` to allow all.

## Steps

1. Read the current config (shown above — it may have come
   from the legacy `.aiw/config.json`)
2. Add the new preset to `feature_presets` (or create the array)
3. Write the updated JSON back to `aiw.json` — preserve existing
   presets and other config — then delete `.aiw/config.json` if the
   config came from there
4. Tell the user the preset will appear when creating a new
   feature in AIW (press `N` on the project)
