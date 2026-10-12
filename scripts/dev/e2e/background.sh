# shellcheck shell=bash
# §19.7 background Claude sessions. Sourced by run.sh after worktree_hooks.sh,
# with the cockpit up and delta/lay-1 running; reuses msg_q / session_row /
# pane_of / wait_pane from messaging.sh and the run's select_row / cap / keys
# / check / wait_for / close_dialogs / focus_* helpers.
#
# The fake `claude --bg` starts a detached worker from a scrubbed environment
# plus the `env` block of its --settings file (as Claude's supervisor does),
# so AIW's hooks only reach the dashboard if AIW put its variables there.
#
#   1. `s` → "Claude (background)" → name → first prompt: claude --bg runs
#      with --settings / --name and the prompt; the short id is stored; no
#      tmux window is made;
#   2. the worker's hooks reach AIW through the settings env (badge: bg · done);
#      the info block shows the id, the state and the latest output;
#   3. Enter attaches in the agent pane; typing goes to the session; /detach
#      closes the window and the row is back in the background;
#   4. x stops (bg · stopped), c restarts (claude respawn);
#   5. d asks; claude rm is refused over a dirty checkout, Claude's words are
#      shown; y again removes with the flag Claude printed;
#   6. the New Feature dialog's `b`: a feature whose Claude runs in the
#      background gets no tmux session, and its task prompt goes to --bg.

echo "=== Part 2i: background Claude sessions (DESIGN.md §19.7)"

BG_LOG="$S/log/claude.bg"
bg_db() { msg_q "select b.$1 from aiw_background_sessions b join feature_sessions s on s.id = b.session_id where s.label = '$2'"; }
bg_last() { python3 - "$BG_LOG" "$1" <<'PY'
import json, sys
rows = [json.loads(l) for l in open(sys.argv[1]) if l.strip()] if __import__("os").path.exists(sys.argv[1]) else []
rows = [r for r in rows if r.get("op") == sys.argv[2]]
print(json.dumps(rows[-1]) if rows else "")
PY
}
bg_wait_op() { # <op> <grep regex> [secs]
    local i
    for i in $(seq 1 $(( ${3:-10} * 10 ))); do bg_last "$1" | grep -Eq -- "$2" && return 0; sleep 0.1; done
    return 1
}

# ── 1. the picker ───────────────────────────────────────────────
select_row 'lay-1' >/dev/null
keys s; wait_for 'Start Session' 3
cap | grep -q 'Claude (background)'; check "bg: the picker offers Claude (background)" $?
keys j Enter; wait_for 'Name Session' 3
cap | grep -q 'Claude bg 1'; check "bg: it is named Claude bg 1 by default" $?
keys Enter; wait_for 'Background: Claude bg 1' 3; check "bg: then asks for the first prompt" $?
keys -l 'BG-E2E first prompt'; keys Enter; sleep 0.5
bg_wait_op dispatch 'BG-E2E first prompt' 10; check "bg: claude --bg ran with the prompt" $?
bg_last dispatch | grep -q '"--settings"' && bg_last dispatch | grep -q '"--name", "lay-1-claude-bg-1"'
check "bg: with AIW's --settings and the session's --name" $?
for i in $(seq 1 50); do [ -n "$(bg_db bg_id 'Claude bg 1')" ] && break; sleep 0.1; done
bg_id=$(bg_db bg_id 'Claude bg 1')
[ -n "$bg_id" ] && [ "$bg_id" = "$(bg_last dispatch | python3 -c 'import json,sys; print(json.load(sys.stdin)["id"])')" ]
check "bg: the short id it printed is stored ($bg_id)" $?
read -r bg_sid bg_tsess bg_win <<<"$(msg_q "select s.id || ' ' || f.tmux_session || ' ' || s.tmux_window from feature_sessions s join features f on f.id = s.feature_id where s.label = 'Claude bg 1'")"
[ "$bg_win" = aiw-bg-claude ]; check "bg: the session's window name marks it as background" $?
! aiw_tmux list-windows -t "$bg_tsess" -F '#W' 2>/dev/null | grep -qx aiw-bg-claude; check "bg: no tmux window was made for it" $?

# ── 2. hooks through the settings env; the info block ──────────
grep -- '--aiw-e2e-bg-worker' "$S/log/claude.args" | tail -1 | grep -q "\"AIW_FEATURE_SESSION_ID\": \"$bg_sid\""
check "bg: the worker (started without AIW's environment) got it from the settings env" $?
select_row 'Claude bg 1' >/dev/null
wait_for 'bg · done' 15; check "bg: its hooks and the roster reach the row (bg · done)" $?
wait_for "bg $bg_id · done" 10; check "bg: the info block shows the id and the state" $?
# The sidebar is narrow: the latest output follows the state, cut to fit.
wait_for "bg $bg_id · done · ." 10; check "bg: and the latest output (claude logs)" $?
[ -n "$(bg_last logs)" ]; check "bg: read with claude logs" $?

# ── 3. attach / detach ──────────────────────────────────────────
keys Enter; sleep 1
for i in $(seq 1 30); do aiw_tmux list-windows -t "$bg_tsess" -F '#W' 2>/dev/null | grep -qx aiw-bg-claude && break; sleep 0.2; done
aiw_tmux list-windows -t "$bg_tsess" -F '#W' 2>/dev/null | grep -qx aiw-bg-claude; check "bg: Enter opens the attach window" $?
wait_agent_for "attached to $bg_id" 8; check "bg: the agent pane runs claude attach $bg_id" $?
[ "$(pane_active)" = 1 ]; check "bg: and takes the keyboard" $?
keys -l 'BG-E2E second'; keys Enter
wait_agent_for "handled 'BG-E2E second'" 10; check "bg: typing reaches the background session" $?
keys -l '/detach'; keys Enter; sleep 1
for i in $(seq 1 30); do aiw_tmux list-windows -t "$bg_tsess" -F '#W' 2>/dev/null | grep -qx aiw-bg-claude || break; sleep 0.2; done
! aiw_tmux list-windows -t "$bg_tsess" -F '#W' 2>/dev/null | grep -qx aiw-bg-claude; check "bg: detaching closes the window" $?
focus_sidebar; select_row 'Claude bg 1' >/dev/null
wait_for 'bg · done' 10; snap bg-after-detach
wait_for 'bg · done' 10 && wait_gone "bg $bg_id · done · attached" 6; check "bg: the row is back in the background" $?

# ── 4. stop / restart ───────────────────────────────────────────
keys x; bg_wait_op stop "$bg_id" 8; check "bg: x runs claude stop" $?
wait_for 'bg · stopped' 10; check "bg: the row says stopped" $?
keys c; bg_wait_op respawn "$bg_id" 8; check "bg: c runs claude respawn" $?
wait_for 'bg · (done|working)' 10; snap bg-after-respawn; check "bg: and it runs again" $?

# ── 5. remove: refused, then confirmed ──────────────────────────
bg_wt=$(msg_q "select workdir from features where name = 'lay-1'")
echo wip >"$bg_wt/bg-dirty.txt"
select_row 'Claude bg 1' >/dev/null
keys d; wait_for 'Remove Claude bg 1' 3; check "bg: d asks before claude rm" $?
keys y; wait_for 'Claude refused' 8; check "bg: claude rm is refused over a dirty checkout" $?
cap | grep -q 'uncommitted changes'; check "bg: Claude's own words are shown" $?
[ -n "$(bg_db bg_id 'Claude bg 1')" ]; check "bg: nothing was removed" $?
keys y; sleep 1
for i in $(seq 1 50); do [ -z "$(msg_q "select id from feature_sessions where label = 'Claude bg 1'")" ] && break; sleep 0.1; done
[ -z "$(msg_q "select id from feature_sessions where label = 'Claude bg 1'")" ]; check "bg: y again removes the session" $?
bg_last rm | grep -q '"--force-remove-worktree"'; check "bg: with the flag Claude printed" $?
[ -z "$(msg_q "select session_id from aiw_background_sessions where session_id = '$bg_sid'")" ]; check "bg: and its record" $?
rm -f "$bg_wt/bg-dirty.txt"
close_dialogs

# ── 6. the New Feature dialog's b ───────────────────────────────
select_row 'delta' >/dev/null
keys n; wait_for 'New Feature' 5
if cap | grep -q 'Source:'; then keys Enter; sleep 0.2; fi
wait_for 'Name:' 3; keys -l 'bg-feat'; keys Enter; sleep 0.3
keys Enter; sleep 0.3
for i in 1 2 3 4 5; do keys k; done
keys b; wait_for 'Claude \(background\)' 3; check "bg: b on the harness list makes it Claude (background)" $?
for i in $(seq 1 6); do cap | grep -q 'Task prompt' && break; keys Enter; sleep 0.3; done
keys -l 'BG-E2E feature prompt'; keys Enter; sleep 1.5; hook_prompts
bg_wait_op dispatch 'BG-E2E feature prompt' 10; check "bg: the feature's Claude started with claude --bg and the task prompt" $?
! aiw_tmux has-session -t aiw-delta-bg-feat 2>/dev/null; check "bg: and the feature has no tmux session" $?
[ "$(msg_q "select s.tmux_window from feature_sessions s join features f on f.id = s.feature_id where f.name = 'bg-feat'")" = aiw-bg-claude ]
check "bg: its Claude is the background session" $?
focus_sidebar; select_row 'bg-feat' >/dev/null
# Feature rows carry no reason line: look at its session row.
keys l; sleep 0.2; keys j; sleep 0.3
wait_for 'bg · (done|working)' 15; snap bg-feature-session; check "bg: the feature's background session shows what it does" $?

no_panic; check "bg: no panic" $?
close_dialogs
