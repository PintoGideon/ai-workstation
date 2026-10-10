#!/bin/bash
# Claude Code hook: clear the session's thinking state.
#
# Never fails the agent's turn — see notify.sh.

if [ "${AIW_ACTIVE:-}" != "1" ]; then
    exit 0
fi

# $AIW_BIN names the aiw that started this session. A rebuilt or deleted
# build leaves it pointing at nothing, so fall back to `aiw` on PATH.
[ -x "${AIW_BIN:-}" ] || AIW_BIN=aiw
"$AIW_BIN" notify \
    --type thinking-stop \
    --fallback-remove /tmp/aiw-thinking >/dev/null 2>&1
exit 0
