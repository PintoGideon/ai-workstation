# shellcheck shell=bash
# §19.6 Claude's own worktrees through AIW's WorktreeCreate / WorktreeRemove
# hook. Sourced by run.sh after messaging.sh, with the cockpit up and
# delta/lay-1 running a fake claude; reuses msg_q / session_row / pane_of /
# wait_pane from messaging.sh and the run's select_row / cap / keys / check /
# wait_for / close_dialogs helpers.
#
# The fake claude does what real Claude does with a WorktreeCreate hook: it
# pipes {"name": …} plus the common fields to the hook from its --settings
# file, takes the last stdout line as the worktree path and cds there
# (`/wt`), or keeps its own directory for a subagent (`/subwt`); `/wtrm`
# runs WorktreeRemove with `worktree_path`.
#
#   1. the session's --settings carries both hooks; the script is installed;
#   2. a subagent's worktree lands in AIW's worktree folder on Claude's branch
#      name, from the feature's HEAD, with the included .env and a port
#      block; recorded with its session; drawn under the feature; `W` lists it;
#   3. a session moving into a worktree (`/wt`) gets one too;
#   4. WorktreeRemove refuses a dirty worktree, removes a clean one with its
#      branch, and the row goes;
#   5. `W` → `t` opens a terminal there with AIW_PORT; `W` → `d` `y` removes;
#   6. `aiw doctor` does not call them orphans.

echo "=== Part 2h: Claude's own worktrees through AIW's hook (DESIGN.md §19.6)"

WT_LOG="$S/log/claude.worktrees"
DELTA="$S/repos/delta"
# An included, git-ignored file the hook must copy (Claude skips
# .worktreeinclude when a hook creates the worktree).
printf '.env\n.worktreeinclude\n' >>"$DELTA/.git/info/exclude"
printf 'SECRET=delta-token\n' >"$DELTA/.env"
printf '.env\n' >"$DELTA/.worktreeinclude"

read -r w_claude w_csess w_cwin <<<"$(session_row lay-1 claude)"
[ -n "$w_claude" ]; check "wt: delta/lay-1 has a Claude session" $?
w_pane="$w_csess:$w_cwin"
claude_say() { aiw_tmux send-keys -t "$w_pane" -l "$1"; aiw_tmux send-keys -t "$w_pane" Enter; }
last_create() { python3 - "$WT_LOG" <<'PY'
import json, sys
rows = [json.loads(l) for l in open(sys.argv[1]) if l.strip()]
rows = [r for r in rows if r.get("op") == "create" and r.get("path")]
print(rows[-1]["path"] if rows else "")
PY
}

# ── 1. the hooks ride on --settings ─────────────────────────────
w_settings="$XDG_STATE_HOME/ai_workstation/claude-settings/$w_claude.json"
w_script=$(python3 - "$w_settings" <<'PY'
import json, sys
h = json.load(open(sys.argv[1]))["hooks"]
c, r = h["WorktreeCreate"][0]["hooks"][0], h["WorktreeRemove"][0]["hooks"][0]
ok = c["args"] == ["create"] and r["args"] == ["remove"] and c["command"] == r["command"]
print(c["command"] if ok else "")
PY
)
[ -n "$w_script" ] && [ "${w_script##*/}" = worktree-hook.sh ]; check "wt: the session's --settings carries WorktreeCreate and WorktreeRemove" $?
[ -x "$w_script" ]; check "wt: the hook script is installed and executable" $?
! grep -q WorktreeCreate "$DELTA/.claude/settings.local.json" 2>/dev/null; check "wt: nothing is written into the repo's settings" $?

# ── 2. a subagent's worktree ────────────────────────────────────
claude_say '/subwt agent-e2e1'
wait_pane "$w_pane" '\[worktree\] subagent /' 15; check "wt: the subagent's WorktreeCreate returned a path" $?
w_sub=$(last_create)
case "$w_sub" in "$XDG_DATA_HOME"/ai_workstation/worktrees/*/claude-agent-e2e1) true ;; *) false ;; esac
check "wt: it is in AIW's worktree folder as claude-agent-e2e1 ($w_sub)" $?
[ "$(git -C "$w_sub" rev-parse --abbrev-ref HEAD 2>/dev/null)" = worktree-agent-e2e1 ]; check "wt: on Claude's branch name worktree-agent-e2e1" $?
lay_head=$(git -C "$(msg_q "select workdir from features where name = 'lay-1'")" rev-parse HEAD 2>/dev/null)
[ -n "$lay_head" ] && [ "$(git -C "$w_sub" rev-parse HEAD^{commit} 2>/dev/null)" = "$lay_head" ]; check "wt: branched from the feature's HEAD" $?
[ "$(cat "$w_sub/.env" 2>/dev/null)" = 'SECRET=delta-token' ]; check "wt: the included .env was copied" $?
grep -Eq '^export AIW_PORT=[0-9]+$' "$w_sub/.aiw/ports.env" 2>/dev/null; check "wt: it has its own port block in .aiw/ports.env" $?
[ -z "$(git -C "$w_sub" status --porcelain 2>/dev/null)" ]; check "wt: AIW's files leave it clean" $?
[ "$(msg_q "select parent_session_id || '|' || origin from aiw_claude_worktrees where path = '$w_sub'")" = "$w_claude|subagent general-purpose" ]
check "wt: recorded with the session that asked and its origin" $?
select_row 'lay-1' >/dev/null
wait_for '⎇ agent-e2e1' 5; check "wt: the sidebar shows it under lay-1" $?
keys W; wait_for 'Worktrees Claude made' 3; check "wt: W lists the feature's Claude worktrees" $?
cap | grep -q 'worktree-agent-e2e1'; check "wt: the list names the branch" $?
keys Escape; sleep 0.3

# ── 3. the session moves into a worktree ────────────────────────
claude_say '/wt e2e-enter'
wait_pane "$w_pane" '\[worktree\] entered /' 15; check "wt: EnterWorktree / --worktree goes through the hook" $?
w_enter=$(last_create)
[ "${w_enter##*/}" = claude-e2e-enter ] && [ -d "$w_enter" ]; check "wt: claude-e2e-enter created" $?
wait_for '⎇ e2e-enter · worktree' 5; check "wt: drawn as a session worktree" $?

# ── 4. WorktreeRemove ───────────────────────────────────────────
echo wip >"$w_enter/scratch.txt"
claude_say "/wtrm $w_enter"
wait_pane "$w_pane" "\\[worktree\\] kept $w_enter" 10; check "wt: a dirty worktree is refused" $?
pane_of "$w_pane" | grep -q 'uncommitted or untracked'; check "wt: and Claude is told why" $?
[ -d "$w_enter" ] && [ -n "$(msg_q "select id from aiw_claude_worktrees where path = '$w_enter'")" ]; check "wt: it stays, recorded" $?
rm -f "$w_enter/scratch.txt"
claude_say "/wtrm $w_enter"
wait_pane "$w_pane" "\\[worktree\\] removed $w_enter" 10; check "wt: a clean one is removed" $?
[ ! -e "$w_enter" ]; check "wt: the directory is gone" $?
! git -C "$DELTA" rev-parse --verify --quiet refs/heads/worktree-e2e-enter >/dev/null; check "wt: its commit-less branch is deleted" $?
[ -z "$(msg_q "select id from aiw_claude_worktrees where path = '$w_enter'")" ]; check "wt: the record is gone" $?
wait_gone '⎇ e2e-enter' 5; check "wt: the row goes" $?

# ── 5. W: a terminal there, then remove ─────────────────────────
select_row 'lay-1' >/dev/null
keys W; wait_for 'Worktrees Claude made' 3
keys t; sleep 1
w_term=$(msg_q "select s.tmux_window from feature_sessions s join features f on f.id = s.feature_id where f.name = 'lay-1' and s.label like '⎇ agent-e2e1'")
[ -n "$w_term" ]; check "wt: t opens a terminal session '⎇ agent-e2e1' in the feature" $?
for i in $(seq 1 30); do [ "$(aiw_tmux display -p -t "aiw-delta-lay-1:$w_term" '#{pane_current_path}' 2>/dev/null)" = "$w_sub" ] && break; sleep 0.2; done
[ "$(aiw_tmux display -p -t "aiw-delta-lay-1:$w_term" '#{pane_current_path}' 2>/dev/null)" = "$w_sub" ]; check "wt: the terminal is in the worktree" $?
aiw_tmux send-keys -t "aiw-delta-lay-1:$w_term" -l 'echo WTPORT=$AIW_PORT'; aiw_tmux send-keys -t "aiw-delta-lay-1:$w_term" Enter
wait_pane "aiw-delta-lay-1:$w_term" '^WTPORT=[0-9]+' 5; check "wt: with the worktree's AIW_PORT" $?
aiw_tmux send-keys -t "aiw-delta-lay-1:$w_term" -l 'cd /'; aiw_tmux send-keys -t "aiw-delta-lay-1:$w_term" Enter
focus_sidebar; select_row 'lay-1' >/dev/null
keys W; wait_for 'Worktrees Claude made' 3
keys d; wait_for 'Only a clean worktree is removed' 3; check "wt: d asks first" $?
keys y; sleep 0.8
[ ! -e "$w_sub" ]; check "wt: y removes it from the dashboard" $?
wait_gone '⎇ agent-e2e1' 5; check "wt: and its row" $?

# ── 6. doctor ───────────────────────────────────────────────────
claude_say '/subwt agent-e2e2'
wait_pane "$w_pane" '\[worktree\] subagent /' 15
w_keep=$(last_create)
! AIW_TMUX_SOCKET=$AIW_SOCK aiw doctor 2>/dev/null | grep -q "${w_keep##*/}"; check "wt: aiw doctor does not report it as an orphan" $?
claude_say "/wtrm $w_keep"; wait_pane "$w_pane" "\\[worktree\\] removed $w_keep" 10

no_panic; check "wt: no panic" $?
close_dialogs
