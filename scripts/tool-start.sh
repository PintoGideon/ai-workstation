#!/bin/bash
# Claude Code PreToolUse hook: mark active tool execution.
#
# `aiw notify` lifts the nested `tool_input.{taskId,subject,...}` fields into
# the flat `task_*` shape the dashboard reads.
#
# Never fails the agent's turn — see notify.sh.

if [ "${AIW_ACTIVE:-}" != "1" ]; then
    exit 0
fi

# $AIW_BIN names the aiw that started this session. A rebuilt or deleted
# build leaves it pointing at nothing, so fall back to `aiw` on PATH.
[ -x "${AIW_BIN:-}" ] || AIW_BIN=aiw
"$AIW_BIN" notify \
    --type tool-start \
    --fallback-touch /tmp/aiw-tool >/dev/null 2>&1
exit 0
