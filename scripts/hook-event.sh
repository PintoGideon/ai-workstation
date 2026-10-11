#!/bin/bash
# AIW hook-event hook (DESIGN.md §19.1): the harness events that decorate a
# session's row — Claude's `Notification` (idle nudge, quota), `Stop`
# (background tasks and /loop schedules still alive), `StopFailure`,
# `PostCompact`, `PostModelSwitch`, `TaskCreated` / `TaskCompleted`,
# `CwdChanged`, `PermissionDenied` and `SessionEnd`.
#
# argv[1] is the kind (`notification`, `stop`, `stop-failure`, …), argv[2] the
# harness. The hook JSON is passed straight through on stdin; `aiw hook-event`
# forwards the few fields the dashboard reads (fire-and-forget). Prints
# nothing; never fails the agent's turn. No JSON is parsed here.

if [ "${AIW_ACTIVE:-}" != "1" ]; then
    exit 0
fi

# $AIW_BIN names the aiw that started this session. A rebuilt or deleted
# build leaves it pointing at nothing, so fall back to `aiw` on PATH.
[ -x "${AIW_BIN:-}" ] || AIW_BIN=aiw
"$AIW_BIN" hook-event --kind "${1:-}" --harness "${2:-claude}" >/dev/null 2>&1
exit 0
