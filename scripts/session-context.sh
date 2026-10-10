#!/bin/bash
# AIW session-start hook (Gemini `SessionStart`, Cursor `sessionStart`): hands
# the agent its launch context — project, feature, branch, worktree, AIW
# session id — as the hook's additional context, the way Claude gets it with
# `--append-system-prompt` and Codex with `developer_instructions`.
#
# argv[1] names the harness. `aiw session-context` rebuilds the paragraph
# from AIW's database (read-only) for the session named by the environment
# and prints the harness's JSON; it prints nothing when `agent_context_prompt`
# is off. No JSON is parsed here; nothing reaches the network.

if [ "${AIW_ACTIVE:-}" != "1" ]; then
    exit 0
fi

# $AIW_BIN names the aiw that started this session. A rebuilt or deleted
# build leaves it pointing at nothing, so fall back to `aiw` on PATH.
[ -x "${AIW_BIN:-}" ] || AIW_BIN=aiw
"$AIW_BIN" session-context --harness "${1:-gemini}" 2>/dev/null
exit 0
