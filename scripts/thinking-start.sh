#!/bin/bash
# Claude Code hook: mark the session as thinking.
#
# The harness's hook JSON is passed straight through on stdin; `aiw notify`
# merges in AIW's session metadata from the environment it inherits from this
# script. No JSON is parsed here, so no `jq` is required.
#
# Never fails the agent's turn — see notify.sh.

if [ "${AIW_ACTIVE:-}" != "1" ]; then
    exit 0
fi

# $AIW_BIN names the aiw that started this session. A rebuilt or deleted
# build leaves it pointing at nothing, so fall back to `aiw` on PATH.
[ -x "${AIW_BIN:-}" ] || AIW_BIN=aiw
"$AIW_BIN" notify \
    --type thinking-start \
    --fallback-touch /tmp/aiw-thinking >/dev/null 2>&1
exit 0
