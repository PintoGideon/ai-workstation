#!/usr/bin/env bash
# Multi-project end-to-end run (DESIGN.md §19.11): every orchestration feature
# of §19.1–§19.10 together, across six differently shaped repositories, in ONE
# aiw instance, against the fake agent CLIs.
#
#   scripts/dev/e2e/multi-project.sh [sandbox-dir] [aiw-binary]
#
# Default sandbox: /var/tmp/aiw-e2e-mp-$USER (keep it short: sockets live under
# it). HOME and XDG_* point at the sandbox, as in run.sh; nothing outside it is
# touched. Prints a PASS/FAIL table; captures land in <sandbox>/captures.
#
# Repositories (under <sandbox>/mp):
#   rusty   a Rust crate                 land check: cargo check --offline
#   nodey   a Node package (npm scripts) land check: npm test (no network)
#   pyth    a Python package             land check: python3 -m tests (fails on purpose)
#   mono    a two-package monorepo       a layout preset; a check per package
#   trunky  default branch `trunk`       fork into a new worktree, land onto trunk
#   remote  a local bare remote          land finish: pr through a fake gh
#
# Interactions checked on purpose: landing a fan-out sibling, deleting a group
# that holds a fan-out pair, a schedule aimed at a forked session (and dropped
# when that feature lands), a message to a background session, the watchdog
# leaving a session that waits on you alone, worktree hooks in two repos at
# once.
set -uo pipefail

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
S_IN=${1:-/var/tmp/aiw-e2e-mp-${USER:-e2e}}
AIW_BIN=${2:-}

rm -rf "$S_IN"
"$HERE/setup-sandbox.sh" "$S_IN" ${AIW_BIN:+"$AIW_BIN"} >/dev/null || exit 1
# shellcheck disable=SC1091
source "$S_IN/env.sh"
# shellcheck disable=SC1091
source "$HERE/lib.sh"
export AIW_CMD="env -u TMUX AIW_PERF=1 aiw"
RESULTS="$S/mp-results.tsv"
: >"$RESULTS"
# rustup and cargo live under the real home; the sandbox HOME would hide them
# from the land check of the Rust repository.
REAL_HOME=$(getent passwd "$(id -u)" | cut -d: -f6)
export RUSTUP_HOME="${RUSTUP_HOME:-$REAL_HOME/.rustup}" CARGO_HOME="${CARGO_HOME:-$REAL_HOME/.cargo}"

cleanup() {
    aiw_quit 2>/dev/null
    cockpit_kill
    tmux -S "$AIW_SOCK" kill-server 2>/dev/null || true
    # detached fake `claude --bg` workers of this sandbox
    local p
    for p in $(pgrep -f -- '--aiw-e2e-bg-worker' 2>/dev/null); do
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

auto() { (cd "$S" && AIW_TMUX_SOCKET=$AIW_SOCK aiw automation "$@") 2>&1; }
jget() { python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); print(eval(sys.argv[2]))' "$1" "$2" 2>/dev/null; }
lg() { git -c user.name=e2e -c user.email=e2e@example.invalid -c commit.gpgsign=false "$@"; }
cfg() { python3 - "$@" <<'PY'
import json, os, sys
p = os.path.join(os.environ["XDG_CONFIG_HOME"], "ai_workstation", "config.json")
c = json.load(open(p))
for kv in sys.argv[1:]:
    k, v = kv.split("=", 1)
    c[k] = json.loads(v)
json.dump(c, open(p, "w"), indent=2)
PY
}
# session_row <feature> <kind> [label]: "<id> <tmux session> <window>"
session_row() {
    python3 - "$1" "$2" "${3:-}" <<PY 2>/dev/null
import sqlite3, sys
c = sqlite3.connect("$XDG_CONFIG_HOME/ai_workstation/aiw.db")
q = ("select s.id, f.tmux_session, s.tmux_window from feature_sessions s join features f on f.id = s.feature_id "
     "where f.name = ? and s.kind = ?")
args = [sys.argv[1], sys.argv[2]]
if sys.argv[3]:
    q += " and s.label = ?"
    args.append(sys.argv[3])
r = c.execute(q + " order by s.sort_order limit 1", args).fetchone()
print(" ".join(r) if r else "")
PY
}
feature_id() { db_q "select id from features where name = '$1'"; }
feature_workdir() { db_q "select workdir from features where name = '$1'"; }
pane_of() { aiw_tmux capture-pane -p -J -S -400 -t "$1" 2>/dev/null; }
wait_pane() { # <target> <regex> [secs]
    local i
    for i in $(seq 1 $(( ${3:-8} * 10 ))); do pane_of "$1" | grep -Eq -- "$2" && return 0; sleep 0.1; done
    return 1
}
count_pane() { pane_of "$1" | grep -cF -- "$2" || true; }
say() { aiw_tmux send-keys -t "$1" -l "$2"; aiw_tmux send-keys -t "$1" Enter; }
sel_has() { local re=$1 secs=${2:-8} i; for i in $(seq 1 $((secs * 10))); do cap | grep '▎' | grep -Eq -- "$re" && return 0; sleep 0.1; done; return 1; }
sel_lacks() { local re=$1 secs=${2:-8} i; for i in $(seq 1 $((secs * 10))); do cap | grep '▎' | grep -Eq -- "$re" || return 0; sleep 0.1; done; return 1; }
# first_session <feature>: select the feature's first session row
first_session() { select_row "$1" >/dev/null; keys l; sleep 0.2; keys j; sleep 0.3; }
inputs() { grep -cxF -- "{\"line\": \"$2\"}" "$S/log/$1.input" 2>/dev/null || true; }
# create_feature <project> <branch> <agents JSON list> [extra JSON fields]
create_feature() {
    cat >"$S/cf-$2.json" <<JSON
{"project_name": "$1", "branch": "$2", "agents": $3, "use_worktree": true, "mode": "vibe" ${4:+, $4}}
JSON
    auto create-feature --file "$S/cf-$2.json" >"$S/cf-$2.out"
    sleep 1.5; hook_prompts; close_dialogs
    grep -Eq '"ok": ?true' "$S/cf-$2.out"
}
# commit_in <dir> <file> <text> <message>
commit_in() { echo "$3" >"$1/$2"; lg -C "$1" add "$2"; lg -C "$1" commit -qm "$4"; }

# ── repositories ─────────────────────────────────────────────────
MP="$S/mp"
mkdir -p "$MP"
mk_repo() { # <name> <base branch>
    local d="$MP/$1"
    rm -rf "$d"; mkdir -p "$d"
    lg -C "$d" init -q -b "$2"
    printf '# %s\n' "$1" >"$d/README.md"
}
seal() { lg -C "$MP/$1" add -A; lg -C "$MP/$1" commit -qm "init $1"; }

mk_repo rusty main
cat >"$MP/rusty/Cargo.toml" <<'T'
[package]
name = "rusty"
version = "0.1.0"
edition = "2021"

[dependencies]
T
mkdir -p "$MP/rusty/src"; printf 'pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n' >"$MP/rusty/src/lib.rs"
printf 'target/\nCargo.lock\n' >"$MP/rusty/.gitignore"
cat >"$MP/rusty/aiw.json" <<'J'
{"land": {"check_command": "cargo check --offline -q && echo RUSTY-CHECK-OK", "finish": "local_merge"}}
J
seal rusty

mk_repo nodey main
cat >"$MP/nodey/package.json" <<'J'
{"name": "nodey", "version": "1.0.0", "private": true,
 "scripts": {"test": "node -e \"process.exit(require('./index.js').add(1, 2) === 3 ? 0 : 1)\" && echo NODEY-CHECK-OK"}}
J
printf 'module.exports = { add: (a, b) => a + b };\n' >"$MP/nodey/index.js"
printf 'node_modules/\n' >"$MP/nodey/.gitignore"
cat >"$MP/nodey/aiw.json" <<'J'
{"land": {"check_command": "npm test --silent", "finish": "local_merge"}}
J
seal nodey

mk_repo pyth main
mkdir -p "$MP/pyth/pyth" "$MP/pyth/tests"
printf 'def add(a, b):\n    return a + b\n' >"$MP/pyth/pyth/__init__.py"
# fails on purpose: the land must stop and merge nothing
printf 'from pyth import add\nassert add(1, 2) == 4, "PYTH-CHECK-FAILED add(1, 2) != 4"\n' >"$MP/pyth/tests/__main__.py"
printf '__pycache__/\n*.pyc\n' >"$MP/pyth/.gitignore"
cat >"$MP/pyth/aiw.json" <<'J'
{"land": {"check_command": "python3 -B -m tests", "finish": "local_merge"}}
J
seal pyth

mk_repo mono main
mkdir -p "$MP/mono/packages/a" "$MP/mono/packages/b" "$MP/mono/scripts"
printf 'a\n' >"$MP/mono/packages/a/VERSION"; printf 'b\n' >"$MP/mono/packages/b/VERSION"
printf '#!/bin/sh\nfor p in packages/*; do test -f "$p/VERSION" || exit 1; done\necho MONO-CHECK-OK\n' >"$MP/mono/scripts/check.sh"
chmod +x "$MP/mono/scripts/check.sh"
cat >"$MP/mono/aiw.json" <<'J'
{
  "custom_sessions": [ { "name": "watch-a", "command": "echo MONO-WATCH-A; sleep 600", "window_name": "watch-a" } ],
  "layout_presets": [
    { "name": "dev", "sessions": [
        { "kind": "terminal", "label": "pkg-b", "command": "cd packages/b && echo MONO-PKG-B-READY" },
        { "kind": "custom", "name": "watch-a" } ] }
  ],
  "land": {"check_command": "sh scripts/check.sh", "finish": "local_merge"}
}
J
seal mono

mk_repo trunky trunk
printf 'trunk work\n' >"$MP/trunky/notes.txt"
seal trunky

mk_repo remote main
printf '{"land": {"finish": "pr"}}\n' >"$MP/remote/aiw.json"
seal remote
lg init -q --bare "$MP/remote.git"
lg -C "$MP/remote" remote add origin "$MP/remote.git"
lg -C "$MP/remote" push -q origin main; lg -C "$MP/remote" fetch -q origin
lg -C "$MP/remote" remote set-head origin main >/dev/null 2>&1 || true

cat >"$S/bin/gh" <<'GH'
#!/bin/sh
echo "$*" >>"$HOME/log/gh.calls"
case "$1 $2" in
    "auth status") exit 0 ;;
    "pr create") echo 'https://github.com/e2e/remote/pull/7' ;;
    *) echo '[]' ;;
esac
GH
chmod +x "$S/bin/gh"

echo "sandbox: $S"
echo "=== MP 0: first run, then the short windows"
aiw_start 200 60
wait_for 'icon check' 10 && keys y
wait_for 'Configure Agent Harnesses' 10 && wait_for '\[x\] Cursor \(installed\)' 10
keys c; wait_for 'No projects yet' 8; check "mp: first run reaches the empty dashboard" $?
aiw_quit
cfg max_concurrent_agents=60 permission_wait_seconds=5 watchdog_stuck_minutes=0.1 \
    watchdog_queued_idle_secs=3 schedule_min_interval_secs=5 fan_out_ready_fallback_secs=8
aiw_start 200 60; wait_for 'No projects yet' 10; check "mp: aiw restarted with the test windows" $?

for p in rusty nodey pyth mono trunky remote; do
    printf '{"path": "%s", "project_name": "%s"}\n' "$MP/$p" "$p" >"$S/proj-$p.json"
    auto create-project --file "$S/proj-$p.json" >"$S/proj-$p.out"
done
sleep 1; close_dialogs
[ "$(db_q "select count(*) from projects")" = 6 ]; check "mp: six projects of different shapes" $?

# ═════════════════════════════════════════════════════════════════
echo "=== MP 1: features in every repository (names, fan-out, layout)"
create_feature rusty r-core '["claude"]'; check "rusty: a Claude feature" $?
create_feature trunky t-main '["claude"]'; check "trunky: a Claude feature on a trunk repo" $?
create_feature remote rem-1 '["claude"]'; check "remote: a Claude feature" $?
create_feature nodey n-api '["claude", "codex"]' '"fan_out": "worktree_each", "prompt": "MP-FANOUT-UNSENT add a route", "send": false'
check "nodey: fan-out claude + codex, one worktree each (automation)" $?
create_feature pyth p-shared '["claude", "codex"]' '"fan_out": "shared", "prompt": "MP-FANOUT-SENT write a test", "send": true'
check "pyth: fan-out claude + codex, shared (automation, send when ready)" $?
create_feature mono m-dev '["claude"]' '"layout": "dev"'; check "mono: a feature with the dev layout preset" $?
sleep 3; hook_prompts; focus_sidebar

read -r RC_ID RC_S RC_W <<<"$(session_row r-core claude)"; RC="$RC_S:$RC_W"
read -r TM_ID TM_S TM_W <<<"$(session_row t-main claude)"; TM="$TM_S:$TM_W"
read -r RM_ID RM_S RM_W <<<"$(session_row rem-1 claude)"; RM="$RM_S:$RM_W"
read -r NC_ID NC_S NC_W <<<"$(session_row n-api-claude claude)"; NC="$NC_S:$NC_W"
read -r NX_ID NX_S NX_W <<<"$(session_row n-api-codex codex)"; NX="$NX_S:$NX_W"
read -r PC_ID PC_S PC_W <<<"$(session_row p-shared claude)"; PC="$PC_S:$PC_W"
read -r PX_ID PX_S PX_W <<<"$(session_row p-shared codex)"; PX="$PX_S:$PX_W"
read -r MC_ID MC_S MC_W <<<"$(session_row m-dev claude)"; MC="$MC_S:$MC_W"
for t in "$RC" "$TM" "$RM" "$NC" "$NX" "$PC" "$PX" "$MC"; do wait_pane "$t" 'fake e2e agent' 15 || echo "agent not up: $t"; done
[ -n "$RC_ID" ] && [ -n "$NX_ID" ] && [ -n "$PX_ID" ] && [ -n "$MC_ID" ]; check "mp: every agent session is known" $?

# §19.2 names: each Claude launched with --name <feature>-<label>, all unique
for n in r-core-claude-1 t-main-claude-1 rem-1-claude-1 n-api-claude-claude-1 p-shared-claude-1 m-dev-claude-1; do
    grep -q "\"--name\", \"$n\"" "$S/log/claude.args" || echo "missing --name $n"
done
python3 - "$S/log/claude.args" <<'PY'
import json, sys, collections
owners = collections.defaultdict(set)
for line in open(sys.argv[1]):
    r = json.loads(line); a = r["args"]
    if "--name" in a:
        owners[a[a.index("--name") + 1]].add(r["env"].get("AIW_FEATURE_SESSION_ID"))
names = set(owners)
want = {"r-core-claude-1", "t-main-claude-1", "rem-1-claude-1", "p-shared-claude-1", "m-dev-claude-1"}
sys.exit(0 if want <= names and all(len(v) == 1 for v in owners.values()) else 1)
PY
check "names: every Claude in every repo has its own --name" $?

# §19.3 fan-out delivery across two repositories
wait_pane "$NC" 'MP-FANOUT-UNSENT add a route' 20 && wait_pane "$NX" 'MP-FANOUT-UNSENT add a route' 25
check "fan-out (nodey): both siblings received the prompt" $?
sleep 2
[ "$(count_pane "$NC" 'MP-FANOUT-UNSENT')" = 1 ] && [ "$(count_pane "$NX" 'MP-FANOUT-UNSENT')" = 1 ] \
    && ! pane_of "$NC" | grep -qF "handled 'MP-FANOUT-UNSENT" && ! pane_of "$NX" | grep -qF "handled 'MP-FANOUT-UNSENT"
check "fan-out (nodey): exactly once each, left unsent" $?
wait_pane "$PC" "handled 'MP-FANOUT-SENT write a test' \(claude\)" 25 && wait_pane "$PX" "handled 'MP-FANOUT-SENT write a test' \(codex\)" 30
check "fan-out (pyth, shared): both agents ran the prompt (sent when ready)" $?
sleep 2
[ "$(count_pane "$PC" "handled 'MP-FANOUT-SENT")" = 1 ] && [ "$(count_pane "$PX" "handled 'MP-FANOUT-SENT")" = 1 ]
check "fan-out (pyth, shared): exactly once each" $?
wait_for '⇶ n-api · claude / codex' 8; check "fan-out (nodey): the sidebar groups the siblings" $?
# clear the unsent text from the nodey panes so later turns read cleanly
for t in "$NC" "$NX"; do aiw_tmux send-keys -t "$t" C-u; done

# §19.4 layout preset in the monorepo
mw=$(aiw_tmux list-windows -t "$MC_S" -F '#W' 2>/dev/null)
echo "$mw" | grep -qx claude && echo "$mw" | grep -qx watch-a && [ "$(echo "$mw" | wc -l)" -ge 3 ]
check "layout (mono): claude + the pkg-b terminal + the watch-a custom session" $?
seen=0; for w in $mw; do pane_of "$MC_S:$w" | grep -q '^MONO-PKG-B-READY' && seen=1; done
[ "$seen" = 1 ]; check "layout (mono): the terminal ran its command in packages/b" $?
wait_pane "$MC_S:watch-a" 'MONO-WATCH-A' 8; check "layout (mono): the custom session runs" $?

# ═════════════════════════════════════════════════════════════════
echo "=== MP 2: row events, fork both ways, the roster"
say "$RC" 'MP first turn in rusty'
wait_pane "$RC" "handled 'MP first turn in rusty'" 10
first_session r-core
say "$RC" '/compact'; sel_has 'compacted' 10; check "events (rusty): PostCompact → the row says compacted" $?
say "$RC" '/task 2'; say "$RC" '/taskdone'; wait_for 'tasks 1/2' 8; check "events (rusty): tasks 1/2 in the info block" $?
first_session t-main
say "$TM" '/model fake-sonnet'; sel_has 'model → fake-sonnet' 10; check "events (trunky): PostModelSwitch on another repo's row" $?

# fork into the same feature (rusty)
first_session r-core
keys f; wait_for "Fork '" 5; check "fork (rusty): f opens the fork dialog" $?
keys Enter; sleep 2.5; hook_prompts; focus_sidebar
wait_for 'Claude 1 fork' 10; check "fork (rusty): 'Claude 1 fork' in the same feature" $?
read -r RF_ID RF_S RF_W <<<"$(session_row r-core claude 'Claude 1 fork')"; RF="$RF_S:$RF_W"
[ -n "$RF_ID" ] && wait_pane "$RF" 'fake e2e agent' 15; check "fork (rusty): the forked agent is up" $?
grep '"--fork-session"' "$S/log/claude.args" | grep -q 'r-core-claude-1-fork'; check "fork (rusty): launched with --fork-session and its own name" $?

# fork into a new worktree (trunky)
say "$TM" 'MP first turn in trunky'; wait_pane "$TM" "handled 'MP first turn in trunky'" 10
first_session t-main
keys f; wait_for "Fork '" 5 && { keys j; sleep 0.2; keys Enter; }
wait_for 'Fork Feature' 5; check "fork (trunky): new worktree opens the feature fork" $?
keys Enter; sleep 0.3; keys Enter; sleep 3; hook_prompts; sleep 1; hook_prompts; focus_sidebar
wait_for 't-main-fork' 12; check "fork (trunky): a t-main-fork feature appears" $?
tf_wd=$(feature_workdir t-main-fork)
[ -d "$tf_wd" ] && [ "$(lg -C "$tf_wd" rev-parse --abbrev-ref HEAD 2>/dev/null)" = t-main-fork ]; check "fork (trunky): in its own worktree on its own branch" $?

# Claude sessions outside aiw in the monorepo and the Rust repo
cat >"$HOME/.e2e-roster.json" <<JSON
[{"cwd": "$MP/mono/packages/a", "kind": "interactive", "pid": 999991, "startedAt": 1,
  "sessionId": "ext-mono", "name": "outside-mono", "status": "busy"},
 {"cwd": "$MP/rusty/src", "kind": "background", "id": "9e9e9e9e", "startedAt": 2, "state": "blocked",
  "status": "waiting", "waitingFor": "permission prompt", "sessionId": "ext-rusty", "name": "bg-rusty"}]
JSON
select_row 'mono' >/dev/null
wait_for '↗ outside-mono' 25; check "roster: an outside session shows under the monorepo" $?
keys E; wait_for 'Claude sessions outside AIW' 5 && cap | grep -q 'claude attach 9e9e9e9e'; check "roster: E lists both repos' outside sessions" $?
close_dialogs
rm -f "$HOME/.e2e-roster.json"
wait_gone '↗ outside-mono' 25; check "roster: they clear when they end" $?

# ═════════════════════════════════════════════════════════════════
echo "=== MP 3: groups with a fan-out pair, notes"
select_row 'n-api-claude' >/dev/null
keys G; wait_for 'Groups · nodey' 4; check "groups (nodey): G opens nodey's groups" $?
keys j; keys Enter; sleep 0.2; keys -l 'API'; keys Enter; sleep 0.4; settle
select_row 'n-api-codex' >/dev/null
keys G; wait_for 'Groups · nodey' 4; keys j; keys Enter; sleep 0.4; settle
[ "$(db_q "select count(*) from aiw_feature_group_members m join aiw_feature_groups g on g.id = m.group_id where g.name = 'API'")" = 2 ]
check "groups (nodey): the fan-out pair sits in group API" $?
lines=$(cap); g=$(echo "$lines" | grep -n '■ API' | head -1 | cut -d: -f1); fo=$(echo "$lines" | grep -n '⇶ n-api' | head -1 | cut -d: -f1)
[ -n "$g" ] && [ -n "$fo" ] && [ "$g" -lt "$fo" ]; check "groups (nodey): the fan-out header stays under the group header" $?
select_row 'r-core' >/dev/null
keys -l '#'; wait_for 'Note · r-core' 4 && { keys -l 'MP-NOTE waiting on the crate review'; keys Enter; sleep 0.4; settle; }
wait_for 'r-core ✎' 6; check "notes (rusty): a feature note marks the row" $?
keys /; sleep 0.2; keys -l 'crate review'; sleep 0.4; keys Enter; sleep 0.4; settle
cap | grep '▎' | grep -q 'r-core'; check "notes: / finds the feature by its note across projects" $?
# delete the group holding the fan-out pair: the features stay, ungrouped
select_row 'n-api-claude' >/dev/null
keys G; wait_for 'Groups · nodey' 4
for i in $(seq 1 6); do cap | grep '▸ ' | grep -q 'API' && break; keys j; sleep 0.15; done
keys d; sleep 0.3; keys y; sleep 0.5; keys Escape; sleep 0.3; settle
[ "$(db_q "select count(*) from aiw_feature_groups where name = 'API'")" = 0 ]; check "groups: deleting API removes the group" $?
[ -n "$(feature_id n-api-claude)" ] && [ -n "$(feature_id n-api-codex)" ]; check "groups: its fan-out pair survives, ungrouped" $?
wait_for '⇶ n-api · claude / codex' 5 && ! cap | grep -q '■ API'; check "groups: the fan-out header is still drawn" $?

# ═════════════════════════════════════════════════════════════════
echo "=== MP 4: messages (inbox and queue)"
for i in $(seq 1 50); do [ -n "$(db_q "select socket from aiw_inbox_addresses where session_id = '$RC_ID'")" ] && break; sleep 0.1; done
first_session r-core
keys -l '@'; wait_for ' Message ' 4 && wait_for 'via inbox' 4; check "msg (rusty): @ reaches Claude via its inbox" $?
keys -l 'MP-MSG-RUSTY run cargo test'; keys Enter; sleep 0.5; settle
wait_pane "$RC" 'Message from @aiw: MP-MSG-RUSTY run cargo test' 10; check "msg (rusty): the claude got it" $?
[ "$(grep -c 'MP-MSG-RUSTY' "$S/log/claude.inbox" 2>/dev/null)" = 1 ]; check "msg (rusty): exactly once" $?
m_name=$(db_q "select name from aiw_session_names where session_id = '$MC_ID'")
auto message --to "$m_name" --from rusty --text 'MP-MSG-MONO packages changed' >"$S/msg-mono.json"
grep -q '"status": "delivered"' "$S/msg-mono.json"; check "msg: one repo's agent messages another repo's Claude by name" $?
wait_pane "$MC" 'Message from @rusty: MP-MSG-MONO' 10; check "msg: the monorepo's claude shows it from @rusty" $?
auto message --to "$NX_ID" --text 'MP-MSG-CODEX lint the routes' >"$S/msg-codex.json"
grep -q '"status": "queued"' "$S/msg-codex.json"; check "msg (nodey): a Codex target is queued" $?
say "$NX" '/done'
wait_pane "$NX" 'followup: MP-MSG-CODEX lint the routes' 12; check "msg (nodey): delivered at the Codex turn end" $?
sleep 1.5; [ "$(count_pane "$NX" 'followup: MP-MSG-CODEX')" = 1 ]; check "msg (nodey): exactly once" $?

# ═════════════════════════════════════════════════════════════════
echo "=== MP 5: Claude's own worktrees in two repos at once"
WT_LOG="$S/log/claude.worktrees"
say "$RC" '/subwt agent-rusty'; say "$MC" '/subwt agent-mono'
wait_pane "$RC" '\[worktree\] subagent /' 15 && wait_pane "$MC" '\[worktree\] subagent /' 15
check "wt: both subagents' WorktreeCreate returned a path" $?
wt_r=$(db_q "select path from aiw_claude_worktrees where name = 'agent-rusty'")
wt_m=$(db_q "select path from aiw_claude_worktrees where name = 'agent-mono'")
case "$wt_r" in "$XDG_DATA_HOME"/ai_workstation/worktrees/*/claude-agent-rusty) true ;; *) false ;; esac \
    && case "$wt_m" in "$XDG_DATA_HOME"/ai_workstation/worktrees/*/claude-agent-mono) true ;; *) false ;; esac
check "wt: each lands in AIW's worktree folder" $?
[ "$(lg -C "$wt_r" rev-parse --git-common-dir 2>/dev/null)" = "$MP/rusty/.git" ] && [ "$(lg -C "$wt_m" rev-parse --git-common-dir 2>/dev/null)" = "$MP/mono/.git" ]
check "wt: each is a worktree of its own repository" $?
[ "$(db_q "select parent_session_id from aiw_claude_worktrees where path = '$wt_r'")" = "$RC_ID" ] \
    && [ "$(db_q "select parent_session_id from aiw_claude_worktrees where path = '$wt_m'")" = "$MC_ID" ]
check "wt: each is recorded with the session that asked" $?
[ -f "$wt_m/packages/a/VERSION" ] && [ -f "$wt_m/packages/b/VERSION" ]; check "wt: the monorepo worktree has both packages" $?
select_row 'r-core' >/dev/null; wait_for '⎇ agent-rusty' 6; check "wt: drawn under r-core" $?
select_row 'm-dev' >/dev/null; wait_for '⎇ agent-mono' 6; check "wt: drawn under m-dev" $?
say "$RC" "/wtrm $wt_r"; say "$MC" "/wtrm $wt_m"
wait_pane "$RC" "\\[worktree\\] removed $wt_r" 10 && wait_pane "$MC" "\\[worktree\\] removed $wt_m" 10
check "wt: both removed through WorktreeRemove" $?
[ ! -e "$wt_r" ] && [ ! -e "$wt_m" ] && [ "$(db_q "select count(*) from aiw_claude_worktrees")" = 0 ]; check "wt: directories and records gone" $?

# ═════════════════════════════════════════════════════════════════
echo "=== MP 6: a background session, and a message to it"
BG_LOG="$S/log/claude.bg"
select_row 't-main' >/dev/null
keys s; wait_for 'Start Session' 4; keys j Enter; wait_for 'Name Session' 4; keys Enter
wait_for 'Background: ' 4; keys -l 'MP-BG first'; keys Enter; sleep 1
for i in $(seq 1 60); do [ -n "$(db_q "select b.bg_id from aiw_background_sessions b join feature_sessions s on s.id = b.session_id join features f on f.id = s.feature_id where f.name = 't-main'")" ] && break; sleep 0.1; done
BG_SID=$(db_q "select b.session_id from aiw_background_sessions b join feature_sessions s on s.id = b.session_id join features f on f.id = s.feature_id where f.name = 't-main'")
BG_ID=$(db_q "select bg_id from aiw_background_sessions where session_id = '$BG_SID'")
[ -n "$BG_ID" ]; check "bg (trunky): claude --bg started, its id stored ($BG_ID)" $?
grep -F -- "$BG_ID" "$BG_LOG" | grep '"op": "dispatch"' | grep -q 'MP-BG first'; check "bg (trunky): with the first prompt" $?
select_row 'Claude bg 1' >/dev/null
wait_for 'bg · done' 20; check "bg: its hooks reach the row (bg · done)" $?
for i in $(seq 1 60); do [ -n "$(db_q "select socket from aiw_inbox_addresses where session_id = '$BG_SID'")" ] && break; sleep 0.2; done
auto message --to "$BG_SID" --text 'MP-MSG-BG summarize' >"$S/msg-bg.json"
st=$(jget "$S/msg-bg.json" 'd.get("status")')
[ "$st" = delivered ] || [ "$st" = queued ]; check "bg: a message to the background session is accepted ($st)" $?
if [ "$st" = delivered ]; then
    for i in $(seq 1 50); do grep -q 'MP-MSG-BG' "$S/log/claude.inbox" 2>/dev/null && break; sleep 0.1; done
    [ "$(grep -c 'MP-MSG-BG' "$S/log/claude.inbox" 2>/dev/null)" = 1 ]; check "bg: delivered into its inbox exactly once" $?
else
    [ "$(db_q "select count(*) from aiw_followups where session_id = '$BG_SID'")" = 1 ]; check "bg: queued exactly once" $?
fi
keys Enter; sleep 1
wait_agent_for "attached to $BG_ID" 10; check "bg: Enter attaches in the agent pane" $?
keys -l '/detach'; keys Enter; sleep 3
for i in $(seq 1 30); do aiw_tmux list-windows -t "$TM_S" -F '#W' 2>/dev/null | grep -qx aiw-bg-claude || break; sleep 0.2; done
! aiw_tmux list-windows -t "$TM_S" -F '#W' 2>/dev/null | grep -qx aiw-bg-claude; check "bg: detach closes the attach window" $?
focus_sidebar; select_row 'Claude bg 1' >/dev/null
keys x; for i in $(seq 1 80); do grep -F -- "$BG_ID" "$BG_LOG" | grep -q '"op": "stop"' && break; sleep 0.1; done
grep -F -- "$BG_ID" "$BG_LOG" | grep -q '"op": "stop"'; check "bg: x stops it" $?
wait_for 'bg · stopped' 10; check "bg: the row says stopped" $?

# ═════════════════════════════════════════════════════════════════
echo "=== MP 7: the watchdog"
# a session waiting on you is never stuck, however long it waits
first_session rem-1
say "$RM" '/ask'; wait_pane "$RM" 'Do you want to proceed' 8
wait_sel_glyph '⛔' 8; check "wd (remote): the waiting session shows ⛔" $?
sleep 12
! cap | grep '▎' | grep -q 'stuck'; check "wd (remote): waiting on you past the stuck window is not flagged stuck" $?
say "$RM" 'yes'; wait_pane "$RM" "handled 'yes'" 8
# a hung turn in the Python repo's shared feature
first_session p-shared
resumes0=$(grep -c -- '--resume' "$S/log/claude.args" 2>/dev/null || echo 0)
say "$PC" '/hang'
wait_for 'stuck · no hooks or output' 30; check "wd (pyth): the hung claude is flagged stuck" $?
keys -l '!'; wait_for ' Recover ' 4 && wait_for 'Restart, resuming' 3; check "wd (pyth): ! offers restart" $?
n=$(cap | grep -E 'Restart, resuming' | head -1 | grep -oE '│ +[0-9] ' | grep -oE '[0-9]'); [ -n "$n" ] && keys "$n"; settle
for i in $(seq 1 200); do [ "$(grep -c -- '--resume' "$S/log/claude.args")" -gt "$resumes0" ] && break; sleep 0.1; done
[ "$(grep -c -- '--resume' "$S/log/claude.args")" -gt "$resumes0" ]; check "wd (pyth): restart relaunched it resuming" $?
sleep 3; ! cap | grep '▎' | grep -q 'stuck'; check "wd (pyth): not stuck after the restart" $?
# an exited agent in the monorepo
say "$MC" '/exit'
listed=1
for i in $(seq 1 30); do
    keys i; sleep 0.6
    if cap | grep -q 'Needs Attention' && cap | grep -qE 'mono / m-dev +- .*exited · ! recover'; then listed=0; break; fi
    cap | grep -q 'Needs Attention' && keys Escape
    sleep 0.4
done
[ "$listed" = 0 ]; check "wd (mono): the exited claude is in the needs-you list" $?
keys Escape; settle
# a follow-up queued for the idle codex beside the Python claude
auto queue --session "$PX_ID" 'MP-WD-QUEUED lint it' >"$S/wd-q.out"
select_row 'p-shared' >/dev/null
wait_for 'queued for an idle agent' 15; check "wd (pyth): a follow-up behind the idle codex is flagged" $?
[ "$(inputs codex 'MP-WD-QUEUED lint it')" = 0 ]; check "wd (pyth): nothing typed by itself" $?

# ═════════════════════════════════════════════════════════════════
echo "=== MP 8: schedules (one aimed at a forked session)"
auto schedule add --session "$RF_ID" --when 'every 6s' --prompt 'MP-SCHED-FORK' >"$S/sc-fork.json"
SC_FORK=$(jget "$S/sc-fork.json" 'd["schedule"]["id"]')
[ -n "$SC_FORK" ]; check "sched: a prompt schedule on the forked session (rusty)" $?
for i in $(seq 1 80); do [ "$(inputs claude 'MP-SCHED-FORK')" -ge 1 ] && break; sleep 0.25; done
wait_pane "$RF" "handled 'MP-SCHED-FORK'" 10; check "sched: the forked session ran the scheduled prompt" $?
! pane_of "$RC" | grep -q 'MP-SCHED-FORK'; check "sched: only the forked session, not its source" $?
auto schedule pause --id "$SC_FORK" >/dev/null
auto schedule add --feature n-api-claude --project nodey --when 'every 1h' --command 'echo MP-SCHED-SHELL-$((6*7))' --target mp-term >"$S/sc-sh.json"
SC_SH=$(jget "$S/sc-sh.json" 'd["schedule"]["id"]')
auto schedule run --id "$SC_SH" >"$S/sc-sh-run.json"
[ "$(jget "$S/sc-sh-run.json" 'd["outcome"]')" = ran ]; check "sched: a command schedule runs in the Node repo's terminal" $?
sh_win=$(db_q "select s.tmux_window from feature_sessions s join features f on f.id = s.feature_id where f.name = 'n-api-claude' and s.label = 'mp-term'")
wait_pane "$NC_S:$sh_win" 'MP-SCHED-SHELL-42' 8; check "sched: its output is in the terminal" $?

# ═════════════════════════════════════════════════════════════════
echo "=== MP 9: land across the repositories"
# rusty: rebase onto an advanced base, cargo check passes, local merge; the
# schedule on the landed feature's fork is dropped with the feature
rc_wd=$(feature_workdir r-core)
commit_in "$rc_wd" src/extra.rs 'pub fn extra() {}' "rusty feature"
commit_in "$MP/rusty" CHANGELOG.md 'base moved' "base work"
auto land --feature r-core --project rusty --yes >"$S/land-rusty.json"
[ "$(jget "$S/land-rusty.json" 'd["state"]')" = succeeded ]; check "land (rusty): rebased, cargo check passed, merged" $?
[ "$(lg -C "$MP/rusty" show main:src/extra.rs 2>/dev/null)" = 'pub fn extra() {}' ] && lg -C "$MP/rusty" show main:CHANGELOG.md >/dev/null 2>&1
check "land (rusty): main holds the feature on top of the moved base" $?
grep -q 'RUSTY-CHECK-OK' "$XDG_STATE_HOME"/ai_workstation/land/*.log; check "land (rusty): cargo check ran in the worktree" $?
for i in $(seq 1 120); do [ -z "$(feature_id r-core)" ] && break; sleep 0.25; done
[ -z "$(feature_id r-core)" ]; check "land (rusty): the feature is removed after the land" $?
[ "$(db_q "select count(*) from aiw_schedules where id = '$SC_FORK'")" = 0 ]; check "land (rusty): the fork's schedule went with the feature" $?
[ ! -d "$rc_wd" ]; check "land (rusty): its worktree is gone" $?

# nodey: land one fan-out sibling; its twin stays and still runs
nx_wd=$(feature_workdir n-api-codex)
commit_in "$nx_wd" routes.js 'module.exports = [];' "codex routes"
auto land --feature n-api-codex --project nodey --yes >"$S/land-nodey.json"
[ "$(jget "$S/land-nodey.json" 'd["state"]')" = succeeded ]; check "land (nodey): npm test passed, the codex sibling landed" $?
lg -C "$MP/nodey" show main:routes.js >/dev/null 2>&1; check "land (nodey): main has the sibling's work" $?
for i in $(seq 1 120); do [ -z "$(feature_id n-api-codex)" ] && break; sleep 0.25; done
[ -z "$(feature_id n-api-codex)" ] && [ -n "$(feature_id n-api-claude)" ]; check "land (nodey): the landed sibling is removed, its twin stays" $?
aiw_tmux has-session -t "=$NC_S" 2>/dev/null; check "land (nodey): the twin's agent still runs" $?

# pyth: the check fails on purpose: nothing merged, ⛔, listed in i
pc_wd=$(feature_workdir p-shared)
commit_in "$pc_wd" pyth/extra.py 'X = 1' "python feature"
py_main=$(lg -C "$MP/pyth" rev-parse main)
auto land --feature p-shared --project pyth --yes >"$S/land-pyth.json"
[ "$(jget "$S/land-pyth.json" 'd["state"]')" = failed ]; check "land (pyth): the failing check fails the land" $?
[ "$(lg -C "$MP/pyth" rev-parse main)" = "$py_main" ]; check "land (pyth): nothing merged" $?
grep -q 'PYTH-CHECK-FAILED' "$XDG_STATE_HOME"/ai_workstation/land/*.log; check "land (pyth): the check's own message is in the log" $?
[ -z "$(lg -C "$pc_wd" status --porcelain 2>/dev/null)" ]; check "land (pyth): the check left the worktree clean (__pycache__ ignored)" $?
select_row 'p-shared' >/dev/null; wait_for '⛔ land failed' 8; check "land (pyth): the row shows ⛔ land failed" $?
keys i; wait_for 'Needs Attention' 4 && wait_for 'p-shared' 3; cap | grep 'p-shared' | grep -q 'land failed'; check "land (pyth): listed in i, naming the land's failure" $?
! cap | grep -qE 'rusty / r-core|nodey / n-api-codex'; check "land: the landed features left nothing behind in i" $?
close_dialogs

# trunky: the base is trunk
tm_wd=$(feature_workdir t-main)
commit_in "$tm_wd" trunk-feature.txt 'on trunk' "trunk feature"
auto land --feature t-main --project trunky --dry-run >"$S/land-trunky-dry.json"
[ "$(jget "$S/land-trunky-dry.json" 'd["preflight"]["base"]')" = trunk ]; check "land (trunky): the base is trunk" $?
auto land --feature t-main --project trunky --yes >"$S/land-trunky.json"
[ "$(jget "$S/land-trunky.json" 'd["state"]')" = succeeded ] && lg -C "$MP/trunky" show trunk:trunk-feature.txt >/dev/null 2>&1
check "land (trunky): landed onto trunk" $?
[ -n "$(feature_id t-main-fork)" ]; check "land (trunky): the fork feature is untouched" $?

# remote: finish pr through the fake gh and the bare remote
rm_wd=$(feature_workdir rem-1)
commit_in "$rm_wd" pr.txt 'pr work' "pr work"
auto land --feature rem-1 --project remote --yes >"$S/land-remote.json"
[ "$(jget "$S/land-remote.json" 'd["state"]')" = succeeded ] && [ "$(jget "$S/land-remote.json" 'd["result"]["pr_url"]')" = https://github.com/e2e/remote/pull/7 ]
check "land (remote): pushed and opened a PR" $?
[ "$(lg -C "$MP/remote.git" rev-parse refs/heads/rem-1 2>/dev/null)" = "$(lg -C "$rm_wd" rev-parse HEAD)" ]; check "land (remote): the branch is on the remote" $?
grep -q 'pr create --base main --head rem-1' "$S/log/gh.calls"; check "land (remote): gh got --base main --head rem-1" $?
[ -d "$rm_wd" ]; check "land (remote): a PR finish keeps the feature" $?

# mono: the monorepo's check passes, merge local
mc_wd=$(feature_workdir m-dev)
commit_in "$mc_wd" packages/b/VERSION 'b2' "bump b"
auto land --feature m-dev --project mono --yes >"$S/land-mono.json"
[ "$(jget "$S/land-mono.json" 'd["state"]')" = succeeded ] && [ "$(lg -C "$MP/mono" show main:packages/b/VERSION 2>/dev/null)" = b2 ]
check "land (mono): the per-package check passed and it landed" $?

# ═════════════════════════════════════════════════════════════════
echo "=== MP 10: wrap-up"
no_panic; check "mp: no panic" $?
aiw_quit; check "mp: q quits" $?
doctor_clean; check "mp: aiw doctor finds nothing orphaned" $?

echo
echo "=== Multi-project results ($RESULTS)"
printf 'PASS %s  FAIL %s\n' "$(grep -c '^PASS' "$RESULTS")" "$(grep -c '^FAIL' "$RESULTS")"
grep '^FAIL' "$RESULTS" || true
echo "captures: $S/captures"
