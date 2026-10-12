# shellcheck shell=bash
# §19.10 landing a feature. Sourced by run.sh after schedules.sh, with the
# cockpit up; uses the run's cap / keys / check / wait_for / wait_gone /
# select_row / close_dialogs / hook_prompts helpers and wd_cfg from
# watchdog.sh.
#
# Real git in sandbox repos, no network. Each scenario has its own repo and
# project (one land per repository at a time):
#   1. lm   (main):  the dashboard: P, preflight, Enter; rebase onto a moved
#                    base, a passing check streamed into the `land` terminal,
#                    the base fast-forwarded; on close the feature, its
#                    worktree and its branch are removed.
#   2. fc   (main):  a failing check (automation): nothing merged, ⛔ row.
#   3. rc   (main):  a rebase conflict (automation): the rebase stays in
#                    progress; P shows it and `a` aborts it.
#   4. dt   (main):  a dirty worktree is refused (automation and dialog).
#   5. mb   (main):  the base moves during the land (the check commits to it):
#                    the land stops, nothing merged.
#   6. tk   (trunk): a repository whose default branch is `trunk`.
#   7. pr   (main):  finish pr: push to a local bare remote and a fake `gh`;
#                    a second land reports the already-open PR.

echo "=== Part 2l: landing a feature (DESIGN.md §19.10)"

LR="$S/land-repos"
mkdir -p "$LR"
lg() { git -c user.name=e2e -c user.email=e2e@example.invalid -c commit.gpgsign=false "$@"; }
land() { (cd "$S" && AIW_TMUX_SOCKET=$AIW_SOCK aiw automation land "$@") 2>&1; }
lj() { python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print(eval(sys.argv[2]))' "$1" "$2" 2>/dev/null; }

# mk_land <name> <base> <aiw.json land block as JSON>: a repo with one
# commit on <base>, registered as project <name>, and a worktree feature
# <name>-feat with one commit of its own. Prints nothing; sets LW (worktree).
mk_land() {
    local name=$1 base=$2 block=$3 repo="$LR/$1"
    rm -rf "$repo"; mkdir -p "$repo"
    lg -C "$repo" init -q -b "$base"
    echo "hello" >"$repo/README.md"
    printf '{"land": %s}\n' "$block" >"$repo/aiw.json"
    lg -C "$repo" add -A; lg -C "$repo" commit -qm init
    cat >"$S/land-proj.json" <<JSON
{"path": "$repo", "project_name": "$name"}
JSON
    (cd "$S" && AIW_TMUX_SOCKET=$AIW_SOCK aiw automation create-project --file "$S/land-proj.json") >"$S/land-proj.out" 2>&1
    cat >"$S/land-feat.json" <<JSON
{"project_name": "$name", "branch": "$name-feat", "agents": ["claude"], "use_worktree": true, "mode": "vibe"}
JSON
    (cd "$S" && AIW_TMUX_SOCKET=$AIW_SOCK aiw automation create-feature --file "$S/land-feat.json") >"$S/land-feat.out" 2>&1
    sleep 1.5; hook_prompts; close_dialogs
    land --feature "$name-feat" --dry-run >"$S/land-dry.json"
    LW=$(lj "$S/land-dry.json" 'd["preflight"]["workdir"]')
    if [ -d "$LW" ]; then
        echo "feature work" >"$LW/feature.txt"
        lg -C "$LW" add feature.txt; lg -C "$LW" commit -qm "feature work"
    fi
}

aiw_quit
wd_cfg max_concurrent_agents=60
aiw_start 200 60; wait_for '╭ Sessions|No projects yet' 10; check "land: aiw restarted with room for more agents" $?

# ── 1. local merge from the dashboard ───────────────────────────
mk_land lm main '{"check_command": "test -f feature.txt && test -f base.txt && echo LAND-CHECK-OK"}'
[ -d "$LW" ]; check "land: lm feature worktree exists" $?
lm_wt=$LW
echo "base work" >"$LR/lm/base.txt"; lg -C "$LR/lm" add base.txt; lg -C "$LR/lm" commit -qm "base work"
select_row 'lm-feat' >/dev/null
keys P; wait_for 'Land lm-feat' 5; check "land: P opens the land dialog" $?
wait_for 'Plan:' 10; check "land: the preflight fills in" $?
cap | grep -q 'lm-feat → main'; check "land: the dialog names branch → base" $?
cap | grep -q '1 ahead · 1 behind main'; check "land: ahead / behind" $?
cap | grep -q 'git rebase main'; check "land: the plan rebases onto main" $?
snap land-preflight
keys Enter
wait_for '✓ landed' 25; check "land: the land succeeds" $?
snap land-landed
[ "$(lg -C "$LR/lm" show main:feature.txt 2>/dev/null)" = "feature work" ]; check "land: main now holds the feature's commit" $?
[ "$(lg -C "$LR/lm" rev-list --count main)" = 3 ]; check "land: rebased (linear: init, base, feature)" $?
[ -z "$(lg -C "$LR/lm" status --porcelain --untracked-files=no)" ]; check "land: the main checkout followed (clean)" $?
grep -q 'LAND-CHECK-OK' "$XDG_STATE_HOME"/ai_workstation/land/*.log; check "land: the check's output is in the land log" $?
land_panes() { aiw_tmux list-panes -a -F '#{pane_id}' 2>/dev/null | while read -r p; do aiw_tmux capture-pane -p -J -S -300 -t "$p" 2>/dev/null; done; }
land_panes | grep -q 'LAND-CHECK-OK'; check "land: the feature's land terminal showed the check streaming" $?
keys Escape
# (toasts can name the feature for a while; look at the row itself)
wait_gone '[▾▸] .{0,3}lm-feat' 25; check "land: on close, the landed feature is removed" $?
[ "$(lj "$(land --feature lm-feat --status >"$S/land-lm-st.json"; echo "$S/land-lm-st.json")" 'd["state"]')" = succeeded ]
check "land: status still answers for the removed feature" $?
sleep 1
[ ! -d "$lm_wt" ]; check "land: its worktree is gone" $?
[ -z "$(lg -C "$LR/lm" branch --list lm-feat)" ]; check "land: its branch is deleted" $?
close_dialogs

# ── 2. a failing check ──────────────────────────────────────────
mk_land fc main '{"check_command": "echo E2E-CHECK-FAIL; exit 1"}'
fc_main=$(lg -C "$LR/fc" rev-parse main)
land --feature fc-feat >"$S/land-fc.json"
[ "$(lj "$S/land-fc.json" 'd["state"]')" = failed ]; check "land: a failing check fails the land" $?
[ "$(lj "$S/land-fc.json" 'd["result"]["failure"]["reason"]')" = "check failed (exit 1)" ]; check "land: the reason says check failed (exit 1)" $?
[ "$(lg -C "$LR/fc" rev-parse main)" = "$fc_main" ]; check "land: nothing was merged" $?
select_row 'fc-feat' >/dev/null
wait_for '⛔ land failed: check failed' 5; check "land: the row shows ⛔ land failed" $?
snap land-failed-row
keys i; wait_for 'Needs Attention' 3 && wait_for 'fc-feat' 3; check "land: the failed land is listed in i" $?
cap | grep 'fc-feat' | grep -q 'land failed'; check "land: the i row says land failed" $?
close_dialogs

# ── 3. a rebase conflict, aborted from the dialog ───────────────
mk_land rc main '{}'
rc_wt=$LW
echo "base version" >"$LR/rc/feature.txt"; lg -C "$LR/rc" add feature.txt; lg -C "$LR/rc" commit -qm "conflicting base"
land --feature rc-feat >"$S/land-rc.json"
[ "$(lj "$S/land-rc.json" 'd["result"]["failure"]["kind"]')" = conflict ]; check "land: a conflict stops the land" $?
[ -d "$(lg -C "$rc_wt" rev-parse --git-path rebase-merge)" ] || [ -d "$rc_wt/$(lg -C "$rc_wt" rev-parse --git-path rebase-merge)" ]
check "land: the rebase is left in progress" $?
select_row 'rc-feat' >/dev/null
keys P; wait_for 'a aborts the rebase' 10; check "land: the dialog offers to abort the rebase" $?
keys a; wait_for 'aborted the rebase' 5; check "land: a aborts it" $?
rm_path=$(lg -C "$rc_wt" rev-parse --git-path rebase-merge)
[ ! -e "$rm_path" ] && [ ! -e "$rc_wt/$rm_path" ]; check "land: no rebase in progress any more" $?
close_dialogs

# ── 4. a dirty worktree is refused ──────────────────────────────
mk_land dt main '{}'
echo "uncommitted" >>"$LW/feature.txt"
land --feature dt-feat >"$S/land-dt.json"
[ "$(lj "$S/land-dt.json" 'd["ok"]')" = False ] && grep -q 'uncommitted work' "$S/land-dt.json"
check "land: automation refuses a dirty worktree" $?
select_row 'dt-feat' >/dev/null
keys P; wait_for 'uncommitted work' 10; check "land: the dialog shows the blocker" $?
keys Enter; sleep 0.3
cap | grep -q 'blocked'; check "land: Enter does not land a blocked feature" $?
close_dialogs

# ── 5. the base moves during the land ───────────────────────────
mk_land mb main "{\"check_command\": \"git -C $LR/mb -c user.name=e2e -c user.email=e2e@example.invalid commit -q --allow-empty -m race\"}"
mb_feat=$(lg -C "$LW" rev-parse HEAD)
land --feature mb-feat >"$S/land-mb.json"
[ "$(lj "$S/land-mb.json" 'd["result"]["failure"]["kind"]')" = base_moved ]; check "land: a base that moved stops the land" $?
[ "$(lg -C "$LR/mb" log -1 --format=%s main)" = race ]; check "land: main keeps the racing commit, not the feature" $?

# ── 6. a trunk-based repository ─────────────────────────────────
mk_land tk trunk '{}'
land --feature tk-feat --dry-run >"$S/land-tk-dry.json"
[ "$(lj "$S/land-tk-dry.json" 'd["preflight"]["base"]')" = trunk ]; check "land: the base is trunk" $?
land --feature tk-feat >"$S/land-tk.json"
[ "$(lj "$S/land-tk.json" 'd["state"]')" = succeeded ]; check "land: trunk lands" $?
[ "$(lg -C "$LR/tk" show trunk:feature.txt 2>/dev/null)" = "feature work" ]; check "land: trunk holds the feature" $?
close_dialogs

# ── 7. finish pr with a fake gh and a local bare remote ─────────
cat >"$S/bin/gh" <<'GH'
#!/bin/sh
echo "$*" >>"$HOME/log/gh.calls"
case "$1 $2" in
    "auth status") exit 0 ;;
    "pr create")
        if [ -f "$HOME/log/gh.pr-exists" ]; then
            echo 'a pull request for branch "pr-feat" into branch "main" already exists:' >&2
            echo 'https://github.com/e2e/pr/pull/41' >&2
            exit 1
        fi
        echo 'https://github.com/e2e/pr/pull/42' ;;
    *) echo '[]' ;;
esac
GH
chmod +x "$S/bin/gh"
mk_land pr main '{"finish": "pr"}'
lg init -q --bare "$LR/pr-remote.git"
lg -C "$LR/pr" remote add origin "$LR/pr-remote.git"
lg -C "$LR/pr" push -q origin main; lg -C "$LR/pr" fetch -q origin
land --feature pr-feat >"$S/land-pr.json"
[ "$(lj "$S/land-pr.json" 'd["state"]')" = succeeded ]; check "land: finish pr succeeds" $?
[ "$(lj "$S/land-pr.json" 'd["result"]["pr_url"]')" = "https://github.com/e2e/pr/pull/42" ]; check "land: the PR URL comes back" $?
[ "$(lg -C "$LR/pr-remote.git" rev-parse refs/heads/pr-feat 2>/dev/null)" = "$(lg -C "$LW" rev-parse HEAD)" ]; check "land: the branch was pushed" $?
grep -q 'pr create --base main --head pr-feat --fill' "$S/log/gh.calls"; check "land: gh pr create got --base main --head pr-feat" $?
[ "$(lg -C "$LR/pr" rev-parse main)" != "$(lg -C "$LW" rev-parse HEAD)" ]; check "land: a PR finish leaves main alone" $?
[ -d "$LW" ]; check "land: the feature stays (pr keeps the worktree)" $?
touch "$S/log/gh.pr-exists"
land --feature pr-feat >"$S/land-pr2.json"
[ "$(lj "$S/land-pr2.json" 'd["result"]["pr_existed"]')" = True ] && [ "$(lj "$S/land-pr2.json" 'd["result"]["pr_url"]')" = "https://github.com/e2e/pr/pull/41" ]
check "land: an open PR is reported, not duplicated" $?
rm -f "$S/bin/gh"
close_dialogs
