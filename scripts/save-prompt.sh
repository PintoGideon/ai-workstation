#!/bin/bash
# Claude Code UserPromptSubmit hook: report the submitted prompt.
#
# `--require prompt` reproduces the old "exit early when there is no prompt"
# guard, and the fallback writes `.claude/latest-prompt.txt` when the dashboard
# cannot be reached.
#
# Never fails the agent's turn — see notify.sh.

if [ "${AIW_ACTIVE:-}" != "1" ]; then
    exit 0
fi

# $AIW_BIN names the aiw that started this session. A rebuilt or deleted
# build leaves it pointing at nothing, so fall back to `aiw` on PATH.
[ -x "${AIW_BIN:-}" ] || AIW_BIN=aiw
"$AIW_BIN" notify \
    --type prompt-submit \
    --require prompt \
    --fallback-write-field prompt latest-prompt.txt >/dev/null 2>&1
exit 0
