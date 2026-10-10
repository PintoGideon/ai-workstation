#!/bin/bash
# AIW status line for Claude Code (passed at launch with `--settings`).
#
# Claude pipes its status JSON (model, context window, cost, rate limits) to
# this command after every assistant message. `aiw status-line` sends the
# numbers to the dashboard's socket (fire-and-forget) and prints one short
# line for Claude's own footer — or runs the status line you configured in
# ~/.claude/settings.json and prints that instead, so your footer does not
# change. No JSON is parsed here. Nothing here reaches the network.
#
# Outside AIW (no AIW_ACTIVE) it prints nothing: Claude shows no line.

if [ "${AIW_ACTIVE:-}" != "1" ]; then
    exit 0
fi

# $AIW_BIN names the aiw that started this session. A rebuilt or deleted
# build leaves it pointing at nothing, so fall back to `aiw` on PATH.
[ -x "${AIW_BIN:-}" ] || AIW_BIN=aiw
"$AIW_BIN" status-line 2>/dev/null
exit 0
