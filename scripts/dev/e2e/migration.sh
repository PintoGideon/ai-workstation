#!/usr/bin/env bash
# AMF → AIW first-run migration, in throwaway sandboxes.
#
#   scripts/dev/e2e/migration.sh <source-aiw.db> <sandbox-root> [aiw-binary]
#
# Builds a fake `~/.config/amf` from <source-aiw.db> (a consistent copy is
# taken with SQLite's backup API; the schema is the same) and starts the TUI
# once per case, on its own tmux driver socket:
#   plain     AMF setup with a database        → copied, toast shown, marker written
#   locked    amf.db held under BEGIN EXCLUSIVE → copy still succeeds (readers are not blocked)
#   nodb      config.json only, no amf.db       → settings copied, no database
#   opencode  every feature/session is OpenCode → loads; sessions dropped, features fall back
# The fake AMF folder is hashed before and after each run: it must not change.
set -uo pipefail

SRC_DB=${1:?source aiw.db}
ROOT=${2:?sandbox root}
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
AIW_BIN=${3:-$(cd "$HERE/../../.." && pwd)/target/release/aiw}
mkdir -p "$ROOT"
RESULTS="$ROOT/migration-results.tsv"
: >"$RESULTS"

snapshot() { (cd "$1" && find . -type f -print0 | sort -z | xargs -0 sha256sum) | sha256sum; }

make_amf() {
    local s=$1 with_db=$2 opencode=$3
    mkdir -p "$s/.config/amf/hooks" "$s/.local/state" "$s/.local/share" "$s/.amf/hooks"
    cat >"$s/.config/amf/config.json" <<'J'
{ "config_version": 1, "theme": "amf", "nerd_font": false, "compose_box": true }
J
    printf 'echo old-amf-hook\n' >"$s/.config/amf/hooks/notify.sh"
    printf '{"ports":{}}\n' >"$s/.config/amf/ports.json"
    mkdir -p "$s/.config/amf/notifications"
    printf '{"pending":true}\n' >"$s/.config/amf/notifications/stale.json"
    mkdir -p "$s/.config/amf/opencode-plugins"
    printf '// plugin\n' >"$s/.config/amf/opencode-plugins/x.js"
    if [ "$with_db" = yes ]; then
        python3 - "$SRC_DB" "$s/.config/amf/amf.db" "$opencode" <<'PY'
import sqlite3, sys
src, dst, opencode = sys.argv[1], sys.argv[2], sys.argv[3] == "yes"
a = sqlite3.connect(src); b = sqlite3.connect(dst); a.backup(b); a.close()
if opencode:
    b.execute("update features set agent='opencode'")
    b.execute("update feature_sessions set kind='opencode'")
    b.execute("update projects set preferred_agent='opencode'")
    b.commit()
b.close()
PY
    fi
}

run_case() {
    local name=$1 with_db=$2 opencode=$3 lock=$4
    local s="$ROOT/$name"
    rm -rf "$s"; make_amf "$s" "$with_db" "$opencode"
    local before; before=$(snapshot "$s/.config/amf")
    local locker=""
    if [ "$lock" = yes ]; then
        python3 - "$s/.config/amf/amf.db" <<'PY' &
import sqlite3, sys, time
c = sqlite3.connect(sys.argv[1], isolation_level=None)
c.execute("BEGIN EXCLUSIVE")
c.execute("update store_meta set value=value where 0")
time.sleep(120)
PY
        locker=$!
        sleep 0.5
    fi
    local sock="$s/drive.sock"
    tmux -S "$sock" new-session -d -s m -x 160 -y 45 \
        -e "HOME=$s" -e "XDG_CONFIG_HOME=$s/.config" -e "XDG_STATE_HOME=$s/.local/state" -e "XDG_DATA_HOME=$s/.local/share" \
        -e "PATH=$(dirname "$AIW_BIN"):$PATH" -e "TERM=xterm-256color" -e "COLORTERM=truecolor" -e "RUST_BACKTRACE=1" \
        "cd $s && env -u TMUX $AIW_BIN 2>$s/aiw.stderr; echo \"[aiw exited \$?]\"; exec bash"
    local i screen=""
    for i in $(seq 1 60); do
        sleep 0.25
        screen=$(tmux -S "$sock" capture-pane -p -t m 2>/dev/null)
        if grep -q "icon check" <<<"$screen"; then tmux -S "$sock" send-keys -t m n; fi
        if grep -q "Configure Agent Harnesses" <<<"$screen"; then tmux -S "$sock" send-keys -t m c; fi
        grep -Eq "AI Workstation v|aiw exited|AMF|amf" <<<"$screen" && [ "$i" -gt 12 ] && break
    done
    sleep 1
    screen=$(tmux -S "$sock" capture-pane -p -t m 2>/dev/null)
    printf '%s\n' "$screen" >"$s/screen.txt"
    local toast; toast=$(grep -iE "AMF|copied|migrat" <<<"$screen" | head -3 | sed 's/^ *//' | tr '\n' ' ')
    local exited; exited=$(grep -o "aiw exited [0-9]*" <<<"$screen" | head -1)
    tmux -S "$sock" send-keys -t m q 2>/dev/null; sleep 1
    tmux -S "$sock" kill-server 2>/dev/null
    [ -n "$locker" ] && kill "$locker" 2>/dev/null
    local after; after=$(snapshot "$s/.config/amf")
    local marker="$s/.config/ai_workstation/migrated-from-amf.json"
    local untouched=no; [ "$before" = "$after" ] && untouched=yes
    local migrated=no; [ -f "$marker" ] && migrated=yes
    local db=no; [ -f "$s/.config/ai_workstation/aiw.db" ] && db=yes
    local stderr; stderr=$(tail -c 300 "$s/aiw.stderr" 2>/dev/null | tr '\n' ' ')
    local logerr; logerr=$(grep -E "ERROR|WARN" "$s/.local/state/ai_workstation/debug.log" 2>/dev/null | grep -iE "migrat|amf" | tail -2 | cut -c1-200 | tr '\n' ' ')
    printf '%s\tamf_untouched=%s\tmarker=%s\taiw.db=%s\texit=%s\ttoast=%s\tstderr=%s\tlog=%s\n' \
        "$name" "$untouched" "$migrated" "$db" "${exited:-still running}" "$toast" "$stderr" "$logerr" | tee -a "$RESULTS"
    if [ -f "$marker" ]; then python3 -c "import json;d=json.load(open('$marker'));print('   marker:',d['projects'],'projects',d['features'],'features; copied',d['copied'],'skipped',d['skipped'])"; fi
}

run_case plain yes no no
run_case locked yes no yes
run_case nodb no no no
run_case opencode yes yes no
echo "results: $RESULTS"
