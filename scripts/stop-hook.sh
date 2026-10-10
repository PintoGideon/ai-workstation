#!/bin/bash
# AIW turn-end hook (Claude `Stop`, Codex `Stop`, Gemini `AfterAgent`, Cursor
# `stop`): asks the dashboard whether the user queued a follow-up for this
# session and, if so, prints the harness's continuation JSON so the text
# becomes the agent's next turn. Nothing is typed into the agent on its own:
# only a follow-up the user queued is ever delivered.
#
# argv[1] names the harness (claude, codex, gemini, cursor). The hook JSON is
# passed straight through on stdin; `aiw stop-hook` parses it and waits at
# most two seconds for the dashboard. No JSON is parsed here.
#
# Codex requires JSON from a Stop hook that exits 0, so outside AIW it is
# given an empty object; the others accept silence.

if [ "${AIW_ACTIVE:-}" != "1" ]; then
    if [ "${1:-}" = "codex" ]; then
        printf '{}\n'
    fi
    exit 0
fi

# $AIW_BIN names the aiw that started this session. A rebuilt or deleted
# build leaves it pointing at nothing, so fall back to `aiw` on PATH.
[ -x "${AIW_BIN:-}" ] || AIW_BIN=aiw
"$AIW_BIN" stop-hook --harness "${1:-claude}" 2>/dev/null
exit 0
