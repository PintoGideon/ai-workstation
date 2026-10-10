#!/bin/bash
# AIW subagent hook (Claude `SubagentStart` / `SubagentStop`, Codex
# `SubagentStart` / `SubagentStop`, Cursor `subagentStart` / `subagentStop`).
#
# argv[1] is `start` or `stop`, argv[2] the harness. The hook JSON is passed
# straight through on stdin; `aiw subagent` sends it to the dashboard
# (fire-and-forget), which keeps a count per session for the badge. Prints
# nothing (Cursor's `subagentStart` answer is added by the hook entry
# itself); never fails the agent's turn. No JSON is parsed here.

if [ "${AIW_ACTIVE:-}" != "1" ]; then
    exit 0
fi

# $AIW_BIN names the aiw that started this session. A rebuilt or deleted
# build leaves it pointing at nothing, so fall back to `aiw` on PATH.
[ -x "${AIW_BIN:-}" ] || AIW_BIN=aiw
"$AIW_BIN" subagent --event "${1:-start}" --harness "${2:-claude}" >/dev/null 2>&1
exit 0
