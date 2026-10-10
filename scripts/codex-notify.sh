#!/usr/bin/env bash
set -uo pipefail

# Codex notify hook:
# - clears Codex "thinking" state for this AIW session
# - persists the last submitted prompt for the latest-prompt dialog
# - emits an input-request event so AIW can notify the user
# - reports the turn as completed, for the dashboard's attention layer
#
# Codex passes a JSON payload as argv[1]. We also support stdin to be robust
# across CLI versions. The payload is forwarded untouched: `aiw notify` does the
# parsing, including recovering the prompt from Codex's message list, so this
# script needs no `jq`.
#
# AIW never edits ~/.codex/config.toml. Each launch passes
# `-c notify=[<this hook>, <the user's own notify argv>...]`, and Codex appends
# the JSON payload as the final argument. So: the last argument is the
# payload, and anything before it is the user's own notify command, which is
# replayed here (in the background, with the same payload) before AIW's work.

INPUT=""
if [ "$#" -gt 0 ]; then
    INPUT="${!#}"
fi
if [ "$#" -gt 1 ]; then
    USER_NOTIFY=("${@:1:$#-1}")
    ("${USER_NOTIFY[@]}" "$INPUT" >/dev/null 2>&1 &)
fi

if [ "${AIW_ACTIVE:-}" != "1" ]; then
    exit 0
fi

if [ -z "$INPUT" ] && ! [ -t 0 ]; then
    INPUT="$(cat || true)"
fi

# $AIW_BIN names the aiw that started this session. A rebuilt or deleted
# build leaves it pointing at nothing, so fall back to `aiw` on PATH.
[ -x "${AIW_BIN:-}" ] || AIW_BIN=aiw
AIW_CMD="$AIW_BIN"

send() {
    printf '%s' "$INPUT" | "$AIW_CMD" notify --source codex-notify "$@" 2>/dev/null || true
}

send --type thinking-stop
send --type prompt-submit --derive-prompt --require prompt \
    --fallback-write-field prompt latest-prompt.txt
send --type input-request \
    --field notification_type=input-request \
    --field "message=Codex finished and is waiting for input"
# Codex's hook fires once, at turn end, and carries nothing that separates
# "I'm asking you something" from "I'm done". Reporting "completed" is the
# strongest claim the signal supports; AIW narrows anything it cannot justify.
send --type attention --event-kind completed

# Clear the thinking sentinel unconditionally, matching the pre-IPC behaviour:
# the dashboard prefers its IPC state, and a stale sentinel would otherwise sit
# there until its 2-second freshness window lapsed.
if [ -n "${AIW_SESSION:-}" ]; then
    # The marker lives under AIW's state dir (Linux: $XDG_STATE_HOME or
    # ~/.local/state; macOS: /tmp); the old /tmp/aiw-thinking is cleared too.
    if [ "$(uname -s 2>/dev/null)" = Darwin ]; then
        aiw_state_dir="/tmp/ai_workstation"
    else
        aiw_state_dir="${XDG_STATE_HOME:-$HOME/.local/state}/ai_workstation"
    fi
    rm -f "$aiw_state_dir/thinking/${AIW_SESSION}" "/tmp/aiw-thinking/${AIW_SESSION}"
fi
