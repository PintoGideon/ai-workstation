#!/usr/bin/env bash
# tmux driver helpers for the AIW e2e run. Source after env.sh.
#
#   aiw_start [cols] [rows] [extra aiw args…]   start aiw in a detached tmux on $S/drive.sock
#   keys <send-keys args…>                      send keys to the driven pane
#   cap                                         capture the pane (plain text)
#   cap_ansi                                    capture with escape sequences
#   wait_for <regex> [seconds]                  poll the capture until it matches
#   wait_gone <regex> [seconds]                 poll until it no longer matches
#   snap <name>                                 save a capture under $S/captures
#   aiw_quit                                    q on the dashboard and wait for exit
#   row_glyph <text> <glyph>                    the first dashboard row matching <text> shows <glyph>
#   wait_row_glyph <text> <glyph> [seconds]     poll until it does
#   aiw_tmux <args…>                            tmux on AIW's private socket (read-only use)
#   check <name> <cond-exit-code>               record pass/fail
#   drive_kill                                  kill the driver tmux server

DRIVE_SOCK=${DRIVE_SOCK:-$S/drive.sock}
DRIVE_SESSION=${DRIVE_SESSION:-drive}
AIW_SOCK=${AIW_SOCK:-$XDG_STATE_HOME/ai_workstation/tmux.sock}
RESULTS=${RESULTS:-$S/results.tsv}

dt() { tmux -S "$DRIVE_SOCK" "$@"; }
aiw_tmux() { tmux -S "$AIW_SOCK" "$@"; }

aiw_start() {
    local cols=${1:-200} rows=${2:-60}
    shift 2 2>/dev/null || true
    dt kill-server 2>/dev/null || true
    rm -f "$DRIVE_SOCK"
    dt new-session -d -s "$DRIVE_SESSION" -x "$cols" -y "$rows" \
        -e "HOME=$HOME" -e "XDG_CONFIG_HOME=$XDG_CONFIG_HOME" -e "XDG_STATE_HOME=$XDG_STATE_HOME" \
        -e "XDG_DATA_HOME=$XDG_DATA_HOME" -e "PATH=$PATH" -e "AIW_E2E_LOG=$AIW_E2E_LOG" \
        -e "RUST_BACKTRACE=1" -e "TERM=${DRIVE_TERM:-xterm-256color}" -e "COLORTERM=${DRIVE_COLORTERM-truecolor}" \
        -e "GIT_AUTHOR_NAME=e2e" -e "GIT_AUTHOR_EMAIL=e2e@example.invalid" \
        -e "GIT_COMMITTER_NAME=e2e" -e "GIT_COMMITTER_EMAIL=e2e@example.invalid" \
        "cd $S && ${AIW_CMD:-env -u TMUX aiw} $* 2>>$S/log/aiw.stderr; echo \"[aiw exited \$?]\"; exec bash"
    dt set -g status off
    dt set -g mouse on
    # crossterm needs a moment to init
    sleep 0.6
}

keys() { dt send-keys -t "$DRIVE_SESSION" "$@"; }
cap() { dt capture-pane -p -t "$DRIVE_SESSION"; }
cap_ansi() { dt capture-pane -p -e -t "$DRIVE_SESSION"; }

wait_for() {
    local re=$1 secs=${2:-10} i=0
    while ! cap | grep -Eq -- "$re"; do
        i=$((i + 1))
        if [ "$i" -ge $((secs * 10)) ]; then return 1; fi
        sleep 0.1
    done
}

wait_gone() {
    local re=$1 secs=${2:-10} i=0
    while cap | grep -Eq -- "$re"; do
        i=$((i + 1))
        if [ "$i" -ge $((secs * 10)) ]; then return 1; fi
        sleep 0.1
    done
}

snap() {
    cap >"$S/captures/$1.txt"
    cap_ansi >"$S/captures/$1.ansi"
}

check() {
    local name=$1 rc=$2
    if [ "$rc" -eq 0 ]; then
        printf 'PASS\t%s\n' "$name" | tee -a "$RESULTS"
    else
        printf 'FAIL\t%s\n' "$name" | tee -a "$RESULTS"
        snap "fail-$(echo "$name" | tr -c 'A-Za-z0-9' '_')"
    fi
}

# q quits from the dashboard; it first closes a result dialog (e.g. the
# "Feature deleted" acknowledgement), so press it again if AIW is still up.
aiw_quit() {
    local i
    for i in $(seq 1 100); do
        [ $((i % 30)) -eq 1 ] && keys q
        [ -z "$(aiw_pid)" ] && return 0
        sleep 0.1
    done
    return 1
}

resize() { dt resize-window -t "$DRIVE_SESSION" -x "$1" -y "$2"; sleep 0.4; }

# row_glyph <text> <glyph>: the first dashboard row containing <text> shows
# <glyph> (⛔ needs you, ● working, ✓ finished, ○ idle, ■ stopped/exited)
row_glyph() { cap | grep -F -- "$1" | head -1 | grep -qF -- "$2"; }

# row_running <text>: the first row containing <text> is up (idle or working)
row_running() { row_glyph "$1" '○' || row_glyph "$1" '●'; }

# sel_glyph <glyph>: the selected row (accent bar ▎) shows <glyph>
sel_glyph() { cap | grep '▎' | head -1 | grep -qF -- "$1"; }

wait_sel_glyph() {
    local glyph=$1 secs=${2:-10} i=0
    while ! sel_glyph "$glyph"; do
        i=$((i + 1))
        if [ "$i" -ge $((secs * 10)) ]; then return 1; fi
        sleep 0.1
    done
}

# sgr_params_before <line> <needle>: the SGR parameters in force at <needle>
# (everything since the last reset), e.g. "1 38;2;255;176;0"
sgr_params_before() {
    python3 - "$1" "$2" <<'PY'
import re, sys
line, needle = sys.argv[1], sys.argv[2]
i = line.rfind(needle)
if i < 0:
    sys.exit(0)
chunks = re.split(r'\x1b\[0?m', line[:i])
print(" ".join(m[2:-1] for m in re.findall(r'\x1b\[[0-9;]*m', chunks[-1])))
PY
}

# sgr_color <params>: the colour part of SGR parameters (38;2;r;g;b or 38;5;n)
sgr_color() { echo "$1" | grep -oE '38;(2;[0-9]+;[0-9]+;[0-9]+|5;[0-9]+)' | tail -1; }

wait_row_glyph() {
    local text=$1 glyph=$2 secs=${3:-10} i=0
    while ! row_glyph "$text" "$glyph"; do
        i=$((i + 1))
        if [ "$i" -ge $((secs * 10)) ]; then return 1; fi
        sleep 0.1
    done
}

drive_kill() { dt kill-server 2>/dev/null || true; }

# pid of the aiw TUI running in this sandbox (the one whose HOME is $S)
aiw_pid() { pgrep -x aiw -u "$(id -u)" | while read -r p; do
    tr '\0' '\n' <"/proc/$p/environ" 2>/dev/null | grep -q "^HOME=$S$" && echo "$p"; done | head -1; }

panics() { grep -l "panicked at" "$S/log/aiw.stderr" "$XDG_STATE_HOME/ai_workstation/debug.log" 2>/dev/null; }

# add_project <name> <path> [harness-index 0..4]
add_project() {
    local name=$1 path=$2 idx=${3:-0} i
    keys N; wait_for 'New Project' 5 || return 1
    keys -l "$name"; keys Tab
    for i in $(seq 1 160); do keys BSpace; done
    keys -l "$path"; keys Enter
    for i in $(seq 1 "$idx"); do keys j; done
    keys Enter
    wait_for "Created project '$name'" 5
}

# new_feature <name> <worktree y|n> <agent-index 0..4 (claude codex pi gemini cursor)> [mode-index 0..2]
new_feature() {
    local name=$1 wt=$2 agent=${3:-0} mode=${4:-1} i
    keys n; wait_for 'New Feature' 5 || return 1
    if cap | grep -q 'Source:'; then keys Enter; sleep 0.2; fi
    wait_for 'Name:' 3 || return 1
    keys -l "$name"; keys Enter; sleep 0.2
    # the Worktree toggle defaults to whatever the project last used; read it
    local on_yes=0
    cap | grep -Eq '> *Yes' && on_yes=1
    if [ "$wt" = y ] && [ "$on_yes" = 0 ]; then keys j; fi
    if [ "$wt" = n ] && [ "$on_yes" = 1 ]; then keys j; fi
    keys Enter; sleep 0.2
    for i in $(seq 1 "$agent"); do keys j; done
    keys Enter
    for i in $(seq 1 "$mode"); do keys j; done
    keys Enter
    local extra=3
    [ "$agent" -eq 0 ] && extra=5
    for i in $(seq 1 "$extra"); do keys Enter; done
    sleep 0.3
    if cap | grep -q 'SuperVibe'; then keys y; sleep 0.3; fi
    # session name step
    keys Enter
}

# select_row <text>: jump the dashboard cursor to the first row matching <text>
# via the search dialog (`/`), falling back to walking the list with j.
select_row() {
    local text=$1 i
    keys /; sleep 0.2
    keys -l "$text"; sleep 0.3
    keys Enter; sleep 0.2
    if cap | grep -q '┌ Search'; then keys Escape; sleep 0.2; fi
    cap | grep '▎' | grep -Fq -- "$text" && return 0
    for i in $(seq 1 60); do keys k; done
    for i in $(seq 1 80); do
        cap | grep '▎' | grep -Fq -- "$text" && return 0
        keys j; sleep 0.05
    done
    return 1
}

# close_dialogs: Esc out of any open dialog (Esc on the dashboard itself is a no-op)
close_dialogs() {
    local i
    for i in $(seq 1 10); do
        cap | grep -qE '╭ (Projects|Sessions)' && return 0
        keys Escape; sleep 0.25
    done
}
