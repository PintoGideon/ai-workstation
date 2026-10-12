# shellcheck shell=bash
# §19.2 session identity: harness names, conversation forks and Claude's own
# roster (`claude agents --json`). Sourced by run.sh at the end of Part 2c,
# with the cockpit up, alpha/ws-1's Claude holding a conversation, and the
# helpers sel_has / sel_lacks / agent_type / ws_wt defined.

echo "=== Part 2d: session identity (DESIGN.md §19.2)"

# names: every Claude launch carries a stable --name; a resume keeps it
grep -q '"--name", "alpha-1-claude-1"' "$S/log/claude.args"; check "identity: Claude launched with --name <feature>-<label>" $?
grep '"--resume"' "$S/log/claude.args" | grep -q '"--name", "alpha-1-claude-1"'; check "identity: a resumed launch keeps the same name" $?
python3 - "$S/log/claude.args" <<'PY'
import json, sys, collections
owners = collections.defaultdict(set)
for line in open(sys.argv[1]):
    r = json.loads(line)
    a = r["args"]
    if "--name" in a:
        owners[a[a.index("--name") + 1]].add(r["env"].get("AIW_FEATURE_SESSION_ID"))
sys.exit(0 if owners and all(len(ids) == 1 for ids in owners.values()) else 1)
PY
check "identity: each name belongs to one AIW session (relaunches keep it)" $?

# fork into the same feature
select_row 'ws-1' >/dev/null; keys j; sleep 0.4
keys f; wait_for "Fork '" 4; check "f: the fork dialog opens on a Claude session with a conversation" $?
cap | grep -q 'Same feature, new window' && cap | grep -q 'New feature worktree'; check "f: the dialog offers both targets" $?
forks_before=$( { wc -l <"$S/log/claude.forks"; } 2>/dev/null || echo 0)
keys Enter; sleep 2.5; hook_prompts; focus_sidebar
wait_for 'Claude 1 fork' 8; check "f → same feature: a 'Claude 1 fork' row appears" $?
tail -1 "$S/log/claude.args" | grep -q '"--fork-session"' && tail -1 "$S/log/claude.args" | grep -q '"--resume"'; check "f → same feature: launched with --resume <source> --fork-session" $?
[ "$( { wc -l <"$S/log/claude.forks"; } 2>/dev/null || echo 0)" -gt "$forks_before" ] \
    && tail -1 "$S/log/claude.forks" | python3 -c 'import json,sys; r=json.loads(sys.stdin.read()); sys.exit(0 if r["from"] != r["to"] else 1)'
check "f → same feature: the fork is a new conversation, the source untouched" $?
tail -1 "$S/log/claude.forks" | grep -q '"name": "ws-1-claude-1-fork"'; check "f → same feature: the fork carries its own name" $?
aiw_tmux list-windows -t aiw-alpha-ws-1 -F '#W' | grep -c claude | grep -q '^[2-9]'; check "f → same feature: a second Claude window in the same tmux session" $?

# fork into a new worktree through F's flow
select_row 'ws-1' >/dev/null; keys j; sleep 0.4
keys f; wait_for "Fork '" 4 && { keys j; sleep 0.2; keys Enter; }
wait_for 'Fork Feature' 4; check "f → new worktree: opens the feature fork" $?
cap | grep -q "continues '"; check "f → new worktree: the dialog says it continues the conversation" $?
keys Enter; sleep 0.3; keys Enter; sleep 3; hook_prompts; sleep 1; hook_prompts; focus_sidebar
wait_for 'ws-1-fork' 10; check "f → new worktree: a ws-1-fork feature appears" $?
grep '"--fork-session"' "$S/log/claude.args" | grep -q 'ws-1-fork'; check "f → new worktree: its Claude starts with --fork-session in the new checkout" $?

# a Codex fork runs `codex … fork <thread>`
if select_row 'cx' >/dev/null; then
    keys f; sleep 0.6
    if cap | grep -q "Fork 'cx'"; then
        keys Enter; sleep 2.5; hook_prompts; focus_sidebar
        tail -3 "$S/log/codex.args" | grep -q '"fork"'; check "f on Codex: launched with codex … fork <thread id>" $?
    else
        cap | grep -q 'No saved conversation'; check "f on Codex: without a saved thread it says so instead of forking" $?
    fi
    close_dialogs
fi

# Claude's roster: sessions outside AIW in a project's checkouts
cat >"$HOME/.e2e-roster.json" <<JSON
[{"cwd": "$S/repos/alpha/docs", "kind": "interactive", "pid": 999999, "startedAt": 1,
  "sessionId": "ext-tty-1", "name": "outside-alpha", "status": "busy"},
 {"cwd": "$ws_wt/src", "kind": "background", "id": "7c5dcf5d", "startedAt": 2, "state": "blocked",
  "status": "waiting", "waitingFor": "permission prompt", "sessionId": "ext-bg-1", "name": "bg-in-ws"},
 {"cwd": "/somewhere/else", "kind": "interactive", "sessionId": "ext-far", "name": "not-ours"}]
JSON
select_row 'ws-1' >/dev/null
wait_for '↗ bg-in-ws · waiting: permis' 25; check "roster: a background Claude outside AIW shows under its feature with its wait" $?
keys E; wait_for 'Claude sessions outside AIW' 4; check "E: the external-sessions list opens" $?
cap | grep -q 'outside-alpha' && cap | grep -q 'claude attach 7c5dcf5d'; check "E: lists both external sessions and the attach command" $?
! cap | grep -q 'not-ours'; check "E: sessions outside every project are left out" $?
close_dialogs
grep -q '"count"' "$S/log/claude.agents"; check "roster: aiw reads claude agents --json" $?
rm -f "$HOME/.e2e-roster.json"
wait_gone '↗ bg-in-ws' 25; check "roster: external lines clear when the sessions end" $?

# Claude's roster as a second needs-you signal for AIW's own session
select_row 'ws-1' >/dev/null; keys j; sleep 0.4
agent_type '/rosterwait dialog open' 1
sel_has 'dialog open' 25; check "roster: an AIW session waiting per the roster alone (no hook) shows needs-you" $?
wait_sel_glyph '⛔' 3; check "roster: … with ⛔" $?
agent_type '/rosteridle' 1
sel_lacks 'dialog open' 25; check "roster: the wait clears with the roster" $?
! cap | grep -q '↗ ws-1-claude-1'; check "roster: AIW's own sessions never show as external lines" $?
