#!/bin/bash
# Claude Code Stop hook: tell the dashboard the turn ended.
#
# Deliberately sets no `--type`: this forwards Claude's own payload, and the
# dashboard's generic path keys off `type` being absent.
#
# Never fails the agent's turn. A hook that exits non-zero aborts the tool call
# that triggered it, so delivery problems — AIW not running, or an `aiw` binary
# older than these flags — are swallowed and the hook exits 0 regardless.

if [ "${AIW_ACTIVE:-}" != "1" ]; then
    exit 0
fi

# $AIW_BIN names the aiw that started this session. A rebuilt or deleted
# build leaves it pointing at nothing, so fall back to `aiw` on PATH.
[ -x "${AIW_BIN:-}" ] || AIW_BIN=aiw
"$AIW_BIN" notify >/dev/null 2>&1
exit 0
