# shellcheck shell=bash
# §19.8 the watchdog. Sourced by run.sh after background.sh, with the cockpit
# up; reuses msg_q / session_row / pane_of / wait_pane from messaging.sh and
# the run's select_row / cap / keys / check / wait_for / close_dialogs /
# hook_prompts helpers.
#
# aiw reads its config at start, so it is restarted with a 6 s stuck window
# and a 3 s queued-idle grace. Then, on one shared Claude + Codex feature in
# alpha (`wd`):
#   1. /hang: the claude's turn hangs — no hooks, no output, only the status
#      line's spinner and seconds tick in place. After the window the feature
#      says stuck and `i` lists it; `!` → Restart ends the agent and relaunches
#      it with --resume <its conversation>;
#   2. /exit: the claude leaves its pane at the shell; `!` → Restart relaunches
#      it, resuming;
#   3. /ctx 92 at the prompt: "ctx 92% at the prompt"; `!` → Send /compact
#      types it into the claude exactly once;
#   4. a follow-up queued for the idle codex: flagged after the grace, nothing
#      is typed until `!` → Deliver, which the codex runs exactly once.

echo "=== Part 2j: the watchdog (DESIGN.md §19.8)"

wd_cfg() { python3 - "$@" <<'PY'
import json, os, sys
p = os.path.join(os.environ["XDG_CONFIG_HOME"], "ai_workstation", "config.json")
c = json.load(open(p))
for kv in sys.argv[1:]:
    k, v = kv.split("=", 1)
    c[k] = json.loads(v)
json.dump(c, open(p, "w"), indent=2)
PY
}
# lines of <agent>.input holding exactly <line>
wd_inputs() { grep -cxF -- "{\"line\": \"$2\"}" "$S/log/$1.input" 2>/dev/null || true; }
# launches of the fake claude that resumed <conversation>
wd_resumes() { grep -F -- "$1" "$S/log/claude.args" 2>/dev/null | grep -c -- '--resume' || true; }
wd_send() { aiw_tmux send-keys -t "$1" -l "$2"; aiw_tmux send-keys -t "$1" Enter; }
# banners the fake printed in a pane so far (old ones stay in its scrollback)
wd_banners() { pane_of "$1" | grep -c 'fake e2e agent' || true; }
# wd_relaunched <target> <banners before> [secs]: a new agent came up there
wd_relaunched() {
    local i
    for i in $(seq 1 $(( ${3:-15} * 10 ))); do
        [ "$(wd_banners "$1")" -gt "$2" ] && sleep 1 && return 0
        sleep 0.1
    done
    return 1
}
# wd_run <regex>: in the open Recover dialog, run the action matching <regex>
# by its number; with no such action, close the dialog so later parts are not
# blocked by it.
wd_run() {
    local n
    n=$(cap | grep -E -- "$1" | head -1 | grep -oE '│ +[0-9] ' | grep -oE '[0-9]')
    if [ -n "$n" ]; then keys "$n"; else keys Escape; fi
    settle
}

aiw_quit
wd_cfg watchdog_stuck_minutes=0.1 watchdog_queued_idle_secs=3
grep -q '"watchdog_stuck_minutes": 0.1' "$XDG_CONFIG_HOME/ai_workstation/config.json"
check "wd: config takes a fractional stuck window" $?
aiw_start 200 60; wait_for '╭ Sessions' 10; check "wd: aiw restarted with the short windows" $?

cat >"$S/wd.json" <<JSON
{"project_name": "alpha", "branch": "wd", "agents": ["claude", "codex"],
 "fan_out": "shared", "use_worktree": true, "mode": "vibe"}
JSON
(cd "$S" && AIW_TMUX_SOCKET=$AIW_SOCK aiw automation create-feature --file "$S/wd.json") >"$S/wd.out" 2>&1
grep -Eq '"ok": ?true' "$S/wd.out"; check "wd: a shared Claude + Codex feature (wd)" $?
sleep 2; hook_prompts; close_dialogs
read -r wd_claude wd_csess wd_cwin <<<"$(session_row wd claude)"
read -r wd_codex wd_xsess wd_xwin <<<"$(session_row wd codex)"
WD_C="$wd_csess:$wd_cwin"; WD_X="$wd_xsess:$wd_xwin"
wait_pane "$WD_C" 'fake e2e agent' 15 && wait_pane "$WD_X" 'fake e2e agent' 15
check "wd: both agents are up" $?
# the conversation the agent itself shows in its banner
wd_conv=$(pane_of "$WD_C" | grep -oE 'session [0-9a-f-]{36}' | head -1 | cut -d' ' -f2)
[ -n "$wd_conv" ]; check "wd: the claude's conversation id is known ($wd_conv)" $?

# ── 1. a hung turn ──────────────────────────────────────────────
select_row 'wd' >/dev/null
resumes0=$(wd_resumes "$wd_conv")
banners0=$(wd_banners "$WD_C")
wd_send "$WD_C" '/hang'
wait_pane "$WD_C" 'Working \([0-9]+s' 5; check "wd: the claude's turn hangs (status line ticking)" $?
! cap | grep -q 'stuck ·'; check "wd: not stuck before the window" $?
wait_for 'stuck · no hooks or output' 30; check "wd: after the window the row says stuck" $?
snap wd-stuck
keys i; wait_for 'Needs Attention' 3 && wait_for 'stuck' 3; check "wd: i lists the stuck agent" $?
keys Escape; settle
keys -l '!'; wait_for ' Recover ' 3; check "wd: ! opens the recovery actions" $?
wait_for 'Interrupt the turn' 2 && wait_for 'Restart, resuming' 2; check "wd: a stuck agent offers interrupt and restart" $?
snap wd-recover-dialog
wd_run 'Restart, resuming'
for i in $(seq 1 200); do [ "$(wd_resumes "$wd_conv")" -gt "$resumes0" ] && break; sleep 0.1; done
[ "$(wd_resumes "$wd_conv")" -gt "$resumes0" ]; check "wd: Restart ended the hung agent and relaunched it with --resume <conversation>" $?
wd_relaunched "$WD_C" "$banners0"; check "wd: the relaunched agent is up" $?
sleep 2
! cap | grep -q 'stuck ·'; check "wd: the relaunched agent is not stuck (its old turn's markers are gone)" $?

# ── 2. an exited agent ──────────────────────────────────────────
resumes1=$(wd_resumes "$wd_conv")
banners1=$(wd_banners "$WD_C")
wd_send "$WD_C" '/exit'
# the sweep and the watchdog take a few seconds to notice: ask again until it is listed
wd_listed=1
for i in $(seq 1 30); do
    keys i; sleep 0.6
    if cap | grep -q 'Needs Attention' && cap | grep -q 'exited · ! recover'; then wd_listed=0; break; fi
    cap | grep -q 'Needs Attention' && keys Escape
    sleep 0.4
done
[ "$wd_listed" = 0 ]; check "wd: the exited claude is in the needs-you list" $?
snap wd-needs-you-exited
keys Escape; settle
keys -l '!'; wait_for ' Recover ' 3 && wait_for 'Restart, resuming' 2; check "wd: ! offers the restart of the exited agent" $?
wd_run 'Restart, resuming'
for i in $(seq 1 100); do [ "$(wd_resumes "$wd_conv")" -gt "$resumes1" ] && break; sleep 0.1; done
[ "$(wd_resumes "$wd_conv")" -gt "$resumes1" ]; check "wd: Restart relaunched the exited claude resuming its conversation" $?
wd_relaunched "$WD_C" "$banners1"; check "wd: the relaunched agent is up again" $?

# ── 3. context critical at the prompt ───────────────────────────
compacts0=$(wd_inputs claude '/compact')
wd_send "$WD_C" '/ctx 92'
wait_for 'ctx 92% at the prompt' 15; check "wd: idle past the critical threshold, the row says so" $?
keys -l '!'; wait_for ' Recover ' 3 && wait_for 'Send /compact' 2; check "wd: ! offers /compact" $?
wd_run 'Send /compact'
wait_pane "$WD_C" 'Compacted the conversation' 8; check "wd: the claude compacted" $?
sleep 1.5
[ "$(wd_inputs claude '/compact')" = $((compacts0 + 1)) ]; check "wd: /compact was typed exactly once" $?
for i in $(seq 1 40); do cap | grep -q 'ctx 92% at the prompt' || break; sleep 0.25; done
! cap | grep -q 'ctx 92% at the prompt'; check "wd: the flag clears once the context is small again" $?

# ── 4. a follow-up queued for the idle codex ────────────────────
AIW_TMUX_SOCKET=$AIW_SOCK aiw automation queue --session "$wd_codex" 'E2E-WD-QUEUED lint it' >"$S/wd-queue.out" 2>&1
grep -Eq '"ok": ?true' "$S/wd-queue.out"; check "wd: a follow-up queued for the idle codex" $?
wait_for 'queued for an idle agent' 15; check "wd: after the grace the row says it waits on an idle agent" $?
[ "$(wd_inputs codex 'E2E-WD-QUEUED lint it')" = 0 ]; check "wd: nothing is typed by itself" $?
keys -l '!'; wait_for ' Recover ' 3 && wait_for 'Deliver the first queued' 2; check "wd: ! offers to deliver it now" $?
wd_run 'Deliver the first queued'
wait_pane "$WD_X" "handled 'E2E-WD-QUEUED lint it'" 10; check "wd: the codex ran it" $?
sleep 2
[ "$(wd_inputs codex 'E2E-WD-QUEUED lint it')" = 1 ]; check "wd: exactly once" $?
[ "$(msg_q "select count(*) from aiw_followups where session_id = '$wd_codex'")" = 0 ]; check "wd: the queue is empty" $?

no_panic; check "wd: no panic" $?
close_dialogs
