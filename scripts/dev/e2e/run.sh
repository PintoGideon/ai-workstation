#!/usr/bin/env bash
# Repeatable end-to-end run of the `aiw` cockpit against fake agent CLIs.
#
#   scripts/dev/e2e/run.sh [sandbox-dir] [aiw-binary]
#
# Default sandbox: /tmp/aiw-e2e-$USER (keep it short: tmux/IPC sockets live
# under it and Unix socket paths are limited to ~107 bytes). Everything AIW
# writes goes under the sandbox (HOME and XDG_* are pointed at it); the real
# ~/.config, ~/.local/state and the agent folders are never touched.
#
# `aiw` builds a tmux cockpit (DESIGN.md §18): the sidebar in the left pane,
# the selected agent's real tmux pane on the right. The run drives the cockpit
# client through tmux on its own socket (keys land in whichever pane has
# focus; Ctrl+Q flips), and prints a PASS/FAIL table; the text captures used
# for the UI review are left in <sandbox>/captures.
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
    cockpit_kill
    tmux -S "$AIW_SOCK" kill-server 2>/dev/null || true
    pkill -f "sleep 600" 2>/dev/null || true
    # §19.7: the fake `claude --bg` workers are detached; end the ones this
    # sandbox started (matched by their HOME, never by a pattern on the
    # command line, which would match this shell too).
    local p
    for p in $(pgrep -u "$(id -u)" -f -- '--aiw-e2e-bg-worker' 2>/dev/null); do
        tr '\0' '\n' <"/proc/$p/environ" 2>/dev/null | grep -qx "HOME=$S" && kill "$p" 2>/dev/null
    done
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

no_panic() { [ "$(grep -c panicked "$XDG_STATE_HOME/ai_workstation/debug.log" 2>/dev/null)" = 0 ] && ! grep -q panicked "$S/log/aiw.stderr" 2>/dev/null; }
doctor_clean() { AIW_TMUX_SOCKET=$AIW_SOCK aiw doctor 2>/dev/null | grep -qE "no orphaned aiw-\*"; }
view_sessions() { aiw_tmux list-sessions -F '#S' 2>/dev/null | grep -c '^aiw-view-'; }
sidebar_width() { ck display-message -p -t aiw:0.0 '#{pane_width}' 2>/dev/null; }
# type into the agent pane: focus it, send the text, come back to the sidebar
agent_type() { focus_agent; keys -l "$1"; keys Enter; sleep "${2:-1.5}"; focus_sidebar; }

echo "sandbox: $S"
echo "=== Part 1: the cockpit comes up"
aiw_start 200 60
wait_for 'icon check' 8; check "first run: icon check shown" $?
cockpit_alive; check "cockpit: aiw built its own tmux server (cockpit.sock) and attached" $?
[ "$(zoomed)" = 1 ]; check "cockpit: the first-run check zooms the sidebar pane" $?
keys y; wait_for 'Configure Agent Harnesses' 10; check "first run: harness selection shown" $?
wait_for '\[x\] Cursor \(installed\)' 10
[ "$(cap | grep -c '\[x\] .* (installed)')" = 5 ]; check "first run: all five fake CLIs probed and ticked without a keypress" $?
keys j Enter; sleep 0.3; cap | grep -q '\[ \] Codex'; check "first run: Enter unticks a found harness" $?
keys Enter; wait_for '\[x\] Codex \(installed\)' 5; check "first run: Enter re-checks and ticks it again" $?
keys c; wait_for 'No projects yet' 5; check "empty dashboard" $?
sleep 0.6; [ "$(zoomed)" = 0 ]; check "cockpit: back on the dashboard the sidebar un-zooms" $?
[ "$(sidebar_width)" = 36 ]; check "cockpit: the sidebar pane is sidebar_width (36) columns" $?
[ "$(pane_active)" = 0 ] && [ "$(agent_title)" = "select a session" ]; check "cockpit: sidebar focused, placeholder titled 'select a session'" $?
snap p0-empty-200x60
# no concurrency guard for the rest of the run: every start would otherwise ask
aiw_quit; check "q: quit tears the cockpit down" $?
! cockpit_alive && ! [ -e "$COCKPIT_SOCK" ]; check "q: no cockpit server and no cockpit socket left" $?
python3 - <<PY
import json; p="$XDG_CONFIG_HOME/ai_workstation/config.json"; c=json.load(open(p)); c["max_concurrent_agents"]=0; c["permission_wait_seconds"]=5; json.dump(c,open(p,"w"),indent=2)
PY
grep -q '"sidebar_width": 36' "$XDG_CONFIG_HOME/ai_workstation/config.json"; check "config: sidebar_width written with its default" $?
! grep -qE '"dashboard_layout"|"compose_box"|"view_auto_refresh"' "$XDG_CONFIG_HOME/ai_workstation/config.json"; check "config: removed keys are not written" $?
aiw_start 200 60; wait_for 'No projects yet' 8; check "relaunch after first run" $?

echo "=== Part 1b: projects and agents (Claude, Pi, Gemini, Cursor, Codex)"
add_project alpha '~/repos/alpha' 0; check "add project via ~ path" $?
wait_for '╭ Sessions' 3; check "the sidebar appears with the first project" $?
add_project beta "$S/repos/beta" 1; check "add project (absolute path)" $?
add_project gamma "$S/repos/gamma" 3; check "add project gamma" $?
add_project plain "$S/repos/plain" 2; check "add non-git project" $?

select_row 'alpha' >/dev/null; new_feature alpha-1 n 0 1; sleep 2; hook_prompts
# The cockpit has no status panel, so check the outcome: the feature is stored
# and its tmux session is up on AIW's socket.
wait_feature_running alpha alpha-1 aiw-alpha-alpha-1 8; check "n: Claude feature in repo dir starts" $?
aiw_tmux list-windows -t aiw-alpha-alpha-1 -F '#W' | grep -q claude; check "tmux window for claude exists on AIW's socket" $?
wait_title 'alpha › alpha-1 › Claude 1' 6; check "cockpit: the agent pane shows the new feature's agent" $?

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
# (the new project's row starts folded, so its features are not on screen)
wait_project_features batch 2 8 && wait_for '[▾▸] batch' 3; check "B: batch project with 2 Codex worktree features" $?
grep -q 'developer_instructions' "$S/log/codex.args"; check "Codex launched (with AIW's developer instructions)" $?
# a second agent in alpha-1: Codex beside Claude, same checkout
# §19.7: "Claude (background)" sits right after Claude in the picker.
select_row 'alpha-1' >/dev/null; keys s; wait_for 'Start Session' 3; keys j j Enter; sleep 0.5
wait_for 'Name Session' 3 && keys Enter; sleep 1.5; hook_prompts; focus_sidebar
aiw_tmux list-windows -t aiw-alpha-alpha-1 -F '#W' | grep -q codex; check "s: a Codex session added beside Claude in the same feature" $?

echo "=== Part 1c: native typing"
select_row 'Claude 1' >/dev/null; sleep 0.5
wait_title '› Claude 1' 6; check "selection: the agent pane follows the row (debounced)" $?
keys Enter; sleep 0.5; [ "$(pane_active)" = 1 ]; check "Enter: focuses the agent's real pane" $?
keys -l 'hello from e2e'; keys Enter; sleep 1.5
wait_agent_for "handled 'hello from e2e' \(claude\)" 6; check "typing is native: the agent echoed what was typed in its own pane" $?
# The info block may already show the session's transcript title (the typed
# prompt) by now; that is the agent's transcript, not sidebar input. Typing that
# reached the sidebar would show in the list/footer or open a zoomed dialog.
! cap_no_info | grep -q 'hello from e2e' && [ "$(zoomed)" != 1 ]; check "nothing was typed through the sidebar" $?
grep -q '"event": "Stop"' "$S/log/claude.args" "$S/log/claude.hooks" 2>/dev/null; check "Claude Stop hook fired through AIW's hook settings (--settings)" $?
keys C-q; sleep 0.5; [ "$(pane_active)" = 0 ]; check "Ctrl+Q: back to the sidebar (tmux root key)" $?
cap | sed -n 2p | grep -q '✓ 1'; check "header pill shows 1 finished" $?
keys C-q; sleep 0.3; [ "$(pane_active)" = 1 ]; check "Ctrl+Q from the sidebar focuses the agent" $?
keys -l '/ask'; keys Enter; sleep 1.2; keys C-q; sleep 0.5
wait_sel_glyph '⛔' 8; cap | sed -n 2p | grep -q '⛔ 1'; check "permission request → ⛔ 1 pill" $?
keys i; wait_for 'Needs Attention' 3; check "i: needs-you list" $?
[ "$(zoomed)" = 1 ]; check "dialogs zoom the sidebar over the agent pane" $?
snap p1-needs-you-zoomed
close_dialogs; sleep 0.6; [ "$(zoomed)" = 0 ]; check "closing the dialog un-zooms" $?

select_row 'alpha-1' >/dev/null; keys s; wait_for 'Start Session' 3; keys j j j j j j Enter; sleep 0.5
wait_for 'Name Session' 3 && keys Enter; sleep 1.5
aiw_tmux list-windows -t aiw-alpha-alpha-1 -F '#W' | grep -q terminal; check "s: terminal session added" $?
select_row 'alpha-1' >/dev/null; keys s; wait_for 'Start Session' 3; keys j j j j j j j j j j Enter; sleep 0.5
wait_for 'Name Session' 3 && keys Enter; sleep 1.5
aiw_tmux list-windows -t aiw-alpha-alpha-1 -F '#W' | grep -q logs; check "s: custom session from aiw.json added" $?
select_row 'alpha-1' >/dev/null; keys s; wait_for 'Start Session' 3; keys Enter; sleep 0.5
wait_for 'Name Session' 3 && keys Enter; sleep 1.5; hook_prompts; focus_sidebar

echo "=== Part 1d: tiles in the right pane"
select_row 'alpha-1' >/dev/null; keys v; sleep 2.5
agent_title | grep -q 'tiles: alpha/alpha-1'; check "v: aiw grid runs in the agent pane" $?
ls "$XDG_STATE_HOME/ai_workstation/" | grep -q 'aiw-grid-.*\.sock'; check "v: the grid server is up on its own socket" $?
snap p1-grid-200x60
resize 120 40; sleep 1.5; resize 80 24; sleep 1.5; snap p1-grid-80x24; resize 40 10; sleep 1.5; resize 200 60; sleep 1
no_panic && cockpit_alive; check "grid survives 120x40 / 80x24 / 40x10 resizes" $?
[ "$(sidebar_width)" = 36 ]; check "cockpit: the sidebar is pinned at 36 columns across resizes" $?
focus_agent; keys M-q; sleep 2
wait_title 'alpha › alpha-1 › ' 6; check "Alt+q closes the grid; the pane returns to the selected agent" $?
! ls "$XDG_STATE_HOME/ai_workstation/" | grep -q 'aiw-grid-.*\.sock'; check "Alt+q: the grid server and its socket are gone" $?
focus_sidebar

echo "=== Part 1e: the rest of the dashboard"
select_row 'Codex 1' >/dev/null; keys r; sleep 0.4; for i in $(seq 1 40); do keys BSpace; done; keys -l cx; keys Enter; sleep 0.8
# the toast can be gone by now: check the stored label and the row
for i in $(seq 1 30); do [ -n "$(db_q "select id from feature_sessions where label = 'cx'")" ] && break; sleep 0.1; done
[ -n "$(db_q "select id from feature_sessions where label = 'cx'")" ] && wait_for '(^|[^a-z])cx( |$)' 3; check "r: rename session" $?
select_row 'alpha-1' >/dev/null; keys x; sleep 2; ! aiw_tmux has-session -t aiw-alpha-alpha-1 2>/dev/null; check "x: stop feature kills its tmux session" $?
wait_title 'select a session' 6; check "x: the agent pane falls back to the placeholder" $?
keys c; sleep 2; hook_prompts; aiw_tmux has-session -t aiw-alpha-alpha-1 2>/dev/null; check "c: start feature again" $?
select_row 'Claude 1' >/dev/null; resume_mark=$(wc -l <"$S/log/claude.args"); keys S; wait_for 'Claude Sessions' 4; check "S: saved Claude conversations listed" $?
keys Enter; sleep 0.8
wait_for 'Confirm Restart' 3 && keys y; sleep 3; hook_prompts; focus_sidebar
# Any launch recorded after the picker opened: a later Claude launch (another
# session) can be the last line, so do not read only the tail.
tail -n +"$((resume_mark + 1))" "$S/log/claude.args" | grep -q '"--resume"'; check "S: Claude relaunched with --resume <id>" $?

keys z; sleep 0.6; cap | grep -qi 'dormant'; check "z: dormant list" $?; close_dialogs
keys '?'; wait_for 'Keybindings' 3; check "?: help overlay" $?
[ "$(zoomed)" = 1 ]; check "?: the help zooms the sidebar" $?
snap p1-help-zoomed
# "attach" is legitimate now for background Claude rows (claude attach) and in
# "typing is native, as in tmux attach"; only the old attach-mode claims are stale.
! cap | grep -iE 'compose|Tab.*type|full view|attach' | grep -viqE 'tmux attach|background Claude|claude attach'; check "?: no stale compose / Tab / view / attach claims" $?
close_dialogs
keys /; keys -l gam; sleep 0.4; cap | grep -q 'gamma-1'; check "/: search finds features" $?; close_dialogs

select_row 'gamma-1' >/dev/null; keys d; wait_for 'Confirm Delete' 3; keys y; sleep 2.5
! aiw_tmux has-session -t aiw-gamma-gamma-1 2>/dev/null; check "d: delete feature" $?

aiw_quit; check "q: quit" $?
[ "$(view_sessions)" = 0 ]; check "q: no aiw-view sessions left on AIW's socket" $?
aiw_tmux has-session -t aiw-alpha-alpha-1 2>/dev/null; check "q: agent sessions kept running" $?
aiw_start 200 60; sleep 2.5; cap | grep -q 'alpha-1'; check "relaunch: projects and features persist" $?
doctor_clean; check "aiw doctor: no orphans" $?

echo "=== Part 2: adversarial"
select_row 'Claude 1' >/dev/null; sleep 1
aiw_tmux kill-window -t aiw-alpha-alpha-1:claude; sleep 6
cap | grep -E 'Claude 1' | head -1 | grep -q '■'; check "killed window from outside → session shows stopped" $?
wait_title 'select a session' 6; check "killed window → the agent pane shows the placeholder" $?
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
sleep 1; cap | grep -q '╭ Sessions' && no_panic; check "hook for an unknown session is harmless" $?
rm -f "$XDG_STATE_HOME/ai_workstation/aiw.sock"; sleep 7
test -S "$XDG_STATE_HOME/ai_workstation/aiw.sock"; check "removed IPC socket is re-bound within the sweep" $?

# a second `aiw` attaches a second client to the same cockpit (the lock is the
# sidebar's, not the launcher's); without a terminal the launcher says so
second=$(timeout 10 env -u TMUX aiw </dev/null 2>&1); rc=$?
[ "$rc" -eq 1 ] && echo "$second" | grep -q "needs an interactive terminal"; check "aiw without a terminal exits 1 with a clear message" $?
dt new-window -d -t "$DRIVE_SESSION" "cd $S && env -u TMUX aiw 2>>$S/log/aiw2.stderr; exec bash"; sleep 2.5
[ "$(ck list-clients 2>/dev/null | wc -l)" = 2 ]; check "a second aiw attaches a second client to the running cockpit" $?
grep -q 'attaching to the running dashboard' "$S/log/aiw2.stderr"; check "the second aiw says it is attaching" $?
dt kill-window -t "$DRIVE_SESSION:1"; sleep 1
[ "$(ck list-clients 2>/dev/null | wc -l)" = 1 ] && cockpit_alive; check "closing the second client leaves the cockpit up" $?
AIW_TMUX_SOCKET=$AIW_SOCK timeout 30 aiw doctor >/dev/null 2>&1; check "aiw doctor works alongside the running TUI" $?
test -S "$XDG_STATE_HOME/ai_workstation/aiw.sock" && [ -n "$(aiw_pid)" ]; check "first instance keeps running with its socket" $?

# an agent that exits (/crash) leaves its shell: the row must say so, and
# `c` must launch the agent again into that same window
select_row 'Claude 1' >/dev/null; keys c; sleep 3; hook_prompts
wait_row_glyph 'Claude 1' '○' 10 || wait_row_glyph 'Claude 1' '●' 5
row_running 'Claude 1'; check "c: restart the stopped Claude session" $?
n_before=$(wc -l <"$S/log/claude.args")
done_before=$(cap | sed -n 2p | grep -o '✓ [0-9]*')
agent_type '/crash' 1
wait_row_glyph 'Claude 1' '■' 20; check "/crash: agent gone, shell alive → row shows ■ exited" $?
[ "$(cap | sed -n 2p | grep -o '✓ [0-9]*')" = "$done_before" ]; check "/crash: no 'finished' attention pill" $?
aiw_tmux list-windows -t aiw-alpha-alpha-1 -F '#W' | grep -qx claude; check "/crash: the window itself is still there" $?

# the feature's whole tmux session ends (its last window closed): the pane
# shows the placeholder, Enter has nothing to focus, nothing panics
aiw_tmux kill-session -t aiw-alpha-alpha-1 2>/dev/null
select_row 'Claude 1' >/dev/null; keys Enter; sleep 0.4; keys -l 'jjj'; sleep 2.5; hook_prompts
wait_title 'select a session' 8; check "whole tmux session gone: the agent pane shows the placeholder" $?
[ "$(pane_active)" = 0 ]; check "whole tmux session gone: Enter kept the sidebar (nothing to focus)" $?
no_panic; check "whole tmux session gone: no panic" $?
close_dialogs
select_row 'Claude 1' >/dev/null; keys c; sleep 3; hook_prompts
wait_row_glyph 'Claude 1' '○' 10 || wait_row_glyph 'Claude 1' '●' 5
row_running 'Claude 1' && [ "$(wc -l <"$S/log/claude.args")" -gt "$n_before" ]; check "c: exited agent is launched again" $?
[ "$(aiw_tmux list-windows -t aiw-alpha-alpha-1 -F '#W' | grep -cx claude)" = 1 ]; check "c: relaunch reused the window (no duplicate)" $?
focus_sidebar

# names git refuses are stopped in the dialog with a plain message
select_row 'alpha' >/dev/null; keys n; wait_for 'New Feature' 5; sleep 0.3  # a frame can be caught half drawn
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
[ -n "$(aiw_pid)" ] && cap | grep -q '╭ Sessions'; check "Esc on the dashboard does not quit" $?

select_row 'alpha' >/dev/null; new_feature taken-branch y 0 1; sleep 2; hook_prompts
cap | grep -q "already checked out"; check "branch checked out elsewhere → clear message" $?; close_dialogs
select_row 'alpha' >/dev/null; new_feature alpha-1 n 0 1; sleep 1.5
cap | grep -q "already exists"; check "duplicate feature name → message" $?; close_dialogs

"$HERE/random-keys.py" 500 >"$S/keys.txt"
focus_sidebar
while IFS= read -r k; do case "$k" in ?) keys -l "$k";; *) keys "$k";; esac; done <"$S/keys.txt"; sleep 2
no_panic && cockpit_alive && [ -n "$(aiw_pid)" ]; check "500 random dashboard keys: no panic, no exit" $?
focus_sidebar; close_dialogs
for sz in "120 40" "80 24" "40 10"; do set -- $sz; resize "$1" "$2"; sleep 0.8; snap "p3-cockpit-${1}x${2}"; keys '?'; sleep 0.5; snap "p3-help-${1}x${2}"; close_dialogs; done
resize 200 60; sleep 0.8; no_panic; check "cockpit/help at 120x40, 80x24, 40x10" $?

echo "=== Part 2b: the cockpit (DESIGN.md §18)"
close_dialogs
# a fresh feature with two fake agents whose banners differ
select_row 'alpha' >/dev/null; new_feature ws-1 y 0 1; sleep 2; hook_prompts
wait_for "Created and started feature" 5 && aiw_tmux has-session -t aiw-alpha-ws-1 2>/dev/null; check "ws: feature with a Claude agent" $?
select_row 'ws-1' >/dev/null; keys s; wait_for 'Start Session' 3; keys j j Enter; sleep 0.5
wait_for 'Name Session' 3 && keys Enter; sleep 1.5; hook_prompts; focus_sidebar
aiw_tmux list-windows -t aiw-alpha-ws-1 -F '#W' | grep -q codex; check "ws: second agent (codex) added" $?
cap | sed -n 2p | grep -qE '⛔ [0-9]+ · ● [0-9]+ · ✓ [0-9]+'; check "ws: header pills still count" $?
select_row 'ws-1' >/dev/null; keys j; sleep 0.6
wait_title '› Claude 1' 6; check "ws: the agent pane shows the selected session" $?
keys j; sleep 0.6; wait_title '› Codex 1' 6; check "ws: j moves the agent pane to the next agent" $?
[ "$(view_sessions)" -le 2 ]; check "ws: old view sessions are destroyed when the pane moves on" $?
keys k; sleep 0.6; wait_title '› Claude 1' 6; check "ws: k moves it back" $?
snap ws-200x60
# native typing: Enter focuses the agent's real tmux pane, typing is the
# terminal's own, Ctrl+Q comes back
keys Enter; sleep 0.5; [ "$(pane_active)" = 1 ]; check "attach: Enter focuses the agent pane" $?
keys -l 'hello native'; keys Enter; wait_agent_for "handled 'hello native' \(claude\)" 6; check "attach: typing reaches the agent natively" $?
snap ws-typing
keys C-q; sleep 0.5; [ "$(pane_active)" = 0 ]; check "attach: Ctrl+Q returns to the sidebar" $?
agent_type '/ask' 1.5
wait_sel_glyph '⛔' 6; check "ws: /ask → row shows ⛔" $?
cap | grep '▎' | sed -n 2p | grep -q 'permission: Bash'; check "ws: /ask → reason line under the row" $?
snap ws-needs-you
agent_type '/done' 1.5
wait_sel_glyph '✓' 6; check "ws: /done → row shows ✓" $?
row=$(cap_ansi | grep -F '▎' | grep -F '✓' | head -1)
echo "$(sgr_params_before "$row" '✓')" | grep -qE '(^|[ ;])1([ ;]|$)'; check "ws: unread ✓ is bright (bold)" $?
keys Enter; sleep 0.4; keys C-q; sleep 0.4
row=$(cap_ansi | grep -F '▎' | grep -F '✓' | head -1)
! echo "$(sgr_params_before "$row" '✓')" | grep -qE '(^|[ ;])1([ ;]|$)'; check "ws: ✓ dims once the agent pane was focused" $?
# the info block: title from the harness's own transcript, model and cost
# from the status line, branch and worktree from the store
wait_for '╭ Claude 1 ─' 8; check "info: the block under the sidebar names the selected session" $?
wait_for 'hello native' 8; check "info: the title is the session's first prompt (from Claude's transcript)" $?
cap | grep -q 'Claude Code · Fake 1'; check "info: harness and model (from the status line)" $?
cap | grep -qE 'ctx 42% · \$0\.84'; check "info: context % and cost" $?
cap | grep -q '⎇ ws-1'; check "info: the branch" $?
cap | grep -qE '…/alpha-[0-9a-f]+/alpha-ws-1|worktrees'; check "info: the worktree, shortened" $?
snap ws-info-block
agent_type '/crash' 0.5
wait_sel_glyph '■' 20; check "ws: /crash → row shows ■" $?
cap | grep '▎' | sed -n 2p | grep -q 'exited'; check "ws: /crash → reason line says exited" $?
snap ws-exited
keys v; sleep 2; agent_title | grep -q 'tiles: alpha/ws-1'; check "ws: v tiles the feature's agents in the pane" $?
focus_agent; keys M-q; sleep 2; wait_title 'alpha › ws-1 › ' 6; check "ws: Alt+q leaves the tiles, back to the agent" $?
focus_sidebar
resize 60 24; sleep 0.8; cap | grep -q '╭ Sessions' && no_panic; check "ws: 60 columns keeps the sidebar (no tree fallback)" $?
snap ws-60x24
resize 120 40; sleep 0.8; cap | grep -q '╭ Sessions'; check "ws: 120 columns" $?
snap ws-120x40
resize 80 24; sleep 0.8; cap | grep -q '╭ Sessions' && ! cap | grep -q '╭ Claude 1 ─'; snap ws-80x24; check "ws: 80x24 keeps the sidebar and hides the info block (under 24 rows)" $?
resize 200 60; sleep 0.8; no_panic; check "ws: no panic across the resizes" $?
# reattach: killing the terminal (the drive client) leaves the cockpit and
# the agents running; the next `aiw` attaches to the same dashboard
drive_kill; sleep 1; cockpit_alive; check "reattach: the cockpit survives its client being killed" $?
aiw_start 200 60; sleep 2; grep -q 'attaching to the running dashboard' "$S/log/aiw.stderr"; check "reattach: aiw re-attaches instead of starting a second dashboard" $?
cap | grep -q '╭ Sessions' && aiw_tmux has-session -t aiw-alpha-ws-1 2>/dev/null; check "reattach: the sidebar and the agents are as they were" $?
aiw_quit; aiw_start 200 60; wait_for '╭ Sessions' 8; check "relaunch: the sidebar is the dashboard" $?

echo "=== Part 2c: hooks (DESIGN.md §16)"
# ws-1's Claude agent exited above (/crash); relaunch it and use the pane
select_row 'ws-1' >/dev/null; keys j; sleep 0.4; keys c; sleep 3; hook_prompts; sleep 1
wait_sel_glyph '○' 10 || wait_sel_glyph '●' 5; check "hooks: Claude agent relaunched for the hook checks" $?
focus_agent; keys -l '/perm cargo test'; keys Enter
wait_for 'permission: Bash' 8; check "perm: PermissionRequest hook → ⛔ row says permission: Bash" $?
wait_sel_glyph '⛔' 3; check "perm: the row's glyph is ⛔" $?
focus_sidebar; sleep 0.4; cap | tail -1 | grep -q 'a allow'; check "perm: footer offers a allow / X deny" $?
keys a; wait_agent_for 'permission: allow' 8; check "perm: a → the hook answered allow (fake prints permission: allow)" $?
for i in $(seq 1 50); do cap | grep '▎' | grep -q 'permission: Bash' || break; sleep 0.1; done
! cap | grep '▎' | grep -q 'permission: Bash'; check "perm: the ⛔ reason line is gone after allowing" $?
wait_for '✓ allowed Bash' 5; check "info: the block shows the last permission decision" $?
focus_agent; keys -l '/perm rm -rf build'; keys Enter
wait_for 'permission: Bash' 8; focus_sidebar; sleep 0.4; keys X
wait_agent_for 'permission: deny' 8; check "perm: X → the hook answered deny" $?
focus_agent; keys -l '/perm sleep 1'; keys Enter
wait_for 'permission: Bash' 8; check "perm: third request shows ⛔ again" $?
wait_agent_for 'permission: prompt' 20; check "perm: no answer within permission_wait_seconds → the agent prompts itself" $?
focus_sidebar; sleep 0.5
grep -q '"allowed Bash' "$XDG_STATE_HOME/ai_workstation/debug.log" 2>/dev/null || grep -qE 'permission.*allowed' "$XDG_STATE_HOME/ai_workstation/debug.log"; check "perm: decisions are in the debug log" $?
no_panic; check "perm: no panic" $?
# P4: the status line passed with --settings reports context and cost
agent_type 'hello ctx' 1.5
wait_agent_for '\[status\] Fake 1 · ctx 42%' 8; check "ctx: statusLine from --settings → aiw status-line prints the footer line" $?
cap | grep '▎' | head -1 | grep -q '42%'; check "ctx: the sidebar row shows 42% context" $?
tail -1 "$S/log/claude.args" | grep -q '"--settings"'; check "ctx: Claude was launched with --settings <file>" $?
# P7: queued follow-ups, delivered by the Stop hook one turn at a time
keys e; wait_for 'Queue a follow-up' 3; check "queue: e opens the follow-up dialog for the selected session" $?
keys -l 'first follow-up'; keys Enter; sleep 0.4; keys -l 'second follow-up'; keys Enter; sleep 0.4
cap | grep -q '2\. second follow-up'; check "queue: two items listed in order" $?
keys Escape; sleep 0.6
cap | grep '▎' | grep -q '2 queued'; check "queue: the row says ⏭ 2 queued" $?
cap | grep -q '⏭ 2 queued'; check "info: the block says ⏭ 2 queued too" $?
ws_claude_id=$(python3 - <<PY
import sqlite3; c=sqlite3.connect("$XDG_CONFIG_HOME/ai_workstation/aiw.db")
print(c.execute("select s.id from feature_sessions s join features f on f.id=s.feature_id where f.name='ws-1' and s.kind='claude'").fetchone()[0])
PY
)
aiw automation queue --session "$ws_claude_id" "third from automation" | grep -q '"queued": 3'; check "queue: aiw automation queue adds a third" $?
focus_agent; keys -l '/done'; keys Enter
wait_agent_for 'followup: first follow-up' 8; check "queue: the Stop hook delivers the first follow-up" $?
wait_agent_for "handled 'first follow-up'" 8; check "queue: the agent ran it as its next turn" $?
wait_agent_for 'followup: second follow-up' 8; check "queue: the second follows when that turn ends" $?
wait_agent_for 'followup: third from automation' 8; check "queue: then the automation-queued one" $?
wait_agent_for "handled 'third from automation'" 8; sleep 1.5
focus_sidebar; sleep 0.6; ! cap | grep '▎' | grep -q 'queued'; check "queue: nothing left queued after the last turn" $?
[ "$(agent_cap | grep -c 'followup: ')" = 3 ]; check "queue: exactly three continuations, then the Stop hook stays quiet" $?
aiw_quit; aiw_start 200 60; wait_for '╭ Sessions' 8
select_row 'ws-1' >/dev/null; keys j; sleep 0.4; keys e; wait_for 'Queue a follow-up' 3; keys -l 'after restart'; keys Enter; sleep 0.3; keys Escape; settle
aiw_quit; aiw_start 200 60; wait_for '╭ Sessions' 8; select_row 'ws-1' >/dev/null; keys j; sleep 0.6
for i in $(seq 1 30); do cap | grep '▎' | grep -q '1 queued' && break; sleep 0.1; done
cap | grep '▎' | grep -q '1 queued'; check "queue: a queued follow-up survives an AIW restart" $?
keys e; wait_for 'Queue a follow-up' 3; keys Tab; sleep 0.2; keys d; sleep 0.3; keys Escape; settle
# P5: subagent starts and stops count on the working badge
focus_agent; keys -l '/sub 2'; keys Enter; focus_sidebar
for i in $(seq 1 80); do cap | grep '▎' | grep -q '2 subagents' && break; sleep 0.1; done
cap | grep '▎' | grep -q '2 subagents'; check "sub: two SubagentStart hooks → row says 2 subagents" $?
for i in $(seq 1 120); do cap | grep '▎' | grep -q 'subagent' || break; sleep 0.1; done
! cap | grep '▎' | grep -q 'subagent'; check "sub: SubagentStop hooks and the turn's end clear the count" $?
# §19.1: the newer hook events decorate the row and the info block
sel_has() { local re=$1 secs=${2:-8} i; for i in $(seq 1 $((secs * 10))); do cap | grep '▎' | grep -Eq -- "$re" && return 0; sleep 0.1; done; return 1; }
sel_lacks() { local re=$1 secs=${2:-8} i; for i in $(seq 1 $((secs * 10))); do cap | grep '▎' | grep -Eq -- "$re" || return 0; sleep 0.1; done; return 1; }
agent_type '/compact' 1; sel_has 'compacted'; check "events: PostCompact → the row says compacted" $?
agent_type '/model sonnet-fake' 1; sel_has 'model → sonnet-fake'; check "events: PostModelSwitch → the row says model → sonnet-fake" $?
agent_type '/task 3' 1; agent_type '/taskdone' 1; wait_for 'tasks 1/3' 6; check "events: TaskCreated×3 + TaskCompleted → info block says tasks 1/3" $?
agent_type '/fail' 1.5; sel_has 'turn failed: overloaded'; check "events: StopFailure → ⛔ turn failed: overloaded_error" $?
wait_sel_glyph '⛔' 3; check "events: a failed turn shows ⛔, not ● working" $?
agent_type 'recover after failure' 2; sel_lacks 'turn failed'; check "events: the next turn clears the failure" $?
agent_type '/idle' 1; sel_has '(^|[^a-z])idle'; check "events: Notification idle_prompt → the row says idle" $?
agent_type '/needs' 1; wait_sel_glyph '⛔' 6; check "events: Notification agent_needs_input → ⛔" $?
agent_type 'answer the question' 2; sel_lacks 'question'; check "events: the next turn clears agent_needs_input" $?
agent_type '/denied WebFetch' 1; sel_has 'denied: WebFetch'; check "events: PermissionDenied → the row says denied: WebFetch" $?
agent_type '/bg' 2; sel_has '1 bg · 1 loop'; check "events: Stop with background_tasks + session_crons → the row says 1 bg · 1 loop" $?
grep -q '"PostCompact"' "$XDG_STATE_HOME"/ai_workstation/claude-settings/*.json; check "events: the --settings file wires PostCompact" $?
grep -q 'hook-event.sh' "$XDG_STATE_HOME"/ai_workstation/claude-settings/*.json && test -x "$XDG_CONFIG_HOME/ai_workstation/hooks/hook-event.sh"; check "events: hook-event.sh is installed and referenced" $?
agent_type 'clear background' 2
# Codex: UserPromptSubmit / PreToolUse in .codex/hooks.json show it working
if select_row 'cx' >/dev/null; then
    grep -q 'thinking-start.sh' "$S/repos/alpha/.codex/hooks.json" 2>/dev/null; check "codex: .codex/hooks.json wires UserPromptSubmit → thinking-start" $?
    # cx exited earlier in the run; start it again first
    if sel_glyph '■'; then keys c; sleep 3; hook_prompts; wait_sel_glyph '○' 10; fi
    agent_type '/work' 1.5; wait_sel_glyph '●' 6; check "codex: a submitted prompt shows ● working before notify" $?
    agent_type '/done' 2; for i in $(seq 1 60); do sel_glyph '●' || break; sleep 0.1; done
    ! sel_glyph '●'; check "codex: notify still ends the turn" $?
    select_row 'ws-1' >/dev/null; keys j; sleep 0.4
else
    check "codex: Codex row cx present for the hook check" 1
fi
# prompts handed to an agent are pasted unsent: Ctrl+Space l re-pastes a past
# prompt into the pane and leaves Enter to the user
handled_before=$(agent_cap | grep -c "handled 'hello ctx'")
keys C-Space; sleep 0.3; keys l; wait_for 'hello ctx' 8; check "paste: Ctrl+Space l lists the session's past prompts" $?
for i in $(seq 1 10); do cap | grep '>' | grep -q 'hello ctx' && break; keys j; sleep 0.2; done
keys Enter; sleep 1.5
[ "$(pane_active)" = 1 ]; check "paste: the prompt was pasted and the agent pane focused" $?
agent_cap | grep -q '❯ hello ctx' && [ "$(agent_cap | grep -c "handled 'hello ctx'")" = "$handled_before" ]; check "paste: the text sits on the agent's prompt line, unsent" $?
keys Enter; for i in $(seq 1 80); do [ "$(agent_cap | grep -c "handled 'hello ctx'")" -gt "$handled_before" ] && break; sleep 0.1; done
[ "$(agent_cap | grep -c "handled 'hello ctx'")" -gt "$handled_before" ]; check "paste: Enter in the pane is what sends it" $?
snap ws-pasted
focus_sidebar
# item 6: every agent is told its project, feature, branch and AIW session
grep -q 'AIW context: project alpha, feature' "$S/log/claude.args"; check "ctx-prompt: Claude gets the AIW context paragraph in --append-system-prompt" $?
grep -q 'developer_instructions=.*AIW context: project' "$S/log/codex.args"; check "ctx-prompt: Codex gets it in developer_instructions" $?
grep '"event": "SessionStart"' "$S/log/gemini.hooks" | grep -q 'additionalContext.*AIW context: project gamma'; check "ctx-prompt: Gemini's SessionStart hook returns it as additionalContext" $?
grep '"event": "sessionStart"' "$S/log/cursor-agent.hooks" | grep -q 'additional_context.*AIW context: project plain'; check "ctx-prompt: Cursor's sessionStart hook returns it as additional_context" $?
grep -q 'Mention the branch name in commit messages and PR titles' "$S/log/claude.args"; check "ctx-prompt: the paragraph asks for the branch name in commits and PR titles" $?
# item 7: Claude's hooks ride on --settings, not in the repo's settings.local.json
ws_wt=$(ls -d "$XDG_DATA_HOME"/ai_workstation/worktrees/*/alpha-ws-1 | head -1)
! grep -q 'ai_workstation/hooks' "$ws_wt/.claude/settings.local.json" 2>/dev/null; check "settings: no AIW hook entry in the worktree's .claude/settings.local.json" $?
ls "$XDG_STATE_HOME"/ai_workstation/claude-settings/*.json >/dev/null 2>&1 && python3 - <<PY
import glob, json, sys
ok = all('Stop' in json.load(open(p)).get('hooks', {}) and 'statusLine' in json.load(open(p)) for p in glob.glob("$XDG_STATE_HOME/ai_workstation/claude-settings/*.json"))
sys.exit(0 if ok else 1)
PY
check "settings: every per-session --settings file carries the hooks and the status line" $?
AIW_TMUX_SOCKET=$AIW_SOCK aiw doctor 2>/dev/null | grep -q 'passed at launch with --settings'; check "doctor: reports the --settings mode and the Codex /hooks trust note" $?

# §19.2: names, forks, Claude's roster
# shellcheck disable=SC1091
source "$HERE/identity.sh"
# §19.3: one prompt, several agents
# shellcheck disable=SC1091
source "$HERE/fan_out.sh"
# §19.4: groups, notes, layout presets
# shellcheck disable=SC1091
source "$HERE/organize.sh"
# §19.5: messages to agent sessions
# shellcheck disable=SC1091
source "$HERE/messaging.sh"
# §19.6: Claude's own worktrees through AIW's hook
# shellcheck disable=SC1091
source "$HERE/worktree_hooks.sh"
# §19.7: background Claude sessions
# shellcheck disable=SC1091
source "$HERE/background.sh"
# §19.8: the watchdog (restarts aiw with short windows)
# shellcheck disable=SC1091
source "$HERE/watchdog.sh"
# §19.9: scheduled prompts and commands.
# shellcheck disable=SC1091
source "$HERE/schedules.sh"
# §19.10: landing a feature (real git in sandbox repos).
# shellcheck disable=SC1091
source "$HERE/land.sh"

aiw_quit; check "q: quit at the end" $?
! cockpit_alive && [ "$(view_sessions)" = 0 ]; check "q: cockpit server gone, no view sessions, agents still running" $?
doctor_clean; check "doctor clean after the adversarial run" $?

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
