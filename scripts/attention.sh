#!/bin/bash
# AIW attention hook.
#
# Reports *why* an agent session stopped, so the dashboard can distinguish a
# session asking a question from one that finished work awaiting review.
#
# argv[1] is the event kind: "question", "completed", or "waiting". Anything
# else (or nothing) is treated as "waiting" by AIW, which is the correct
# degradation for a harness whose signal we can't interpret.
#
# The special kind "notification" means "classify this from the payload": it is
# used for Claude's Notification event, which covers far more than a blocked
# agent. The classification happens in AIW, not here, so it is unit-tested and
# needs no JSON parsing in shell.
#
# This is deliberately separate from notify.sh: it only feeds the in-memory
# attention layer and never queues a pending input, so wiring a new harness
# event here cannot disturb the existing notification flow.
#
# Never fails the agent's turn — see notify.sh.

if [ "${AIW_ACTIVE:-}" != "1" ]; then
    exit 0
fi

# $AIW_BIN names the aiw that started this session. A rebuilt or deleted
# build leaves it pointing at nothing, so fall back to `aiw` on PATH.
[ -x "${AIW_BIN:-}" ] || AIW_BIN=aiw
"$AIW_BIN" notify \
    --type attention \
    --source attention-hook \
    --event-kind "${1:-waiting}" >/dev/null 2>&1
exit 0
