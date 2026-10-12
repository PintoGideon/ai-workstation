# shellcheck shell=bash
# §19.5 messages to agent sessions. Sourced by run.sh after organize.sh, with
# the cockpit up, delta/lay-1 running a fake claude (which binds an inbox
# socket and exports it to its hooks like real Claude), alpha/fo-each-codex
# running a fake codex, and the helpers select_row / cap / keys / check /
# wait_for / close_dialogs / settle defined.
#
#   1. the claude's SessionStart hook reported its inbox; aiw stored the
#      socket (never the token);
#   2. `@` on lay-1 sends into that inbox: the fake prints the peer message
#      once and the line carried the token and the conversation id;
#   3. `aiw automation message --to <name> --from ci` does the same by name;
#   4. a Codex target has no inbox: the message is queued and the fake codex
#      gets it exactly once when its turn ends (the Stop hook).

echo "=== Part 2g: messages to agent sessions (DESIGN.md §19.5)"

msg_q() {
    python3 -c "import sqlite3,sys; r=sqlite3.connect('$XDG_CONFIG_HOME/ai_workstation/aiw.db').execute(sys.argv[1]).fetchone(); print('' if r is None else r[0])" "$1" 2>/dev/null
}
session_row() { # <feature> <kind> → "<id> <tmux session> <window>"
    python3 - "$1" "$2" <<PY 2>/dev/null
import sqlite3, sys
c = sqlite3.connect("$XDG_CONFIG_HOME/ai_workstation/aiw.db")
r = c.execute("select s.id, f.tmux_session, s.tmux_window from feature_sessions s join features f on f.id = s.feature_id where f.name = ? and s.kind = ? order by s.sort_order limit 1", (sys.argv[1], sys.argv[2])).fetchone()
print(" ".join(r) if r else "")
PY
}
pane_of() { aiw_tmux capture-pane -p -J -S -300 -t "$1" 2>/dev/null; }
wait_pane() { # <target> <regex> [secs]
    local i
    for i in $(seq 1 $(( ${3:-8} * 10 ))); do pane_of "$1" | grep -Eq -- "$2" && return 0; sleep 0.1; done
    return 1
}
INBOX_LOG="$S/log/claude.inbox"

# ── 1. the inbox is reported ────────────────────────────────────
read -r m_claude m_csess m_cwin <<<"$(session_row lay-1 claude)"
[ -n "$m_claude" ]; check "msg: delta/lay-1 has a Claude session" $?
for i in $(seq 1 50); do [ -n "$(msg_q "select socket from aiw_inbox_addresses where session_id = '$m_claude'")" ] && break; sleep 0.1; done
m_sock=$(msg_q "select socket from aiw_inbox_addresses where session_id = '$m_claude'")
[ -S "$m_sock" ]; check "msg: the SessionStart hook reported the inbox socket and aiw stored it" $?
[ "$(msg_q "select count(*) from pragma_table_info('aiw_inbox_addresses') where name like '%token%'")" = 0 ]
check "msg: the stored inbox has no token column" $?

# ── 2. `@` from the dashboard ───────────────────────────────────
select_row 'lay-1' >/dev/null
keys -l '@'; wait_for ' Message ' 3; check "msg: @ opens the message dialog" $?
wait_for 'via inbox' 3; check "msg: the dialog says the Claude session is reached via its inbox" $?
keys -l 'E2E-DIALOG-MSG check the build'; keys Enter; sleep 0.5
! cap | grep -q ' Message '; check "msg: Enter sends and closes the dialog" $?
wait_pane "$m_csess:$m_cwin" 'Message from @aiw: E2E-DIALOG-MSG check the build' 8
check "msg: the claude received it in its inbox (shown as a peer message)" $?
[ "$(grep -c 'E2E-DIALOG-MSG' "$INBOX_LOG" 2>/dev/null)" = 1 ]; check "msg: exactly once" $?
grep 'E2E-DIALOG-MSG' "$INBOX_LOG" | grep -q '"authed": true'; check "msg: the line was authenticated with the session's token" $?
grep 'E2E-DIALOG-MSG' "$INBOX_LOG" | grep -q '"session_id": "[0-9a-f-]\{36\}"'; check "msg: and named the receiving conversation" $?
[ "$(msg_q "select count(*) from aiw_followups where session_id = '$m_claude'")" = 0 ]; check "msg: nothing was queued for it" $?

# ── 3. automation, by name ──────────────────────────────────────
m_name=$(msg_q "select name from aiw_session_names where session_id = '$m_claude'")
[ -n "$m_name" ]; check "msg: the claude has a harness name ($m_name)" $?
m_out=$(aiw automation message --to "$m_name" --from ci --text 'E2E-AUTO-MSG ship it' 2>&1)
echo "$m_out" | grep -q '"status": "delivered"' && echo "$m_out" | grep -q '"via": "inbox"'
check "msg: aiw automation message by name → delivered via inbox" $?
wait_pane "$m_csess:$m_cwin" 'Message from @ci: E2E-AUTO-MSG ship it' 8; check "msg: the claude shows it from @ci" $?
[ "$(grep -c 'E2E-AUTO-MSG' "$INBOX_LOG" 2>/dev/null)" = 1 ]; check "msg: exactly once" $?
aiw automation message --to nobody-here --text x 2>&1 | grep -q '"status": "failed"'; check "msg: an unknown target fails with a reason" $?

# ── 4. Codex: the follow-up queue ───────────────────────────────
read -r m_codex m_xsess m_xwin <<<"$(session_row fo-each-codex codex)"
[ -n "$m_codex" ]; check "msg: alpha/fo-each-codex has a Codex session" $?
select_row 'fo-each-codex' >/dev/null
keys -l '@'; wait_for ' Message ' 3; wait_for 'via queue' 3; check "msg: the dialog says a Codex session is reached via the queue" $?
keys Escape; sleep 0.3
m_out=$(aiw automation message --to "$m_codex" --text 'E2E-QUEUED-MSG run the tests' 2>&1)
echo "$m_out" | grep -q '"status": "queued"' && echo "$m_out" | grep -q '"via": "followup-queue"'
check "msg: a message to Codex is queued (no inbox)" $?
! pane_of "$m_xsess:$m_xwin" | grep -q 'E2E-QUEUED-MSG'; check "msg: nothing is typed into the idle Codex pane" $?
aiw_tmux send-keys -t "$m_xsess:$m_xwin" -l '/done'; aiw_tmux send-keys -t "$m_xsess:$m_xwin" Enter
wait_pane "$m_xsess:$m_xwin" 'followup: E2E-QUEUED-MSG run the tests' 10; check "msg: the Codex turn end delivers it" $?
wait_pane "$m_xsess:$m_xwin" "handled 'E2E-QUEUED-MSG run the tests'" 10; sleep 1.5
[ "$(pane_of "$m_xsess:$m_xwin" | grep -c 'followup: E2E-QUEUED-MSG')" = 1 ]; check "msg: exactly once" $?
no_panic; check "msg: no panic" $?
close_dialogs
