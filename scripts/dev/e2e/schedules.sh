# shellcheck shell=bash
# §19.9 scheduled prompts and commands. Sourced by run.sh after watchdog.sh,
# with the cockpit up; reuses msg_q / session_row / pane_of / wait_pane from
# messaging.sh, wd_cfg / wd_inputs / wd_send from watchdog.sh, and the run's
# select_row / cap / keys / check / wait_for / close_dialogs / settle /
# hook_prompts helpers.
#
# aiw reads its config at start, so it is restarted with a 5 s shortest
# interval. Then, on a Claude feature in alpha (`sch`):
#   1. an `every 6s` prompt schedule (automation add) is pasted into the idle
#      claude and sent, once per fire, and fires again at the next interval;
#   2. pause (automation) stops the fires;
#   3. `t` lists it with its ⏲ / ⏸ state, Space resumes and pauses it, h
#      shows the history;
#   4. while the claude waits on a question the fire is skipped ("waiting on
#      you"); once answered, the retry delivers it;
#   5. run now (automation) fires at once;
#   6. a shell schedule opens its terminal and runs the command there; a
#      terminal running something else is skipped as busy;
#   7. automation list / history / delete.

echo "=== Part 2k: schedules (DESIGN.md §19.9)"

sc() { (cd "$S" && AIW_TMUX_SOCKET=$AIW_SOCK aiw automation schedule "$@") 2>&1; }
# sc_get <file> <python expression on d>: read a field of a JSON reply
sc_get() { python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print(eval(sys.argv[2]))' "$1" "$2" 2>/dev/null; }
# sc_hist <id> <outcome>: how many runs of <id> had <outcome>
sc_hist() {
    sc history --id "$1" >"$S/sc-hist.json"
    python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print(sum(1 for r in d.get("runs", []) if r["outcome"] == sys.argv[2]))' "$S/sc-hist.json" "$2" 2>/dev/null || echo 0
}
sc_detail() { # <id> <outcome> → the newest run's detail with that outcome
    sc history --id "$1" >"$S/sc-hist.json"
    python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print(next((r["detail"] for r in d.get("runs", []) if r["outcome"] == sys.argv[2]), ""))' "$S/sc-hist.json" "$2" 2>/dev/null
}
sc_wait() { # <secs> <command…>: until the command succeeds
    local secs=$1 i
    shift
    for i in $(seq 1 $((secs * 4))); do "$@" && return 0; sleep 0.25; done
    return 1
}
sc_pings() { wd_inputs claude 'E2E-SCHED-PING'; }
# predicates for sc_wait (re-evaluated on every try)
pings_above() { [ "$(sc_pings)" -gt "$1" ]; }
hist_at_least() { [ "$(sc_hist "$1" "$2")" -ge "$3" ]; }
detail_is() { [ "$(sc_detail "$1" "$2")" = "$3" ]; }

aiw_quit
wd_cfg schedule_min_interval_secs=5
aiw_start 200 60; wait_for '╭ Sessions' 10; check "sched: aiw restarted with a 5 s shortest interval" $?

cat >"$S/sch.json" <<JSON
{"project_name": "alpha", "branch": "sch", "agents": ["claude"], "use_worktree": true, "mode": "vibe"}
JSON
(cd "$S" && AIW_TMUX_SOCKET=$AIW_SOCK aiw automation create-feature --file "$S/sch.json") >"$S/sch.out" 2>&1
grep -Eq '"ok": ?true' "$S/sch.out"; check "sched: a Claude feature (sch)" $?
sleep 2; hook_prompts; close_dialogs
read -r sc_claude sc_csess sc_cwin <<<"$(session_row sch claude)"
SC_C="$sc_csess:$sc_cwin"
wait_pane "$SC_C" 'fake e2e agent' 15; check "sched: the claude is up" $?

# ── 1. an interval prompt into the idle claude ──────────────────
sc add --session "$sc_claude" --when 'every 6s' --prompt 'E2E-SCHED-PING' >"$S/sc-add.json"
[ "$(sc_get "$S/sc-add.json" 'd["ok"]')" = True ]; check "sched: automation add (every 6s, prompt)" $?
sc_id=$(sc_get "$S/sc-add.json" 'd["schedule"]["id"]')
[ "$(sc_get "$S/sc-add.json" 'd["schedule"]["describe"]')" = "every 6 s" ]; check "sched: stored as every 6 s" $?
pings0=$(sc_pings)
sc_wait 20 pings_above "$pings0"; check "sched: the first fire reached the idle claude" $?
wait_pane "$SC_C" "handled 'E2E-SCHED-PING'" 8; check "sched: it was sent (the claude ran it)" $?
pings1=$(sc_pings)
sc_wait 20 pings_above "$pings1"; check "sched: it fires again at the next interval" $?

# ── 2. pause ────────────────────────────────────────────────────
sc pause --id "$sc_id" >"$S/sc-pause.json"
[ "$(sc_get "$S/sc-pause.json" 'd["paused"]')" = True ]; check "sched: automation pause" $?
sleep 2
paused_at=$(sc_pings)
[ $(( $(sc_hist "$sc_id" delivered) + $(sc_hist "$sc_id" queued) )) = "$paused_at" ]
check "sched: exactly once per fire (fires delivered or queued = lines the claude got)" $?
sleep 13
[ "$(sc_pings)" = "$paused_at" ]; check "sched: paused, it does not fire" $?

# ── 3. the dashboard ────────────────────────────────────────────
select_row 'sch' >/dev/null
keys t; wait_for ' Schedules · ' 3; check "sched: t opens the schedules list" $?
wait_for 'every 6 s' 2 && wait_for '→ Claude 1' 2; check "sched: the list shows the rule and its target" $?
wait_for '⏸' 2 && wait_for 'paused' 2; check "sched: a paused schedule says so" $?
snap sched-list
keys ' '; sleep 0.5
[ "$(sc list --feature sch --project alpha | python3 -c 'import json,sys; print(json.load(sys.stdin)["schedules"][0]["paused"])')" = False ]
check "sched: Space resumes it" $?
keys h; wait_for 'History' 2 && wait_for 'delivered' 2; check "sched: h shows the history" $?
snap sched-history
keys ' '; sleep 0.5
[ "$(sc list --feature sch --project alpha | python3 -c 'import json,sys; print(json.load(sys.stdin)["schedules"][0]["paused"])')" = True ]
check "sched: Space pauses it again" $?
keys Escape; settle
! cap | grep -q ' Schedules · '; check "sched: Esc closes the list" $?

# ── 4. waiting on you ───────────────────────────────────────────
# A question the agent keeps open (its Notification hook raised it): a
# dashboard permission request would time out in this sandbox and let the
# fake carry on at its prompt.
wd_send "$SC_C" '/ask'
wait_pane "$SC_C" 'Do you want to proceed' 8; check "sched: the claude asks a question" $?
sleep 1
before_wait=$(sc_pings)
sc resume --id "$sc_id" >/dev/null
sc_wait 20 hist_at_least "$sc_id" 'skipped: waiting on you' 1; check "sched: the fire is skipped while it waits on you" $?
[ "$(sc_pings)" = "$before_wait" ]; check "sched: nothing was typed into the waiting agent" $?
wd_send "$SC_C" 'yes'
wait_pane "$SC_C" "handled 'yes'" 8; check "sched: answered in the agent's pane" $?
sc_wait 45 detail_is "$sc_id" delivered "after waiting on you"; check "sched: the retry delivered it once it could" $?
sc pause --id "$sc_id" >/dev/null

# ── 5. run now ──────────────────────────────────────────────────
sleep 2
before_run=$(sc_pings)
sc run --id "$sc_id" >"$S/sc-run.json"
[ "$(sc_get "$S/sc-run.json" 'd["outcome"]')" = delivered ]; check "sched: run now delivers at once" $?
sc_wait 8 pings_above "$before_run"; check "sched: the claude got the run-now prompt" $?

# ── 6. a shell schedule ─────────────────────────────────────────
sc add --feature sch --project alpha --when 'every 1h' --command 'echo E2E-SCHED-SHELL-$((40+2))' --target sched-term >"$S/sc-add2.json"
[ "$(sc_get "$S/sc-add2.json" 'd["ok"]')" = True ]; check "sched: automation add (shell, in terminal sched-term)" $?
sc_id2=$(sc_get "$S/sc-add2.json" 'd["schedule"]["id"]')
sc run --id "$sc_id2" >"$S/sc-run2.json"
[ "$(sc_get "$S/sc-run2.json" 'd["outcome"]')" = ran ]; check "sched: run now runs the command" $?
sc_term=$(msg_q "select s.tmux_window from feature_sessions s join features f on f.id = s.feature_id where f.name = 'sch' and s.label = 'sched-term'")
[ -n "$sc_term" ]; check "sched: the sched-term terminal was opened" $?
wait_pane "$sc_csess:$sc_term" 'E2E-SCHED-SHELL-42' 8; check "sched: the command ran in it (its output is there)" $?
aiw_tmux send-keys -t "$sc_csess:$sc_term" -l 'sleep 30'; aiw_tmux send-keys -t "$sc_csess:$sc_term" Enter
sleep 1.5
sc run --id "$sc_id2" >"$S/sc-run3.json"
sc_get "$S/sc-run3.json" 'd["outcome"]' | grep -q 'skipped: busy (sleep)'; check "sched: a terminal running something else is skipped as busy" $?
aiw_tmux send-keys -t "$sc_csess:$sc_term" C-c

# ── 7. list, history, delete ────────────────────────────────────
[ "$(sc list --feature sch --project alpha | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["schedules"]))')" = 2 ]
check "sched: automation list shows both" $?
[ "$(sc_hist "$sc_id2" ran)" = 1 ]; check "sched: the history records the run" $?
sc delete --id "$sc_id2" >"$S/sc-del.json"
[ "$(sc_get "$S/sc-del.json" 'd["deleted"]')" = True ]; check "sched: automation delete" $?
[ "$(sc list --feature sch --project alpha | python3 -c 'import json,sys; print(len(json.load(sys.stdin)["schedules"]))')" = 1 ]
check "sched: one left" $?
sc delete --id "$sc_id" >/dev/null
[ "$(msg_q "select count(*) from aiw_schedules")" = 0 ]; check "sched: deleted from aiw.db too" $?

no_panic; check "sched: no panic" $?
close_dialogs
