# shellcheck shell=bash
# §19.3 one prompt, several agents. Sourced by run.sh after identity.sh, with
# the cockpit up and the helpers select_row / cap / keys / check / wait_for /
# hook_prompts / focus_sidebar / close_dialogs defined.
#
# Runs three fan-outs in project alpha with the fake claude and codex:
#   1. worktree-each from the dialog, prompt pasted unsent into both agents;
#   2. shared from the dialog with "send when ready": both agents run it once;
#   3. worktree-each through `aiw automation create-feature` with prompt_file.

echo "=== Part 2e: fan-out (DESIGN.md §17, §19.3)"

# pane <tmux session> <window>: the pane's text (scrollback included)
pane() { aiw_tmux capture-pane -p -J -S -400 -t "$1:$2" 2>/dev/null; }
# count <text> <tmux session> <window>: lines in the pane holding <text>
count_in() { pane "$2" "$3" | grep -cF -- "$1"; }
# wait_count <n> <text> <tmux session> <window> [secs]
wait_count() {
    local want=$1 text=$2 s=$3 w=$4 secs=${5:-15} i
    for i in $(seq 1 $((secs * 4))); do
        [ "$(count_in "$text" "$s" "$w")" -ge "$want" ] && return 0
        sleep 0.25
    done
    return 1
}
# open_fan_out_dialog <branch>: n on alpha, name, worktree yes, harness list
open_fan_out_dialog() {
    select_row 'alpha' >/dev/null
    keys n; wait_for 'New Feature' 5 || return 1
    if cap | grep -q 'Source:'; then keys Enter; sleep 0.2; fi
    wait_for 'Name:' 3 || return 1
    keys -l "$1"; keys Enter; sleep 0.3
    cap | grep -Eq '> *Yes' || keys j
    keys Enter; sleep 0.3
}
# tick_claude_codex: highlight Claude (top), tick it, tick Codex below it
tick_claude_codex() {
    local i
    for i in 1 2 3 4 5; do keys k; done
    keys Space; keys j; keys Space; sleep 0.2
}

# ── 1. worktree-each, prompt unsent ─────────────────────────────
open_fan_out_dialog fo-each
tick_claude_codex
cap | grep -q '\[x\] Claude  (primary)' && cap | grep -q '\[x\] Codex'; check "fan-out: Space ticks Claude (primary) and Codex" $?
keys Enter; keys Enter; keys Enter; sleep 0.4
wait_for 'Fan-out: claude \+ codex' 3; check "fan-out: two ticked harnesses open the layout step" $?
cap | grep -q "feature 'fo-each-claude'" && cap | grep -q "feature 'fo-each-codex'"; check "fan-out: the layout step lists the sibling features it will create" $?
keys Enter; wait_for 'Task prompt for claude \+ codex' 3; check "fan-out: then the task prompt step" $?
cap | grep -q 'Codex has no Vibeless'; check "fan-out: a Vibeless set with Codex says it runs in Vibe" $?
keys -l 'Fan out prompt one'; keys Enter
for _ in 1 2 3; do sleep 1.5; hook_prompts; done; focus_sidebar
wait_for '⇶ fo-each · claude / codex' 10; check "fan-out: the sidebar groups the siblings under one header" $?
aiw_tmux has-session -t aiw-alpha-fo-each-claude 2>/dev/null && aiw_tmux has-session -t aiw-alpha-fo-each-codex 2>/dev/null
check "fan-out: each sibling runs in its own tmux session" $?
ls -d "$XDG_DATA_HOME"/ai_workstation/worktrees/*/alpha-fo-each-claude "$XDG_DATA_HOME"/ai_workstation/worktrees/*/alpha-fo-each-codex >/dev/null 2>&1
check "fan-out: each sibling has its own worktree" $?
wait_count 1 'Fan out prompt one' aiw-alpha-fo-each-claude claude 15; check "fan-out: Claude's pane received the prompt once it was ready" $?
wait_count 1 'Fan out prompt one' aiw-alpha-fo-each-codex codex 25; check "fan-out: Codex's pane received the prompt once it was ready" $?
sleep 2
[ "$(count_in 'Fan out prompt one' aiw-alpha-fo-each-claude claude)" = 1 ] && [ "$(count_in 'Fan out prompt one' aiw-alpha-fo-each-codex codex)" = 1 ]
check "fan-out: each pane got the prompt exactly once" $?
! pane aiw-alpha-fo-each-claude claude | grep -qF "handled 'Fan out prompt one'" && ! pane aiw-alpha-fo-each-codex codex | grep -qF "handled 'Fan out prompt one'"
check "fan-out: unsent — neither agent ran it on its own" $?
grep -q 'session-start' "$S/log/claude.hooks" && grep -q 'session-start' "$S/log/codex.hooks"
check "fan-out: both harnesses reported ready through the SessionStart hook" $?

# v on a sibling tiles the whole group
select_row 'fo-each-codex' >/dev/null; keys v; sleep 2.5
agent_title | grep -q 'tiles: ⇶ fo-each'; check "fan-out: v on a sibling tiles the whole group" $?
focus_agent; keys M-q; sleep 2; focus_sidebar

# ── 2. shared, send when ready ──────────────────────────────────
open_fan_out_dialog fo-shared
tick_claude_codex
keys Enter; keys Enter; keys Enter; sleep 0.4
wait_for 'Fan-out: claude \+ codex' 3 && keys j; sleep 0.2
cap | grep -q "window 'Codex 1' in 'fo-shared'"; check "fan-out: the shared layout puts a window per agent in one feature" $?
keys Enter; wait_for 'Task prompt for' 3
keys -l 'Shared prompt two'; keys C-t; sleep 0.2
cap | grep -q '\[x\] send it to every agent when ready'; check "fan-out: Ctrl+T ticks send when ready" $?
keys Enter
for _ in 1 2 3; do sleep 1.5; hook_prompts; done; focus_sidebar
aiw_tmux list-windows -t aiw-alpha-fo-shared -F '#W' 2>/dev/null | grep -qx claude \
    && aiw_tmux list-windows -t aiw-alpha-fo-shared -F '#W' 2>/dev/null | grep -qx codex
check "fan-out shared: one tmux session with a claude and a codex window" $?
wait_count 1 "handled 'Shared prompt two' (claude)" aiw-alpha-fo-shared claude 20; check "fan-out shared: Claude ran the prompt (sent when ready)" $?
wait_count 1 "handled 'Shared prompt two' (codex)" aiw-alpha-fo-shared codex 30; check "fan-out shared: Codex ran the prompt (sent when ready)" $?
sleep 3
[ "$(count_in "handled 'Shared prompt two'" aiw-alpha-fo-shared claude)" = 1 ] && [ "$(count_in "handled 'Shared prompt two'" aiw-alpha-fo-shared codex)" = 1 ]
check "fan-out shared: each agent ran it exactly once" $?

# ── 3. automation: worktree-each with prompt_file and send ──────
printf 'Auto prompt three\n' >"$S/fo-prompt.txt"
cat >"$S/fo-auto.json" <<JSON
{"project_name": "alpha", "branch": "fo-auto", "agents": ["claude", "codex"],
 "fan_out": "worktree_each", "use_worktree": true, "mode": "vibe",
 "prompt_file": "$S/fo-prompt.txt", "send": true}
JSON
(cd "$S" && AIW_TMUX_SOCKET=$AIW_SOCK aiw automation create-feature --file "$S/fo-auto.json") >"$S/fo-auto.out" 2>&1
python3 - "$S/fo-auto.out" <<'PY'
import json, sys
r = json.load(open(sys.argv[1]))
ok = r.get("ok") and [f["branch"] for f in r.get("features", [])] == ["fo-auto-claude", "fo-auto-codex"]
sys.exit(0 if ok else 1)
PY
check "automation: create-feature with agents + worktree_each answers with both siblings" $?
focus_sidebar
wait_for '⇶ fo-auto · claude / codex' 10; check "automation: the siblings are grouped in the sidebar" $?
wait_count 1 "handled 'Auto prompt three' (claude)" aiw-alpha-fo-auto-claude claude 25; check "automation: Claude ran the prompt from prompt_file" $?
wait_count 1 "handled 'Auto prompt three' (codex)" aiw-alpha-fo-auto-codex codex 30; check "automation: Codex ran the prompt from prompt_file" $?
sleep 2
[ "$(count_in "handled 'Auto prompt three'" aiw-alpha-fo-auto-claude claude)" = 1 ] && [ "$(count_in "handled 'Auto prompt three'" aiw-alpha-fo-auto-codex codex)" = 1 ]
check "automation: each agent ran it exactly once" $?
sqlite3 "$XDG_CONFIG_HOME/ai_workstation/aiw.db" 'SELECT COUNT(*) FROM aiw_pending_prompts' 2>/dev/null | grep -qx 0 \
    || python3 -c "import sqlite3,sys; sys.exit(0 if sqlite3.connect('$XDG_CONFIG_HOME/ai_workstation/aiw.db').execute('SELECT COUNT(*) FROM aiw_pending_prompts').fetchone()[0]==0 else 1)"
check "fan-out: no prompt is left waiting once every agent got its own" $?
no_panic; check "fan-out: no panic" $?
close_dialogs
