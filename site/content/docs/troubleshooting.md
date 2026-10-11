+++
title = "Troubleshooting and Automation"
description = "aiw doctor, the automation CLI, the debug log, and common fixes."
weight = 100
+++

## `aiw doctor`

```bash
aiw doctor
aiw doctor --json
```

A read-only report on what AIW is putting on the machine: agents running
against your `max_concurrent_agents` limit, editor windows open alongside
them, memory and swap, `aiw-*` tmux sessions and worktrees with no matching
feature, editors still running for features you've stopped, and any project
still keeping its config at the legacy `.aiw/config.json` path. `--json`
emits the same findings for scripting. It changes nothing on the machine and
always exits `0`.

## Automation CLI

A running AIW instance accepts structured commands for creating projects and
features without scripting the TUI:

```bash
aiw automation create-project --file request.json
aiw automation create-feature --file request.json
aiw automation create-batch-features --file request.json
aiw automation queue --session <id> "follow-up text"
```

The create commands send a request to the running dashboard over the same local
IPC system AIW's own lifecycle hooks use; AIW applies it inside the
dashboard process and prints a JSON response. Every command accepts
`--dry-run` to validate and preview without changing AIW state.

See the
automation guide (`docs/automation/README.md` in the repository)
for the full field reference, request templates, and examples.

## Debug log

Press `D` on the dashboard to view AIW's debug log from inside the TUI. The
same log is written to `~/.local/state/ai_workstation/debug.log`.

## Common issues

- **Updating.** AIW has no built-in upgrader; pull the fork and rebuild (see
  [Installation](@/docs/installation.md#upgrade)).
- **Icons render incorrectly.** Your terminal likely lacks a
  [Nerd Font](https://www.nerdfonts.com/); see
  [Configuration](@/docs/configuration.md) for the ASCII-fallback setting.
- **An agent doesn't appear during feature creation.** Press `A` and confirm
  that CLI is installed and enabled.
