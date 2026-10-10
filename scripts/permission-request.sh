#!/bin/bash
# AIW permission hook (Claude `PermissionRequest`, Codex `PermissionRequest`,
# Cursor `beforeShellExecution` / `beforeMCPExecution`).
#
# argv[1] names the harness (claude, codex, cursor); argv[2], optional, the
# Cursor event name. The hook's JSON is passed straight through on stdin;
# `aiw permission-request` parses it, sends the request to the dashboard and
# waits (up to `permission_wait_seconds`) for the user's allow / deny there.
# It prints the harness's decision JSON, or nothing when the dashboard did not
# answer (or is not running), so the harness shows its own prompt. No JSON is
# parsed here.
#
# Cursor treats an empty answer from a permission hook as invalid, which
# would block the call: outside AIW it is told to ask as usual.

if [ "${AIW_ACTIVE:-}" != "1" ]; then
    if [ "${1:-}" = "cursor" ]; then
        printf '{"continue":true,"permission":"ask"}\n'
    fi
    exit 0
fi

# $AIW_BIN names the aiw that started this session. A rebuilt or deleted
# build leaves it pointing at nothing, so fall back to `aiw` on PATH.
[ -x "${AIW_BIN:-}" ] || AIW_BIN=aiw
"$AIW_BIN" permission-request --harness "${1:-claude}" ${2:+--event "$2"} 2>/dev/null
exit 0
