# shellcheck shell=bash
# §19.4 feature groups, notes and layout presets. Sourced by run.sh after
# fan_out.sh, with the cockpit up, alpha holding the fan-out features from
# fan_out.sh, and the helpers select_row / cap / keys / check / wait_for /
# hook_prompts / focus_sidebar / close_dialogs / settle defined.
#
#   1. two groups in alpha, a fan-out pair moved into one, alpha-1 into the
#      other; a group folded to one row and reopened by a search jump;
#   2. notes on a feature and a session: the sidebar marks them, the info
#      block shows one, `/` finds both by their text;
#   3. a new project whose aiw.json has a layout preset: a feature created
#      with it opens claude + a terminal running a command + a custom session.

echo "=== Part 2f: groups, notes, layout presets (DESIGN.md §19.4)"

db_count() {
    python3 -c "import sqlite3,sys; print(sqlite3.connect('$XDG_CONFIG_HOME/ai_workstation/aiw.db').execute(sys.argv[1]).fetchone()[0])" "$1" 2>/dev/null
}
# groups_dialog: G on the selected row, wait for the dialog
groups_dialog() { keys G; wait_for 'Groups · alpha' 3; }

# ── 1. groups ───────────────────────────────────────────────────
select_row 'fo-each-claude' >/dev/null
groups_dialog; check "groups: G opens the project's groups" $?
wait_for "Move 'fo-each-claude' to:" 3 && wait_for '\+ New group' 3; check "groups: the dialog offers (no group) and + New group for the feature" $?
keys j; keys Enter; sleep 0.2; keys -l 'Experiments'; keys Enter; sleep 0.4; settle
cap | grep -q '■ Experiments'; check "groups: a new group takes the feature at once and heads it in the sidebar" $?
select_row 'fo-each-codex' >/dev/null
groups_dialog; keys j; keys Enter; sleep 0.4; settle
[ "$(db_count "SELECT COUNT(*) FROM aiw_feature_group_members m JOIN aiw_feature_groups g ON g.id = m.group_id WHERE g.name = 'Experiments'")" = 2 ]
check "groups: both fan-out siblings sit in Experiments" $?
select_row 'fo-each-codex' >/dev/null
lines=$(cap); g=$(echo "$lines" | grep -n '■ Experiments' | head -1 | cut -d: -f1); fo=$(echo "$lines" | grep -n '⇶ fo-each' | head -1 | cut -d: -f1)
[ -n "$g" ] && [ -n "$fo" ] && [ "$g" -lt "$fo" ]; check "groups: the fan-out header stays, under the group header" $?
select_row 'fo-shared' >/dev/null
groups_dialog; keys n; keys -l 'Shared work'; keys Enter; sleep 0.4; settle
select_row 'alpha-1' >/dev/null
groups_dialog
cap | grep -q 'Shared work'; check "groups: the dialog lists both groups" $?
keys j; keys j; keys Enter; sleep 0.4; settle   # (no group) → Experiments → Shared work
[ "$(db_count "SELECT COUNT(*) FROM aiw_feature_group_members m JOIN aiw_feature_groups g ON g.id = m.group_id WHERE g.name = 'Shared work'")" = 2 ]
check "groups: alpha-1 moved into Shared work beside fo-shared" $?
groups_dialog; keys n; keys -l 'experiments'; keys Enter; sleep 0.3
cap | grep -q 'already exists'; check "groups: a duplicate name is refused in the dialog" $?
keys Escape; keys Escape; sleep 0.3; settle
# fold Experiments with C, reopen it with a search jump
select_row 'fo-each-claude' >/dev/null
keys C; sleep 0.4
cap | grep -q '▸ ■ Experiments · 2 features'; check "groups: C folds the group to one row" $?
[ "$(db_count "SELECT collapsed FROM aiw_feature_groups WHERE name = 'Experiments'")" = 1 ]; check "groups: the fold is saved" $?
keys /; sleep 0.2; keys -l 'fo-each-codex'; sleep 0.3; keys Enter; sleep 0.4; settle
cap | grep -q '▾ ■ Experiments'; check "groups: a search jump into a folded group opens it" $?
cap | grep '▎' | grep -q 'fo-each-codex'; check "groups: and selects the feature" $?
snap groups-sidebar

# ── 2. notes ────────────────────────────────────────────────────
select_row 'fo-shared' >/dev/null
keys -l '#'; wait_for 'Note · fo-shared' 3; check "notes: # opens the note editor on the feature" $?
keys -l 'Waiting on the API review'; keys M-Enter; keys -l 'second line'
wait_for 'second line' 3; keys Enter
wait_for 'fo-shared ✎' 5; check "notes: the feature row is marked ✎" $?
select_row 'alpha-1' >/dev/null; keys l; sleep 0.2; keys j; sleep 0.3   # alpha-1's first session
cap | grep '▎' | grep -q 'Claude'; check "notes: on alpha-1's agent session" $?
keys -l '#'; wait_for 'Note · Claude 1 · alpha-1' 3; check "notes: # on a session names the session and its feature" $?
keys -l 'remember: rotate keys'; wait_for 'remember: rotate keys' 3; keys Enter; sleep 0.3; settle
wait_for '✎ remember: rotate keys' 5; check "notes: the info block shows the selected session's note" $?
keys /; sleep 0.2; keys -l 'rotate keys'; sleep 0.4
cap | grep -q '✎ remember: rotate keys'; check "notes: / finds a session by its note, showing the line" $?
keys Escape; sleep 0.2; settle
keys /; sleep 0.2; keys -l 'API review'; sleep 0.4; keys Enter; sleep 0.4; settle
cap | grep '▎' | grep -q 'fo-shared'; check "notes: / jumps to a feature by its note" $?
[ "$(db_count 'SELECT COUNT(*) FROM aiw_notes')" = 2 ]; check "notes: both notes are saved" $?
snap notes-sidebar

# ── 3. layout presets ───────────────────────────────────────────
rm -rf "$S/repos/delta"; mkdir -p "$S/repos/delta"
git -C "$S/repos/delta" init -q -b main
printf '# delta\n' >"$S/repos/delta/README.md"
cat >"$S/repos/delta/aiw.json" <<'J'
{
  "custom_sessions": [ { "name": "server", "command": "echo LAYOUT-SERVER-UP; sleep 600", "window_name": "server" } ],
  "layout_presets": [
    { "name": "full", "description": "agent, a watcher and the server",
      "sessions": [
        { "kind": "terminal", "label": "watch", "command": "echo LAYOUT-WATCH-RAN" },
        { "kind": "custom", "name": "server" }
      ] },
    { "name": "broken", "sessions": [ { "kind": "custom", "name": "nope" } ] }
  ]
}
J
git -C "$S/repos/delta" add -A && git -C "$S/repos/delta" commit -qm init
add_project delta "$S/repos/delta" 0; check "layout: project delta added" $?
# open_layout_dialog <branch> <presses of j on the Layout row>
open_layout_dialog() {
    select_row 'delta' >/dev/null
    keys n; wait_for 'New Feature' 5 || return 1; sleep 0.3  # a frame can be caught half drawn
    if cap | grep -q 'Source:'; then keys Enter; sleep 0.2; fi
    wait_for 'Name:' 3 || return 1
    keys -l "$1"; keys Enter; sleep 0.3
    keys Enter; sleep 0.3                     # worktree as offered
    local i; for i in 1 2 3 4 5; do keys k; done   # Claude
    keys Enter; keys Enter; keys Enter; sleep 0.3  # mode → chrome → layout
    for i in $(seq 1 "$2"); do keys j; done
    sleep 0.2
}
open_layout_dialog lay-bad 2
cap | grep -q '‹ broken ›'; check "layout: the New Feature dialog has a Layout row" $?
keys Enter; keys Enter; keys Enter; sleep 0.6
# the toast wraps at ~47 columns, so look for the short token and for the
# dialog still being open
wait_for 'nope' 3 && cap | grep -qE 'Task prompt|New Feature'; check "layout: a bad preset is refused before anything is created" $?
! aiw_tmux has-session -t aiw-delta-lay-bad 2>/dev/null; check "layout: no feature was started for the refused preset" $?
close_dialogs
open_layout_dialog lay-1 1
cap | grep -q '‹ full ›'; check "layout: j picks the preset" $?
keys Enter; keys Enter; keys Enter
for _ in 1 2 3; do sleep 1.5; hook_prompts; done; focus_sidebar
windows=$(aiw_tmux list-windows -t aiw-delta-lay-1 -F '#W' 2>/dev/null)
echo "$windows" | grep -qx claude && echo "$windows" | grep -qx server && [ "$(echo "$windows" | wc -l)" -ge 3 ]
check "layout: the feature runs claude, the terminal and the custom session" $?
seen=0
for w in $windows; do aiw_tmux capture-pane -p -J -S -200 -t "aiw-delta-lay-1:$w" 2>/dev/null | grep -q '^LAYOUT-WATCH-RAN' && seen=1; done
[ "$seen" = 1 ]; check "layout: the terminal ran its command" $?
for i in $(seq 1 20); do aiw_tmux capture-pane -p -S -50 -t aiw-delta-lay-1:server 2>/dev/null | grep -q LAYOUT-SERVER-UP && break; sleep 0.25; done
aiw_tmux capture-pane -p -S -50 -t aiw-delta-lay-1:server 2>/dev/null | grep -q LAYOUT-SERVER-UP; check "layout: the custom session runs its own command" $?
select_row 'lay-1' >/dev/null
cap | grep -q 'watch' && cap | grep -q 'server'; check "layout: the sidebar lists the layout's sessions" $?
no_panic; check "organize: no panic" $?
close_dialogs
