#!/usr/bin/env bash
# Repeatable end-to-end run of the `aiw` TUI against fake agent CLIs.
#
#   scripts/dev/e2e/run.sh [sandbox-dir] [aiw-binary]
#
# Default sandbox: /tmp/aiw-e2e-$USER (keep it short: tmux/IPC sockets live
# under it and Unix socket paths are limited to ~107 bytes). Everything AIW
# writes goes under the sandbox (HOME and XDG_* are pointed at it); the real
# ~/.config, ~/.local/state and the agent folders are never touched. The run
# drives AIW through tmux on its own socket and prints a PASS/FAIL table; the
# text captures used for the UI review are left in <sandbox>/captures.
#
# Needs: a release build (cargo +1.99.0 build --release --locked), tmux, git,
# python3.
set -uo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
S_IN=${1:-/tmp/aiw-e2e-${USER:-e2e}}
AIW_BIN=${2:-}

rm -rf "$S_IN"
"$HERE/setup-sandbox.sh" "$S_IN" ${AIW_BIN:+"$AIW_BIN"} >/dev/null || exit 1
# shellcheck disable=SC1091
source "$S_IN/env.sh"
# shellcheck disable=SC1091
source "$HERE/lib.sh"
export AIW_CMD="env -u TMUX AIW_PERF=1 aiw"
: >"$RESULTS"

cleanup() {
    aiw_quit 2>/dev/null
    tmux -S "$AIW_SOCK" kill-server 2>/dev/null || true
    pkill -f "sleep 600" 2>/dev/null || true
    drive_kill
}
trap cleanup EXIT

hook_prompts() {
    local r
    for r in 1 2 3; do
        cap | grep -q 'Run this repository hook' && { keys y; sleep 1.5; }
        cap | grep -q 'Running Hook' && { keys Enter; sleep 1.2; }
        cap | grep -q 'Resource Check' && { keys y; sleep 1; }
    done
}

no_panic() { [ "$(grep -c panicked "$XDG_STATE_HOME/ai_workstation/debug.log" 2>/dev/null)" = 0 ]; }
doctor_clean() { AIW_TMUX_SOCKET=$AIW_SOCK aiw doctor 2>/dev/null | grep -qE "no orphaned aiw-\*"; }

echo "sandbox: $S"
echo "=== Part 1: happy path"
aiw_start 200 60
wait_for 'icon check' 8; check "first run: icon check shown" $?
keys y; wait_for 'Configure Agent Harnesses' 10; check "first run: harness selection shown" $?
keys Enter j Enter j Enter j Enter j Enter; sleep 0.3
cap | grep -q '\[x\] Cursor (installed)'; check "all five fake CLIs detected as installed" $?
keys c; wait_for 'No projects yet' 5; check "empty dashboard" $?
# no concurrency guard for the rest of the run: every start would otherwise ask
aiw_quit; python3 - <<EOF
import json; p="$XDG_CONFIG_HOME/ai_workstation/config.json"; c=json.load(open(p)); c["max_concurrent_agents"]=0; json.dump(c,open(p,"w"),indent=2)
EOF
aiw_start 200 60; wait_for 'No projects yet' 8; check "relaunch after first run" $?

add_project alpha '~/repos/alpha' 0; check "add project via ~ path" $?
add_project beta "$S/repos/beta" 1; check "add project (absolute path)" $?
add_project gamma "$S/repos/gamma" 3; check "add project gamma" $?
add_project plain "$S/repos/plain" 2; check "add non-git project" $?

select_row 'alpha' >/dev/null; new_feature alpha-1 n 0 1; sleep 2; hook_prompts
wait_for "Created and started feature 'alpha-1'" 5; check "n: Claude feature in repo dir starts" $?
aiw_tmux list-windows -t aiw-alpha-alpha-1 -F '#W' | grep -q claude; check "tmux window for claude exists on AIW's socket" $?

select_row 'beta' >/dev/null; new_feature beta-1 y 1 1; sleep 2; hook_prompts; sleep 1
cap | grep -q 'beta-1'; check "n: Pi feature in a worktree (hook approved)" $?
test -f "$XDG_DATA_HOME"/ai_workstation/worktrees/*/beta-beta-1/.env; check ".env copied into the worktree (.worktreeinclude)" $?
test -f "$XDG_DATA_HOME"/ai_workstation/worktrees/*/beta-beta-1/.aiw-hook-ran; check "on_worktree_created hook ran" $?

select_row 'gamma' >/dev/null; new_feature gamma-1 n 0 1; sleep 2; hook_prompts
grep -q '"--approval-mode", "auto_edit"' "$S/log/gemini.args"; check "Gemini launched with --approval-mode auto_edit" $?
select_row 'plain' >/dev/null; new_feature plain-1 n 2 1; sleep 2; hook_prompts
test -f "$S/log/cursor-agent.args"; check "Cursor agent launched in the non-git project" $?

# batch creation makes a new project (workspace = the selected project's repo)
select_row 'alpha' >/dev/null; keys B; wait_for 'Workspace Path' 3; keys Enter; sleep 0.3
for i in $(seq 1 12); do keys BSpace; done; keys -l batch; keys Enter; sleep 0.3
keys j Enter; sleep 0.3; for i in $(seq 1 12); do keys BSpace; done; keys -l batch; keys Enter; sleep 0.3
keys j Enter j Enter Enter; sleep 2; hook_prompts
wait_for "Created project 'batch' with 2 features" 5; check "B: batch project with 2 Codex worktree features" $?

select_row 'Claude 1' >/dev/null; keys Enter; sleep 1; keys -l 'hello from e2e'; keys Enter; sleep 1.5
cap | grep -q "handled 'hello from e2e'"; check "typing goes straight to the agent" $?
grep -q '"event": "Stop"' "$S/log/claude.args" "$S/log/claude.hooks" 2>/dev/null; check "Claude Stop hook fired through AIW's settings.local.json" $?
keys C-q; sleep 0.8; cap | sed -n 2p | grep -q '✓ 1'; check "header pill shows 1 finished" $?
keys Enter; sleep 0.6; keys -l '/ask'; keys Enter; sleep 1.2; keys C-q; sleep 0.8
cap | sed -n 2p | grep -q '⚠ 1'; check "permission request → ⚠ 1 pill" $?
keys i; wait_for 'Needs Attention' 3; check "i: needs-you list" $?; close_dialogs

select_row 'alpha-1' >/dev/null; keys s; wait_for 'Start Session' 3; keys j j j j j Enter; sleep 0.5
wait_for 'Name Session' 3 && keys Enter; sleep 1.5
aiw_tmux list-windows -t aiw-alpha-alpha-1 -F '#W' | grep -q terminal; check "s: terminal session added" $?
select_row 'alpha-1' >/dev/null; keys s; wait_for 'Start Session' 3; keys j j j j j j j j j Enter; sleep 0.5
wait_for 'Name Session' 3 && keys Enter; sleep 1.5
aiw_tmux list-windows -t aiw-alpha-alpha-1 -F '#W' | grep -q logs; check "s: custom session from aiw.json added" $?
select_row 'alpha-1' >/dev/null; keys s; wait_for 'Start Session' 3; keys j Enter; sleep 0.5
wait_for 'Name Session' 3 && keys Enter; sleep 1.5; hook_prompts; keys C-q; sleep 0.5

select_row 'alpha-1' >/dev/null; keys v; sleep 2.5; cap | tail -1 | grep -q leave; check "v: split view" $?
snap p1-split-200x60
resize 120 40; sleep 1.5; resize 80 24; sleep 1.5; snap p1-split-80x24; resize 40 10; sleep 1.5; resize 200 60; sleep 1
no_panic && ! cap | grep -q 'aiw exited'; check "split view survives 120x40 / 80x24 / 40x10 resizes" $?
keys C-Space; keys v; sleep 0.8; cap | grep -q '╭ Projects'; check "leave split view" $?

select_row 'Codex 1' >/dev/null; keys r; sleep 0.4; for i in $(seq 1 40); do keys BSpace; done; keys -l cx; keys Enter; sleep 0.8
cap | grep -q "Renamed to 'cx'"; check "r: rename session" $?
select_row 'alpha-1' >/dev/null; keys x; sleep 2; ! aiw_tmux has-session -t aiw-alpha-alpha-1 2>/dev/null; check "x: stop feature kills its tmux session" $?
keys c; sleep 2; hook_prompts; aiw_tmux has-session -t aiw-alpha-alpha-1 2>/dev/null; check "c: start feature again" $?
select_row 'Claude 1' >/dev/null; keys S; wait_for 'Claude Sessions' 4; check "S: saved Claude conversations listed" $?
keys Enter; sleep 0.8
wait_for 'Confirm Restart' 3 && keys y; sleep 3; hook_prompts; keys C-q; sleep 0.5
tail -1 "$S/log/claude.args" | grep -q '"--resume"'; check "S: Claude relaunched with --resume <id>" $?

keys z; sleep 0.6; cap | grep -qi 'dormant'; check "z: dormant list" $?; close_dialogs
keys '?'; wait_for 'Keybindings' 3; check "?: help overlay" $?; close_dialogs
keys /; keys -l gam; sleep 0.4; cap | grep -q 'gamma-1'; check "/: search finds features" $?; close_dialogs

select_row 'gamma-1' >/dev/null; keys d; wait_for 'Confirm Delete' 3; keys y; sleep 2.5
! aiw_tmux has-session -t aiw-gamma-gamma-1 2>/dev/null; check "d: delete feature" $?

aiw_quit; check "q: quit" $?
aiw_start 200 60; sleep 2.5; cap | grep -q 'alpha-1'; check "relaunch: projects and features persist" $?
aiw_tmux has-session -t aiw-alpha-alpha-1 2>/dev/null; check "relaunch: agent sessions kept running" $?
doctor_clean; check "aiw doctor: no orphans" $?

echo "=== Part 2: adversarial"
select_row 'Claude 1' >/dev/null; keys Enter; sleep 1
aiw_tmux kill-window -t aiw-alpha-alpha-1:claude; sleep 2; keys C-q; sleep 6
cap | grep -E 'Claude 1' | head -1 | grep -q '■'; check "killed window from outside → session shows stopped" $?
no_panic; check "no panic after external kill" $?

wt=$(ls -d "$XDG_DATA_HOME"/ai_workstation/worktrees/*/batch-batch1 2>/dev/null | head -1)
select_row 'batch1' >/dev/null; keys x; sleep 1.5; rm -rf "$wt"
keys c; sleep 3; hook_prompts; sleep 1
test -f "$wt/.git"; check "deleted worktree dir is recreated on start" $?

printf '{ "lifecycle_hooks": { "on_start": ' >"$S/repos/gamma/aiw.json"
select_row 'gamma' >/dev/null; new_feature gamma-2 n 0 1; sleep 2; hook_prompts
cap | grep -q 'gamma-2'; check "corrupt aiw.json does not block feature creation" $?
cap | grep -q 'aiw.json in gamma is invalid'; check "corrupt aiw.json → toast names the project and the error" $?

echo '{"session_id":"nope","cwd":"/tmp","hook_event_name":"Stop"}' | AIW_ACTIVE=1 AIW_SESSION=aiw-ghost aiw notify --type thinking-start
sleep 1; cap | grep -q '╭ Projects' && no_panic; check "hook for an unknown session is harmless" $?
rm -f "$XDG_STATE_HOME/ai_workstation/aiw.sock"; sleep 7
test -S "$XDG_STATE_HOME/ai_workstation/aiw.sock"; check "removed IPC socket is re-bound within the sweep" $?

# a second TUI must not share the database and steal the socket; read-only
# subcommands keep working alongside the running one
second=$(timeout 10 env -u TMUX aiw </dev/null 2>&1); rc=$?
[ "$rc" -eq 1 ] && echo "$second" | grep -q "already running (pid $(aiw_pid))"; check "second aiw instance exits 1 with 'already running (pid N)'" $?
AIW_TMUX_SOCKET=$AIW_SOCK timeout 30 aiw doctor >/dev/null 2>&1; check "aiw doctor works alongside the running TUI" $?
test -S "$XDG_STATE_HOME/ai_workstation/aiw.sock" && [ -n "$(aiw_pid)" ]; check "first instance keeps running with its socket" $?

# an agent that exits (/crash) leaves its shell: the row must say so, and
# `c` must launch the agent again into that same window
select_row 'Claude 1' >/dev/null; keys c; sleep 3; hook_prompts; sleep 1
row_glyph 'Claude 1' '●'; check "c: restart the stopped Claude session" $?
n_before=$(wc -l <"$S/log/claude.args")
done_before=$(cap | sed -n 2p | grep -o '✓ [0-9]*')
keys Enter; sleep 1.5; keys -l '/crash'; keys Enter; sleep 1; keys C-q; sleep 0.5
wait_row_glyph 'Claude 1' '■' 20; check "/crash: agent gone, shell alive → row shows ■ exited" $?
[ "$(cap | sed -n 2p | grep -o '✓ [0-9]*')" = "$done_before" ]; check "/crash: no 'finished' attention pill" $?
aiw_tmux list-windows -t aiw-alpha-alpha-1 -F '#W' | grep -qx claude; check "/crash: the window itself is still there" $?

# the feature's whole tmux session ends (its last window closed) and the user
# presses Enter and types before the next sweep: no error per keystroke
aiw_tmux kill-session -t aiw-alpha-alpha-1 2>/dev/null
select_row 'Claude 1' >/dev/null; keys Enter; sleep 0.4; keys -l 'xyz'; sleep 2.5; hook_prompts
! cap | grep -q 'send-keys failed'; check "whole tmux session gone: Enter + typing shows no send-keys error" $?
no_panic; check "whole tmux session gone: no panic" $?
close_dialogs; keys C-q; sleep 0.3; close_dialogs
select_row 'Claude 1' >/dev/null; keys c; sleep 3; hook_prompts; sleep 1
row_glyph 'Claude 1' '●' && [ "$(wc -l <"$S/log/claude.args")" -gt "$n_before" ]; check "c: exited agent is launched again" $?
[ "$(aiw_tmux list-windows -t aiw-alpha-alpha-1 -F '#W' | grep -cx claude)" = 1 ]; check "c: relaunch reused the window (no duplicate)" $?
keys C-q; sleep 0.3

# names git refuses are stopped in the dialog with a plain message
select_row 'alpha' >/dev/null; keys n; wait_for 'New Feature' 5
if cap | grep -q 'Source:'; then keys Enter; sleep 0.2; fi
wait_for 'Name:' 3; keys -l 'bad name'; keys Enter; sleep 0.6
cap | grep -q 'cannot contain spaces'; check "feature name with a space → plain message, no git error" $?
close_dialogs

# an unparsable notification file is moved aside, not re-parsed for ever
junk="$XDG_CONFIG_HOME/ai_workstation/notifications/junk-e2e.json"
mkdir -p "$(dirname "$junk")"; printf '{"session_id": ' >"$junk"; touch -d '30 seconds ago' "$junk"; sleep 7
test -f "$XDG_STATE_HOME/ai_workstation/notifications/bad/junk-e2e.json" && ! test -e "$junk"; check "malformed notification file is quarantined by the sweep" $?
no_panic; check "no panic after the malformed notification" $?

# Esc on the dashboard is a no-op (q quits)
keys Escape; sleep 0.6
[ -n "$(aiw_pid)" ] && cap | grep -q '╭ Projects'; check "Esc on the dashboard does not quit" $?

select_row 'alpha' >/dev/null; new_feature taken-branch y 0 1; sleep 2; hook_prompts
cap | grep -q "already checked out"; check "branch checked out elsewhere → clear message" $?; close_dialogs
select_row 'alpha' >/dev/null; new_feature alpha-1 n 0 1; sleep 1.5
cap | grep -q "already exists"; check "duplicate feature name → message" $?; close_dialogs

"$HERE/random-keys.py" 500 >"$S/keys.txt"
while IFS= read -r k; do case "$k" in ?) keys -l "$k";; *) keys "$k";; esac; done <"$S/keys.txt"; sleep 2
no_panic && ! cap | grep -q 'aiw exited'; check "500 random dashboard keys: no panic, no exit" $?
keys C-q; keys C-q; sleep 0.5; close_dialogs
for sz in "120 40" "80 24" "40 10"; do set -- $sz; resize "$1" "$2"; sleep 0.8; snap "p3-dashboard-${1}x${2}"; keys '?'; sleep 0.5; snap "p3-help-${1}x${2}"; close_dialogs; done
resize 200 60; sleep 0.8; no_panic; check "dashboard/help at 120x40, 80x24, 40x10" $?
aiw_quit; doctor_clean; check "doctor clean after the adversarial run" $?

echo "=== Part 3: AMF migration cases"
"$HERE/migration.sh" "$XDG_CONFIG_HOME/ai_workstation/aiw.db" "$S/mig" >"$S/migration.log" 2>&1
grep -q "plain.*amf_untouched=yes.*marker=yes.*aiw.db=yes" "$S/mig/migration-results.tsv"; check "migration: AMF setup copied, AMF folder untouched" $?
grep -q "nodb.*marker=yes" "$S/mig/migration-results.tsv"; check "migration: settings without a database" $?
grep -q "opencode.*marker=yes" "$S/mig/migration-results.tsv"; check "migration: OpenCode-only features load" $?

echo
echo "=== Results ($RESULTS)"
printf 'PASS %s  FAIL %s\n' "$(grep -c '^PASS' "$RESULTS")" "$(grep -c '^FAIL' "$RESULTS")"
grep '^FAIL' "$RESULTS" || true
echo "captures: $S/captures"
