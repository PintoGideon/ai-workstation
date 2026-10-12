# AI Workstation: what it does

This is the plain-English guide to AI Workstation (`aiw`) as it is today. It
describes every feature the current source ships, how the pieces fit, and
what the app deliberately leaves to the agents. Read it top to bottom the
first time; later, jump to the section you need. Keys and settings here were
checked against the code on the branch this file was written on.

Contents

1. [What aiw is, and is not](#1-what-aiw-is-and-is-not)
2. [Core concepts](#2-core-concepts)
3. [A day with aiw](#3-a-day-with-aiw)
4. [Every dashboard key](#4-every-dashboard-key)
5. [Sessions, the info block, context and cost](#5-sessions-the-info-block-context-and-cost)
6. [TODO lists](#6-todo-lists)
7. [Prompt library and past prompts](#7-prompt-library-and-past-prompts)
8. [GitHub: issues and pull-request badges](#8-github-issues-and-pull-request-badges)
9. [Configuration](#9-configuration)
10. [Supported harnesses](#10-supported-harnesses)
11. [Command line and automation](#11-command-line-and-automation)
12. [What aiw writes on disk, and the no-cloud guarantee](#12-what-aiw-writes-on-disk-and-the-no-cloud-guarantee)
13. [The desktop GUI](#13-the-desktop-gui)
14. [Removed features and their native replacements](#14-removed-features-and-their-native-replacements)

## 1. What aiw is, and is not

AI Workstation is a terminal dashboard for running many AI coding agents at
once. Each agent gets its own git branch, usually its own worktree, and its own
tmux window. The dashboard shows every agent in one list, tells you which one
is waiting for you, lets you answer its permission prompt without opening it,
queues your next instruction for when it finishes, and shows how much context
and money each session has used. It supports Claude Code, Codex, Gemini CLI,
Cursor agent and Pi. It makes no network calls of its own.

aiw is a workstation, not a second agent. On 2026-10-11 everything that
re-implemented a feature the harnesses already ship was removed: the plan
interview, review mode and the final review viewer, AI pull-request review,
pull-request triage, learning mode, the model analyzer, session summaries,
the fresh-context handoff, the steering prompt, remote control, editable
headless prompts and the headless runner. Section 14 lists each one with the
harness feature that replaces it. The short version: plan inside the agent
with its plan mode, review with its review command, compact with its compact
command, and ask it questions in its own pane.

## 2. Core concepts

**Project.** A directory you work in. It can be a git repository or an
ordinary folder. Projects are the top level of the sidebar.

**Feature.** One task inside a project. A feature has a name, a git branch
and a working directory. The first feature in a repository may use the
repository directory itself. Other features get a worktree, so several agents
can edit the same project without touching each other's files. A feature owns
a tmux session on aiw's private tmux server.

**Worktree.** A separate checkout of the repository on its own branch. aiw
creates worktrees outside the repository, under its own data directory, so
nothing lands inside your checkout. Files that git ignores but a fresh
checkout needs, such as `.env`, are copied in. A repository can list those
files in a `.worktreeinclude` file at its root. Worktrees created by older
versions under `.worktrees` inside the repository keep working.

**Session.** One tmux window inside a feature. A session is an agent, a
terminal, Neovim, VS Code, a TODO list, or a custom command you configured.
A feature can hold any number of sessions.

**Harness.** The agent CLI a session runs: Claude Code, Codex, Pi, Gemini
CLI or Cursor agent. The harness setup screen decides which ones aiw offers.
A project can narrow that list in its own config.

**Mode.** How much freedom an agent gets. aiw calls the three levels
Vibeless, Vibe and SuperVibe, and maps each one to the harness's own flags.

| Mode | Meaning | Claude Code | Codex | Gemini CLI | Cursor agent | Pi |
| --- | --- | --- | --- | --- | --- | --- |
| Vibeless | Every file edit is held until you approve it on the dashboard | default permissions plus aiw's edit-approval hook | not available; aiw refuses the combination | `--approval-mode default` | default | no flags |
| Vibe | Edits are accepted; other permissions still ask | `--permission-mode acceptEdits` | default | `--approval-mode auto_edit` | default | no flags |
| SuperVibe | No permission prompts at all; aiw warns before enabling it | `--dangerously-skip-permissions` | `--sandbox danger-full-access --ask-for-approval never` | `--approval-mode yolo` | `--force` | no flags |

Claude sessions can also get the `--chrome` flag for browser automation. That
is the Chrome row in the new-feature dialog. Pi receives no mode flags at all,
so the mode you pick for a Pi feature does not change Pi's behaviour.

**The cockpit.** `aiw` with no arguments builds a small tmux layout for
itself: the sidebar on the left, and on the right the selected agent's real
tmux window. Nothing is relayed or redrawn. tmux draws the agent, aiw draws
the sidebar. Typing in the right pane is native, exactly like a tmux attach.
`Ctrl+Q` flips the keyboard between the two panes and works from either side.
Any dialog zooms the sidebar over the whole window and un-zooms when it
closes. Closing the terminal window stops nothing: the cockpit, the dashboard
process and every agent keep running, and the next `aiw` attaches again. A
second `aiw` attaches a second client to the same cockpit. `q` in the sidebar
shuts the dashboard down and leaves the agents running.

**The grid.** `v` on a feature replaces the right pane with all of that
feature's agents tiled as live tmux panes. On a project row it tiles the first
agent of each feature. Click a pane to focus it. `Alt+q` closes the grid and
the pane returns to the selected agent. The same tiles are available in any
terminal with `aiw grid`.

**Badges.** Every row carries one glyph, and the header, the needs-you list
and the grid use the same ones.

| Glyph | Meaning |
| --- | --- |
| ⛔ | needs you: a permission prompt, a question, a turn that failed on an API error, or a stop nothing explained. The reason sits on the next line |
| ● | working. The activity line shows the elapsed time, context use, subagent count, open tasks and background work when the harness reports them |
| ✓ | finished a turn. Bright until you look at it, dim afterwards |
| ○ | idle: running, nothing to report. A terminal shows "open" |
| ■ | stopped. "exited" when the agent died at its shell; `c` relaunches it in the same window |

Project rows show counts such as `⛔1 ●7 ✓2`. A feature row shows its most
pressing session.

## 3. A day with aiw

**Start.** Open a real terminal window and run `aiw`. On the very first run
aiw probes each supported agent CLI on your PATH, ticks the installed ones,
and asks whether your terminal font has icon glyphs. Press `c` to confirm the
harness list. `A` reopens that screen later; there, `Enter` or `Space` ticks a
row and `c` saves. Pressing `Esc` on the later screen discards your changes.

**Add a project.** Press `N`, type a name and a directory. A leading `~` is
expanded. The directory may be a git repository or a plain folder.

**Create a feature.** Select the project and press `n`. The dialog asks for a
name, whether to use a worktree, which harness, which mode, and whether to
enable Chrome for Claude, then the session name and an optional task prompt.
Names are checked against git's branch rules as you type. If the project's
config has feature presets, the dialog offers them. The feature starts as soon
as the dialog closes, unless the machine is past its agent or memory limit, in
which case it is created stopped with a toast saying why. A task prompt waits
until the agent has started and is then pasted into its pane unsent, unless
you ticked "send when ready".

**Watch the agent.** Move the sidebar cursor onto the agent and the right
pane shows it after a short pause. Press `Enter` to put the keyboard in the
pane and type to the agent. `Ctrl+Q` brings the keyboard back.

**Notice when it needs you.** When the agent asks a question or wants
permission, its row turns ⛔ and the header count goes up. Press `i` for the
list of everything waiting, questions first and oldest first. `Enter` on a
row opens the session. `x` dismisses the row. A state nobody answers for
thirty minutes ages back to idle so the list stays useful.

**Answer a permission from the dashboard.** When Claude, Codex or Cursor is
about to run a tool, its hook sends the request to aiw and waits. The row
reads `⛔ permission: Bash — cargo test`. Press `a` to allow or `X` to deny,
on the row, in the `i` list, or in the detail overlay that `Enter` opens from
the list. The detail overlay shows the tool and its full input; `p` lets the
agent ask in its own pane instead and `o` opens the session. If you do not
answer within five minutes the hook gives up and the agent shows its own
prompt, so nothing can get stuck. Gemini's hooks can only observe, so its ⛔
is answered in the pane.

**Approve an edit in Vibeless mode.** A Claude session in Vibeless mode has
each file write held by a hook until you decide. A popup shows the diff.
`Enter` lets the edit through, `r` rejects it and asks for a one-line reason
that goes back to the agent, and `q` or `Esc` skips the decision and lets the
agent carry on. `v` switches between unified and side-by-side, `i` picks a
syntax parser for the file. The popup ignores `Enter` for a moment after it
opens so a keystroke meant for the sidebar cannot approve an edit by
accident. `V` on the dashboard reopens a pending approval.

**Queue what comes next.** Press `e` on an agent row to queue a follow-up.
Type the instruction and press `Enter`; `Shift+Enter` adds a line. `Tab`
moves to the list where `J` and `K` reorder, `d` drops, and `Enter` takes an
item back into the editor. Items are delivered one per turn, when the
agent's own stop hook asks aiw for the next one. Nothing is ever typed into a
busy agent. The queue lives in the database, so it survives a restart. The
row shows `⏭ 2 queued`.

**Add a second session.** Press `s` on the feature. The picker offers every
enabled harness, a terminal, Neovim, VS Code when the `code` command exists,
a TODO list once per feature, and any custom sessions the project config
defines. A second agent in the same feature shares the same checkout; use a
second feature with its own worktree when you want isolation.

**Run Claude in the background.** The picker's "Claude (background)" entry
(right after Claude) adds a Claude session that runs under Claude Code's own
supervisor (`claude --bg`) instead of in a pane: it keeps working when the
terminal, the cockpit or aiw itself closes. You name it, give it a first
prompt (or none, and it waits for you), and it starts. In the New Feature
dialog, `b` on the harness list turns the highlighted Claude into
"Claude (background)" for that feature. See "Background Claude sessions"
below for what the row shows and how to attach.

**Give several agents the same prompt.** On the dialog's harness list, press
`Space` on each harness you want; the first one ticked is the primary. With
two or more ticked, the dialog asks where they work. "One worktree each"
creates sibling features named `<name>-claude`, `<name>-codex` from the same
base, and the sidebar groups them under a `⇶ name · claude / codex` header
with one ⛔ count for the group. "One worktree, shared" creates one feature
with a window per agent, `Claude 1` and `Codex 1`. The repo directory and an
existing worktree can only be shared. Then type or paste the task prompt;
`Ctrl+L` inserts a template from the prompt library and `Alt+Enter` adds a
line. Each agent gets the prompt once its pane is ready, which aiw learns from
the agent's own session-start hook. It is pasted unsent so you can read it
first; tick `Ctrl+T` "send when ready" and it is sent instead. A row waiting
for its prompt shows `⏸ prompt`. If Codex is in a Vibeless set, every agent
runs in Vibe, since Codex has no per-edit approval, and the dialog says so.
`v` on any sibling tiles the whole group. Presets in `aiw.json` can tick
several agents and choose the layout too.

**Switch and resume.** `Enter` on a stopped session whose window vanished
behind your back, after a reboot or a crash, offers to resume the saved
Claude or Codex conversation, start fresh, or pick another saved transcript.
`S` opens that transcript picker directly. `c` starts a stopped feature or
relaunches an exited agent in the same window.

**Stop, forget, clean up.** `x` stops a session or a whole feature; a
stopped session stays listed and stays stopped when the feature starts again.
Stopping a feature also closes the VS Code window aiw opened for it, with its
language servers. `z` lists dormant features, meaning idle for an hour and
untouched for four, with per-row stop, close-editor, delete and open. `d`
deletes a project, feature or session. Deleting a feature with unfinished
TODOs asks where they should go first.

## 4. Every dashboard key

Every key below has a handler in the current source. `?` shows the same list
inside the app.

**Moving around**

| Key | Action |
| --- | --- |
| `j` / `k`, arrows | Move through projects, features and sessions |
| `Ctrl+j` / `Ctrl+k` | Jump to the next or previous feature |
| `h` / `l`, left / right | Collapse or expand the selected project or feature. `h` on a folded feature in a group folds the group; `l` on a folded group's row opens it |
| `Enter` | Put the keyboard in the selected running session's pane. On a project or feature row, expand or collapse; on a folded group's row, open the group. On a vanished session, open the resume dialog |
| `Ctrl+Q` | Flip the keyboard between the sidebar and the agent pane |
| `/` | Search and jump. Matches project, feature and session names, notes and group names. Every letter is a search character; arrows or `Tab` pick a result, `Enter` jumps (opening a folded group), `Esc` cancels |
| `?` | Show the keybinding reference |
| `q` | Quit the dashboard; the agents keep running. With `confirm_quit` on, the first press shows how many agents run and a second press within five seconds quits |
| `Esc` | Take back a pending quit confirmation. Nothing else on the dashboard |

**Projects and features**

| Key | Action |
| --- | --- |
| `N` | Create a project |
| `n` | Create a feature in the selected project. On its harness list, `Space` ticks several agents for one prompt (a fan-out) and `b` makes a single Claude "Claude (background)" (`claude --bg`); a Layout row (when the project defines `layout_presets`) opens extra sessions with the feature; the last step takes an optional task prompt (`Ctrl+L` library, `Ctrl+T` send when ready, `Alt+Enter` newline) |
| `B` | Create several features at once: a project path, a count, a name prefix, one harness and mode for all |
| `F` | Fork the selected feature onto a new branch, with its own harness and mode, optionally carrying context |
| `g` | On a project row, browse its open GitHub issues and start a feature from one |
| `u` | Set the project's preferred harness |
| `y` | Toggle the feature's ready mark, shown as a dim ✓ |
| `r` | Rename the selected feature or session |
| `d` | Delete the selected project, feature or session |
| `O` | Open aiw's own config directory as a project, so an agent can edit your settings |
| `G` | The project's feature groups: move the selected feature into one (or out), and create, rename, recolour, reorder and delete groups |
| `C` | Fold or unfold the selected feature's group |
| `#` | Write the note on the selected session or feature |

**Sessions**

| Key | Action |
| --- | --- |
| `s` | Add a session: an agent, Claude (background), terminal, Neovim, VS Code, TODOs or a custom session |
| `S` | Pick a saved Claude or Codex transcript to resume in the selected feature or session |
| `f` | Fork the selected Claude or Codex conversation. Choose a new window in the same feature, or a new feature worktree from this branch; the fork continues the conversation under a new id and the original keeps its own |
| `E` | List Claude sessions started outside aiw in your projects' checkouts. `Enter` copies the `claude attach` command for a background one |
| `c` | Start the selected feature or stopped session; relaunch an exited agent in its window |
| `x` | Stop the selected session or feature. The row stays |
| `v` | Tile the feature's agents in the right pane; on a fan-out sibling, the first agent of every sibling. `Alt+q` there comes back |
| `i` | List everything that needs you: questions first, then finished work |
| `a` / `X` | Allow or deny the permission the selected agent is waiting on |
| `e` | Queue a follow-up for the selected agent |
| `W` | List the worktrees Claude made for the selected feature (or, on a project row, the project) with its own `--worktree`, subagents or workflows. `t` opens a terminal there, `y` copies the path, `d` then `y` removes a clean one |
| `@` | Send a message to an agent session. A Claude session gets it straight in its inbox as its next prompt; any other agent gets it when its current turn ends. The dialog shows which; `↑`/`↓` pick the target |
| `Enter` / `c` / `x` / `d` on a background Claude row | Attach in the agent pane (`claude attach`; detaching sends it back) / restart it (`claude respawn`, or resume its conversation when Claude no longer lists it) / stop it (`claude stop`) / remove it (`claude rm`, asking first and showing Claude's refusal with the flag it suggests) |
| `V` | Reopen a pending Vibeless edit approval |
| `I` | On a TODOs row, start an agent on the next unstarted TODO |
| `z` | List dormant features |
| `R` | Refresh statuses and pull-request badges. This is the only key that runs `gh` on its own |

**Tools and settings**

| Key | Action |
| --- | --- |
| `L` | Open the prompt library |
| `A` | Manage harnesses: probe the CLIs, tick the ones to offer, `c` saves |
| `T` | Pick a theme |
| `p` | Pick a syntax parser for the diff viewer; installing one clones a tree-sitter grammar and says so first |
| `w` | Context window settings: an override for the assumed window size and the warning and critical percentages |
| `D` | View the debug log |

**`Ctrl+Space`, then a key: commands on the selected session**

| Key | Action |
| --- | --- |
| `d` | Diff viewer: all current changes, or one commit from the feature's history |
| `m` | Pick and read a Markdown file from the worktree |
| `l` | Past and unsent prompts. Pick one and it is pasted again, unsent. `y` copies it, `s` saves it as a template |
| `N` | Quick-capture a one-line TODO for this worktree |
| `z` | Mark the TODO this session was started from as complete |
| `/` | Command picker: the slash commands under `.claude/commands` in your home and in the repo, plus aiw's local actions. A slash command is typed into the agent's pane and run |
| `a` | The local actions alone: debug log, refresh, pending edit approval |
| `c` | Configuration wizard |
| `h` / `H` / `M` / `1`-`9` | Bookmarks: open the picker, bookmark the current session, remove it, jump to slot 1 to 9. Nine slots; the oldest is dropped |
| `r` | Refresh statuses and pull-request badges |

Keys in the TODO view and in the other overlays are listed in their own
sections.

## 5. Sessions, the info block, context and cost

**The info block.** When the sidebar is at least 24 rows tall its bottom
eight rows describe the selected session. The lines are the session title,
the harness and model, context use with cost and age, the branch, the
worktree path shortened to the project and feature folders, and the follow-up
and permission line such as `⏭ 2 queued · ✓ allowed Bash`. The title is
Claude's own session summary when it wrote one, else the first prompt; for
Codex and Gemini the first user message; else the aiw task prompt or the
feature name. Everything is read from the harness's own files on disk on a
five-second sweep in a background thread and cached per session.

**Context use.** Claude pipes its status line to aiw, which reports the model,
the percentage of the context window used, cost and rate limits. Codex's
rollout file gives the model and the last request's context. Gemini's newest
chat file gives tokens only. Cursor reports nothing usable. A session row
shows its percentage at the right edge in the warning colour past 70 percent
and the critical colour past 85 percent; both thresholds are settings. The
working badge reads `working 6m · 42% ctx · 2 subagents`.

**Row events.** Claude Code's newer hooks decorate the row's reason line, so
you can tell what a session did without opening it:

| On the row | What happened |
| --- | --- |
| `compacted` | the context was compacted |
| `model → sonnet` | the session switched model |
| `turn failed: overloaded_error` | the turn ended on an API error; shown with ⛔ until the next turn |
| `idle` | Claude's idle nudge: it has been waiting for you for a minute |
| `quota: paused`, `quota: resumed` | a usage-limit pause or auto-resume |
| `denied: Bash` | auto mode's classifier refused a tool call |
| `tasks 1/3` | the session's task list has open items (working rows and the info block) |
| `1 bg · 1 loop` | background tasks and `/loop` schedules still alive at the last turn end |

One-off events fade after ten minutes or when the next turn starts; a model
switch stays. Claude's `agent_needs_input` and elicitation dialogs count as
needing you. A feature whose session still has background work or a `/loop`
schedule is never listed as dormant. The info block's last line repeats the
tasks, background work, latest event, and `cd api` when the session moved to
another directory. Codex rows show working as soon as a prompt is submitted and
while a shell command runs, through Codex's own hooks; the turn still ends on
Codex's notify call.

**Names.** Every Claude session is launched with `--name`, a stable handle
made from its feature and label, such as `auth-fix-claude-1`. It is chosen on
the session's first launch, kept when you rename the row in aiw, and made
unique by adding the first characters of the session id when two would
collide. The info block shows it after the model as `@auth-fix-claude-1`.
Claude shows the same name in its prompt bar, in `/resume`, in `claude agents`
and to its other sessions. `claude --resume auth-fix-claude-1` reopens it from
any terminal. Codex has no launch-time name, so Codex sessions are unnamed.

**Forks.** `f` on a Claude or Codex row with a saved conversation opens a small
dialog. "Same feature, new window" adds a session labelled `Claude 1 fork` in
the same checkout. "New feature worktree from this branch" runs the `F` flow
with the harness fixed to the source's and the transcript checkbox replaced by
"continues 'Claude 1'". Either way the new session's first launch runs
`claude --resume <id> --fork-session` or `codex … fork <id>`: a new conversation
that starts from the old one, while the original keeps going. Later restarts
resume the fork's own conversation.

**Sessions outside aiw.** Claude Code keeps its own roster of sessions, which
`claude agents --json` prints. aiw reads it every ten seconds in the
background. A Claude session that runs in one of a project's checkouts but was
not started by aiw shows as a dimmed `↗ name · state` line under its feature,
or under the project when it runs in the repo but no feature's checkout. At
most two lines show per row, then `↗ +N more (E)`. `E` lists them all with
their state and folder; `Enter` copies `claude attach <id>` for a background
session. An interactive one runs in another terminal and can only be used
there. The same roster is a second needs-you signal for aiw's own sessions: if
Claude says one of them waits on a permission, a dialog or input that its hooks
did not report, the row turns ⛔ with the reason. The mark goes as soon as a
hook shows the agent running again. Set `claude_agents_roster` to
false to turn all of this off.

**Groups.** `G` opens the project's groups. With a feature selected, `Enter`
moves it into the highlighted group or back to "(no group)", and "+ New
group…" creates a group and moves the feature in at once. `n` creates a group,
`r` renames, `c` cycles its colour (six ANSI colours, so every theme shows
them), `J` / `K` move it down or up, and `d` deletes it after a `y`; its
features become ungrouped. Ungrouped features come first, then each group in
order, with a `▾ ■ Backend` header above its first feature and a coloured bar
beside each of its features and sessions. The header carries a ⛔ count when a
feature in the group needs you. `C` folds the group into one row, `▸ ■ Backend ·
3 features`, which keeps the ⛔ count; `Enter`, `l` or a search jump into it opens
it again. A fan-out group (one prompt, several agents) can sit inside a user group and keeps its own
`⇶` header under the group's. Groups and the fold state are saved in `aiw.db`.

**Notes.** `#` writes a free-text note on the selected session, or on the
feature when a feature row is selected. `Enter` saves, `Alt+Enter` adds a line,
`Ctrl+U` clears and saving an empty note deletes it. A row with a note shows
`✎`. The info block shows the first line of the session's note, or the
feature's when the session has none, on one extra row. `/` matches note text
and shows the matching line. Notes are saved in `aiw.db` and removed with
their session or feature.

**Messages.** `@` opens a message dialog addressed to the selected agent
session (or the selected feature's first agent); `↑`/`↓` or `Tab` pick another
target, `Enter` sends, `Alt+Enter` adds a line. How the message travels depends
on the target, and the dialog says which before you send:

- **A Claude session** reports its inbox when it starts. The message goes
  straight into that inbox and Claude takes it as its next prompt, marked as a
  message from `aiw`. Nothing is typed into the pane, and a turn in progress is
  not interrupted.
- **Codex and other agents**, or a Claude session that has not reported an
  inbox, get the message through the follow-up queue: it is delivered when the
  agent next finishes a turn, never typed into a busy pane.

Scripts use `aiw automation message --to <session> --text "…" [--from <name>]`.
`--to` takes a session id, a Claude session's name (`<feature>-<label>`, the
same name Claude's own session list shows), `<feature>/<label>`, or a label that
is unique across aiw. The JSON reply says `delivered`, `queued` or `failed` and
why. Because every Claude session aiw starts carries its aiw name, agents can
also message each other by name with Claude's own messaging.

aiw sets Claude's rule for incoming messages on each launch from
`claude_cross_session_inbound` (default `accept`). `accept` delivers messages
from aiw and your other local sessions without an approval dialog; the inbox is
only reachable by your own user, and permission prompts still apply to whatever
a message asks for. SuperVibe sessions keep Claude's own rule, which holds such
messages for your approval, because a session that skips every permission
prompt should not take instructions from another agent unasked.

**Claude's own worktrees.** Claude Code makes worktrees by itself: when you
start it with `--worktree`, when it moves into one, when a subagent runs with
`isolation: worktree`, or when a workflow or background session isolates its
edits. On its own it puts them in `<repo>/.claude/worktrees/`, without your
`.env` and without a port block. Every Claude session aiw starts carries a
worktree hook instead, so those worktrees are created the way aiw creates its
own:

- in aiw's worktree folder, next to the feature worktrees, as
  `claude-<name>` on Claude's own branch name `worktree-<name>`;
- branched from the feature's current commit, so a subagent works on the
  feature rather than on `main`;
- with the repository's included files copied (`.worktreeinclude` or the
  `worktree_include` setting, which Claude itself skips when a hook makes the
  worktree);
- with their own block of ports, written to `.aiw/ports.env` inside the
  worktree (git ignores it).

Each one appears under its feature as a dimmed `⎇ <name> · subagent` or
`⎇ <name> · worktree` line, up to three and then `+N more (W)`; a toast says
when Claude made one. aiw records it even when the dashboard is closed and
shows it at the next start. `W` lists them: `t` opens a terminal session in
the feature that changes into the worktree with its ports exported, `y`
copies the path, and `d` then `y` removes it. A removal, whether Claude asks
for it when a worktree session ends or you press `d`, only happens when the
worktree has no uncommitted or untracked files; otherwise Claude is told why
and the worktree stays. The branch is deleted with it unless it holds commits,
in which case it is kept so the work stays reachable. Worktrees a finished
subagent leaves behind stay until you remove them, as with Claude's own
default. The hook only rides on the settings aiw passes at launch, so a
`claude` you start yourself is unaffected. `claude_worktree_hooks: false`
turns it off.

**Background Claude sessions.** A session added as "Claude (background)"
runs under Claude Code's supervisor (`claude --bg`) with the same settings,
hooks, name, mode and context as an interactive Claude. It has no pane until
you attach, so a feature whose only agent runs in the background gets no tmux
session at all. Its row shows `bg ·` before the state Claude reports for it:
`bg · working`, `bg · permission prompt` (it needs you), `bg · done`,
`bg · stopped · Enter resumes`, or `bg · not running · c resumes` when Claude
no longer lists it. Its hooks still reach aiw, so a question shows at once.
The info block reads "Claude Code (background)" and its last row shows the
short id, the state and the latest line of its output.

- `Enter` attaches: aiw opens `claude attach <id>` in a window of the feature
  and focuses it in the agent pane; typing goes to the session. Detach in
  Claude and the window closes itself; the session keeps running.
- `x` stops it (`claude stop`); its conversation is kept, and attaching
  resumes it.
- `c` restarts it (`claude respawn`), or, when Claude no longer lists it,
  resumes its conversation in a new background session.
- `d` asks, then runs `claude rm`. When Claude refuses (uncommitted work,
  unpushed commits), the dialog shows Claude's words and the flag Claude
  suggests; a second `y` runs `rm` with that flag and discards the work.

Stopping the feature leaves its background sessions running; deleting the
row or the feature stops them in Claude. `claude_background_sessions: false`
hides the option.

**Layout presets.** A project's `aiw.json` (or the global `extension` block)
can name sets of extra sessions to open with a feature:

```json
"layout_presets": [
  { "name": "full", "description": "agent, watcher, server",
    "sessions": [
      { "kind": "terminal", "label": "watch", "command": "npm test -- --watch" },
      { "kind": "custom", "name": "server" },
      { "kind": "agent", "agent": "codex" },
      { "kind": "nvim" }
    ] }
]
```

`kind` is `agent` (with `agent`), `terminal` (with an optional `command`, typed
into the new shell), `nvim` or `custom` (with `name`, one of `custom_sessions`).
`label` names the session. The New Feature dialog shows a Layout row when any
preset exists; `j` / `k` pick one. A feature preset can name one with
`"layout": "full"`, and `aiw automation create-feature` takes `"layout"`. The
preset is checked before anything is created: an unknown kind, harness or
custom session name stops the dialog (or the request) with a message naming
the preset and the entry. Its sessions open once the feature runs, after its
own agents; with one worktree per agent every sibling gets them. A session
that cannot open (Neovim not installed, a failing `pre_check`) is reported and
the rest still open.

**Cost.** Claude's status line carries the dollar figure. For harnesses that
report tokens but not cost, aiw multiplies by the prices in the
`token_pricing` setting, which defaults to Claude's published rates. Set
`show_cost` to false to hide it.

**Usage bars.** The status panel at the bottom of the dashboard shows the
five-hour and seven-day usage windows for Claude, from the rate-limit numbers
Claude's own status line hands over, and the daily counts from Claude's local
stats cache. Codex windows come from its local session files. A z.ai plan can
be described in the `zai` setting so its limits show the same way. No usage
API is called.

**The status panel.** The three-row box at the bottom of the dashboard shows
the last action's result in green, or an error in red, until the next action.
With nothing to report it shows the selected session's label, harness,
feature and directory, or the project and feature counts, with the usage
bars on the right. Its bottom border carries the key hints for the current
context.

**Subagents and launch context.** Subagent start and stop hooks keep a count
per session on the working badge. At launch every agent is told its project,
feature, branch, worktree and aiw session id, and asked to mention the branch
name in commit messages and pull-request titles. Claude gets it through an
appended system prompt, Codex through developer instructions, Gemini and
Cursor through their session-start hooks. The `agent_context_prompt` setting
turns it off.

## 6. TODO lists

Add a TODOs session to a feature with `s`. It opens as a native overlay, not a
tmux window. Three lists can sit side by side.

| List | Holds |
| --- | --- |
| Worktree | Work for this feature's own checkout. A feature on the repository root has no worktree list |
| Project | Work for the project as a whole |
| Global | Work for no project at all, shared across every repository aiw knows |

Keys in the TODO view: `j` / `k` move, `a` or `n` add, `e` edits the title,
`o` the notes, `b` the scratchpad banner above the list. `Space` or `x`
cycles an item from todo to in progress to done. `P` cycles priority through
high, medium and low. `J` / `K` reorder. `d` deletes after a confirmation.
`p` hides or shows the project list and `g` the global list; the choice
lasts until aiw exits. `Tab` and `Shift+Tab` move between visible lists.
`M` moves an item to another list and keeps its links and state; `C` copies
it as fresh, unstarted work. `Ctrl+T` while editing switches the editor to
Vim keys. `q`, `Esc` or `Ctrl+Q` leaves the view.

`Enter` on an item starts work on it. For a worktree TODO the item is pasted,
unsent, into an agent in this feature. For a project or global TODO aiw asks
which feature should take it. You can also start a new feature from the
item; the branch name is suggested from the title, and the item is linked to
the feature so `Enter` afterwards jumps there. `I` works the queue instead:
it takes the highest-priority unstarted item across the visible lists,
narrowest scope first, starts an agent on it and marks it in progress. When
every item is already underway aiw offers the next one and asks whether to
go to the work already started, start a second agent, skip it or cancel.

A TODO remembers the session that works it. If that session dies the link is
dropped but the in-progress mark stays, because a missing agent does not make
the work unstarted. Deleting a feature with unfinished worktree TODOs asks
whether to move them to the project list, move them to the global list,
delete them with the worktree, or cancel. If the deleted feature hosted the
project list, aiw asks which surviving feature should host it.

## 7. Prompt library and past prompts

Press `L` for the prompt library. A template has a name, a description, a
body and tags, and may contain `{{placeholder}}` slots. Each slot is a text
field, a multi-line field or a select menu, optional or required. Picking a
template fills the slots, then pastes the result into the selected agent's
pane unsent. Templates you write are stored in the database. A project can
also ship templates in its config, and the library shows where each one came
from. `n` adds a template, `e` edits it, `y` duplicates a shared one into your
own, and the export keys copy a template into the global config, the
project config or the worktree config.

`Ctrl+Space` then `l` lists the prompts aiw pasted into the selected session
and any prompt that could not be delivered because the agent was not
running. `Enter` pastes the selected prompt again, unsent. `y` copies it to
the clipboard. `s` saves it as a library template. The last pasted prompt is
also written to `.claude/latest-prompt.txt` in the checkout, which the info
block falls back to for the title.

Prompts are never sent on their own. Every path that prepares text for an
agent, a TODO, a template, a feature's startup prompt or a past prompt,
pastes it and focuses the pane so you can read, edit and press `Enter`.

## 8. GitHub: issues and pull-request badges

Both features need an authenticated GitHub CLI and a GitHub remote. Nothing
runs in the background; `gh` runs only when you press a key that needs it.

**Issue browser.** `g` on a project row lists the repository's open issues.
`h` / `l` page, `r` refreshes. Pick an issue and aiw opens a setup dialog
with a feature name, the harness, mode and worktree choice, and an editable
prompt built from the issue's title, body and comments. The issue text is
treated as data and is never passed to a shell. Confirming creates the
feature and pastes the prompt into its agent, unsent. If a feature already
tracks the same issue aiw warns before creating another.

**Pull-request badges.** A feature whose branch has a pull request shows
`[PR #N · M open]` on its row, with the number of open review threads. Once
the PR is merged or closed the badge reads `[PR #N merged]` or
`[PR #N closed]` and stays, so a finished feature looks different from one
that never had a PR. Badges refresh when you press `R`, or `Ctrl+Space` then
`r`, and otherwise show the last known state. Reviewing the pull request is
the agent's job inside its session.

## 9. Configuration

Global settings live in `~/.config/ai_workstation/config.json`. A repository
can add `aiw.json` at its root; project settings override matching global
ones. The configuration wizard, `Ctrl+Space` then `c`, edits the common
parts of both: custom sessions, feature presets, lifecycle hooks, keybindings
and allowed agents. Unknown keys in an existing config are ignored and never
written back, so a config from an older version keeps loading.

**config.json keys and defaults**

| Key | Default | What it does |
| --- | --- | --- |
| `theme` | `default` | The colour theme; `T` changes it |
| `theme_follow_terminal` | `false` | Pick `theme_dark` or `theme_light` from the terminal's background at startup |
| `theme_dark` / `theme_light` | `ember` / `paper` | The themes used when following the terminal |
| `transparent_background` | `false` | Let the terminal's background show through |
| `color_mode` | `auto` | `auto`, `truecolor` or `256` |
| `nerd_font` | `true` | Use icon glyphs; set false for ASCII fallbacks |
| `nerd_font_confirmed` | unset | Whether the first-run icon check was answered |
| `sidebar_width` | `36` | Width of the sidebar pane in columns, minimum 28 |
| `confirm_quit` | `false` | Make `q` ask first while agents are running |
| `leader_timeout_seconds` | `5` | How long `Ctrl+Space` waits for its second key |
| `permission_wait_seconds` | `300` | How long a permission hook waits for your answer |
| `agent_context_prompt` | `true` | Tell each agent its project, feature, branch and worktree at launch |
| `claude_hooks_via_settings_flag` | `true` | Pass Claude's hooks with `--settings` instead of writing the repo's settings file |
| `claude_agents_roster` | `true` | Read `claude agents --json` every ten seconds to show Claude sessions outside aiw and catch a session waiting on input that its hooks missed |
| `fan_out_ready_fallback_secs` | `15` | How long a task prompt waits for its agent's session-start hook before it is pasted anyway, once the pane runs the agent |
| `messaging_inbox` | `true` | Deliver `@` messages and `aiw automation message` straight into a Claude session's inbox. Off: every message goes through the follow-up queue |
| `claude_worktree_hooks` | `true` | Pass Claude a worktree hook so the worktrees it makes itself (`--worktree`, subagents, workflows) are created in aiw's worktree folder with the included files and a port block, and listed under their feature |
| `claude_background_sessions` | `true` | Offer "Claude (background)" (`claude --bg`, attached on demand) in the `s` picker and as `b` on the New Feature harness list. The default harness stays interactive |
| `claude_cross_session_inbound` | `"accept"` | Claude's rule for messages from your other sessions and aiw: `accept`, `hold` (approve each in the pane), `refuse`, or `default` to leave Claude's own rule. `accept` is not applied to SuperVibe sessions |
| `max_concurrent_agents` | `4` | Warn before starting an agent past this many running harnesses; 0 disables |
| `low_memory_warn_mb` | `1536` | Warn before starting an agent with less free memory than this; 0 disables |
| `max_agent_autostart_sessions` | `1` | How many saved agent sessions a feature start relaunches; the rest open at a shell |
| `kill_editor_on_stop` | `true` | Close the VS Code window aiw opened when its feature stops |
| `dormant_idle_minutes` | `60` | A feature is dormant after this long idle and… |
| `dormant_last_accessed_hours` | `4` | …this long since you last opened it. Both must hold; 0 disables |
| `waiting_stale_minutes` | `30` | How long a needs-you state stays before ageing to idle; 0 keeps it |
| `context_window_override` | unset | Assume this context window size when a harness does not report one |
| `context_warning_percent` / `context_critical_percent` | `70` / `85` | Colour thresholds for context use |
| `token_pricing` | Claude list prices | Per-million prices for input, output, reasoning, cache read and cache write, plus `show_cost` |
| `worktree_base_dir` | unset | Where new worktrees go; the default is aiw's data directory |
| `worktree_include` | `[".env*"]` | Ignored files to copy into a fresh worktree when the repo has no `.worktreeinclude` |
| `diff_viewer_layout` | `unified` | `unified` or `split` for the diff viewer and the edit popup |
| `diff_review_popup_hold_secs` | `1.5` | How long the edit popup ignores `Enter` after opening |
| `diff_review_viewer` | `aiw` | Which viewer shows a held edit |
| `input_request_wait_seconds` | `1.5` | How long a generic input request waits before it is shown as needing you |
| `tmux_control_mode` | `false` | Used by the desktop GUI's terminal only |
| `projects.default_preferred_agent` | unset | The harness new features default to |
| `zai` | unset | A z.ai plan name and its token limits, for the usage bars |
| `extension` | empty | The same block `aiw.json` holds, applied to every project |

**aiw.json**

```json
{
  "allowed_agents": ["claude", "codex"],
  "feature_presets": [
    { "name": "quick fix", "branch_prefix": "fix/", "agent": "claude", "mode": "vibe", "enable_chrome": false },
    { "name": "duel", "agents": ["claude", "codex"], "fan_out": "worktree_each", "mode": "vibe" },
    { "name": "web", "agent": "claude", "mode": "vibe", "layout": "dev" }
  ],
  "layout_presets": [
    { "name": "dev", "sessions": [
      { "kind": "terminal", "label": "tests", "command": "npm test -- --watch" },
      { "kind": "custom", "name": "dev" }
    ] }
  ],
  "custom_sessions": [
    { "name": "dev", "description": "Vite dev server", "command": "npm run dev",
      "working_dir": null, "icon": "🚀", "on_stop": "pkill -f 'npm run dev'",
      "autolaunch": false, "pre_check": "which npm" }
  ],
  "lifecycle_hooks": {
    "on_start": "scripts/aiw-start.sh",
    "on_stop": null,
    "on_worktree_created": {
      "script": "scripts/aiw-setup.sh",
      "prompt": { "title": "Seed the database?", "options": ["yes", "no"] }
    }
  },
  "keybindings": { "quit": "Q" },
  "prompt_templates": [
    { "name": "Explain this file", "body": "Explain {{path}} to a new teammate.",
      "tags": ["docs"], "placeholders": [{ "key": "path", "kind": "text", "required": true }] }
  ]
}
```

Custom sessions appear in the `s` picker with their icon and description. A
session with `pre_check` is skipped when that command fails, and `on_stop`
runs when you stop it from aiw. Lifecycle hooks run when a feature starts,
stops, or gets a new worktree; a hook with a prompt asks you to pick an option
first and passes it to the script in the `AIW_HOOK_CHOICE` variable. A hook
that comes from a repository runs only after you approve it once, pinned to
the exact command and script contents, so cloning a repository cannot run
code on your machine. Hooks in your own global config never ask.

Remappable actions for `keybindings` are `quit`, `create_project`,
`create_feature`, `issue_fixer`, `start_session`, `stop_session`, `delete`,
`sessions`, `help`, `search`, `refresh`, `syntax_picker`, `session_config`,
`fork_feature` and `mark_ready`.

**Appearance.** Thirty-three themes ship: Default, Ember, Graphite, Midnight,
Paper, Porcelain, Classic, Dracula, Nord, four Catppuccin flavours, Gruvbox
Dark and Light, and eighteen Gruvbox Material variants. `T` opens the picker,
grouped by light and dark. With `theme_follow_terminal` on, aiw reads the
`COLORFGBG` variable or asks the terminal for its background colour before it
starts and picks the dark or light theme. Fonts belong to the terminal; `aiw
fonts` prints a ready-to-paste font and icon snippet for Ghostty, iTerm2,
WezTerm, Kitty or Alacritty and installs nothing.

**Resource limits.** Before starting an agent, aiw counts running harnesses
across every project and checks free memory. If either limit is past, one
dialog says which and `y` starts anyway. It never refuses. Terminals, editors
and TODO sessions are not counted. Creating a feature never raises the
dialog; the feature is created stopped instead.

## 10. Supported harnesses

| | Claude Code | Codex | Gemini CLI | Cursor agent | Pi |
| --- | --- | --- | --- | --- | --- |
| Binary | `claude` | `codex` | `gemini` | `agent` | `pi` |
| Launch | binary plus mode flags, `--settings` with aiw's hooks and status line, `--append-system-prompt` with the launch context | `codex -C <dir> --add-dir <dir>` plus mode flags and `-c` overrides that add the notify hook and developer instructions | `gemini --approval-mode …` | `agent` with `--force` for SuperVibe | `pi` |
| Needs-you and finished badges | ✓ question, completed, waiting | ✓ waiting only: one hook fires at turn end and cannot tell a question from a completion | ✓ all three | ✓ working and completed; questions through the screen | none; Pi has no hooks |
| Answer permissions from the dashboard | ✓ | ✓ | no; its hooks only observe | ✓ shell and MCP calls | no |
| Vibeless edit approval | ✓ | refused | no | no | no |
| Context and cost on the row | ✓ status line | ✓ rollout file | tokens only | no | no |
| Queued follow-ups | ✓ | ✓ | ✓ | ✓ | no |
| Subagent count | ✓ | ✓ | no | ✓ | no |
| Launch context | system prompt | developer instructions | session-start hook | session-start hook | no |
| Resume a saved session | `--resume <id>`, picker with `S` | `codex resume <id>`, picker with `S` | `--resume <id>` | `--resume <id>` | no |
| Where aiw's hooks go | a per-session settings file under aiw's state directory, passed with `--settings` | `.codex/hooks.json` and the notify hook per launch | `.gemini/settings.json` | `.cursor/hooks.json` | nowhere |

How the hooks work: each harness's hook points at a small shell script under
aiw's hooks directory. The script calls an `aiw` subcommand with the hook's
JSON on standard input, and that subcommand writes one line to aiw's local
Unix socket. The shell never parses JSON. A hook that needs an answer keeps
its connection open until you decide or its timeout passes.

One-time setup: Codex asks you to trust aiw's hooks once per worktree with its
own hooks command, and again after an aiw update that changes them. Cursor runs project hooks only in a trusted workspace.
Gemini cannot grant permissions from a hook, so add safe commands to its own
allowed-tools setting to stop being asked.

aiw never edits a harness's global configuration. Claude's hooks travel in a
`--settings` file by default, so nothing of aiw's is written into the
repository's `.claude/settings.local.json`; set the flag to false to go back
to writing that file. Codex gets its notify hook per launch rather than in
`~/.codex/config.toml`.

## 11. Command line and automation

| Command | What it does |
| --- | --- |
| `aiw` | Open the cockpit, or attach to the one already running |
| `aiw grid <feature>` | Tile a feature's agents as live tmux panes in this terminal. Use `project/feature` when the name is ambiguous, `-p` to treat the name as a project, `--with <feature>` (repeatable) to tile the first agent of several features side by side, `--all` to include terminals, `--list` to print what would be shown. `Alt+q` closes it |
| `aiw doctor [--json]` | Read-only health report: agents against the limit, editor windows, memory and swap, orphaned `aiw-` tmux sessions and worktrees, editors left running for stopped features, projects still configured by an old `amf.json`, the hook mode, and a line confirming no network use. Changes nothing, exits 0 |
| `aiw fonts [--all] [--font …] [--terminal …]` | Print the detected terminal, colour depth and icon settings and a font config snippet |
| `aiw automation create-project` | Register a directory as a project from a JSON request |
| `aiw automation create-feature` | Create a feature in an existing project from JSON: project name, branch, agent, mode, worktree yes or no, an optional hook choice, and a dry-run flag. `agents` with `fan_out` (`shared` or `worktree_each`) starts several agents; `prompt` or `prompt_file` gives each one a task prompt once it is ready, and `send` sends it instead of pasting it unsent. `layout` opens a layout preset's sessions in each created feature. The answer lists every feature created |
| `aiw automation create-batch-features` | Create one project with many parallel feature worktrees |
| `aiw automation queue --session <id> "text"` | Queue a follow-up for a session; the id is in the doctor's JSON output |
| `aiw automation message --to <session> --text "text" [--from <name>]` | Message a session: into a Claude session's inbox, otherwise queued for its next turn end. Prints `delivered`, `queued` or `failed` |
| `aiw --help` | Everything else |

Automation talks to the running dashboard over its socket and prints a JSON
response. `--dry-run` validates without changing anything and, for a
repository whose worktree hook asks a question, returns the options so a
script can answer on the real call. Request templates and examples are in
`docs/automation`.

The hook subcommands that scripts call are hidden from the help: `notify`,
`notify-wait`, `hook-payload`, `hook-field`, `permission-request`,
`status-line`, `stop-hook`, `subagent`, `hook-event`, `worktree-hook`, `session-context`, `set-status` and
`sidebar`, which is the dashboard process the cockpit runs.

One dashboard per config directory: a second `aiw` attaches to the same
cockpit, and the dashboard holds a lock so a second dashboard process on the
same config prints who holds it and exits. The other subcommands never take
the lock.

## 12. What aiw writes on disk, and the no-cloud guarantee

| Where | What | When |
| --- | --- | --- |
| `~/.config/ai_workstation/` | `config.json`, the database `aiw.db`, port reservations, the instance lock, the hook scripts under `hooks/` | first launch |
| `~/.local/state/ai_workstation/` on Linux, `/tmp/ai_workstation/` on macOS | the private tmux socket, the cockpit socket, the IPC socket, `debug.log`, per-session Claude settings files, thinking markers, quarantined bad notifications | first launch |
| `~/.local/share/ai_workstation/worktrees/` on Linux, Library/Application Support on macOS | feature worktrees, and the `claude-<name>` worktrees Claude makes through aiw's hook, unless `worktree_base_dir` says otherwise | when a feature starts, or when Claude makes a worktree |
| inside each repository | `.aiw/`, `.claude/latest-prompt.txt`, `.claude/skills/aiw-*`, `.agents/skills/aiw-*`, `.codex/`, `.gemini/`, `.cursor/hooks.json` | when a feature starts |

Every file aiw puts inside a repository is added to that repository's
`.git/info/exclude`, so it can never be committed. Each feature also gets a
block of ten TCP ports, reserved under a lock and passed to every window as
`AIW_PORT` and `AIW_PORT_COUNT`, so parallel dev servers do not fight over
one port. The database keeps projects, features, sessions, bookmarks, TODOs,
prompt templates, follow-ups, hook approvals, token caches and the debug log.
Tables that belonged to removed features stay in the schema, unread, so an
older database opens unchanged.

aiw injects a few skills into Claude Code and Codex workspaces so an agent can
edit the project-local config safely: `aiw:configure` explains the current
setup, `aiw:add-session`, `aiw:add-hook`, `aiw:add-preset` and
`aiw:add-prompt` add the matching entries, and `aiw:release-notes` keeps the
changelog in the house style.

**No cloud.** aiw makes no network connections. No HTTP client or server
library is linked into the binary, and `scripts/check-no-network.sh` checks
the dependency tree and the sources. The agents talk to their own providers.
`git` runs for branches and worktrees, `gh` runs only when you press a key
that needs GitHub, and installing a syntax parser clones a grammar only
after telling you so. There is no self-upgrade: pull the fork and rebuild.

To remove everything: delete each feature with `d`, quit, kill the private
tmux server, and delete the three directories above.

## 13. The desktop GUI

`gui/` holds a Tauri desktop application, `aiw-gui`, built from this
repository. It reads the same database and the same tmux sessions as the
terminal dashboard, so both can run at once. It covers project and feature
creation, every session kind the picker offers, terminal tabs with
scrollback, the TODO lists, the prompt library, a prompt composer with
per-session drafts, the diff viewer, Vibeless edit approvals, dormant
features, a refresh button for pull-request badges, themes and the debug
log. Agent tabs have a sidebar with status, usage, context, issue and pull
request, prompt and TODO sections. It has no cockpit; prompts it prepares
land in its composer instead of a pane. Build it with `npm ci` and `npm run
tauri build` inside `gui/`; the GUI README has platform notes for macOS and
Windows under WSL2.

## 14. Removed features and their native replacements

All of these were deleted on 2026-10-11. Their database tables remain in the
schema, unread.

| Removed | Use instead |
| --- | --- |
| Remote Control flag and dialog row | Claude's own remote-control command |
| Steering prompt, the "prompt coach" box after launch | Type into the agent's pane; prompts are pasted unsent |
| Learning mode: a read-only repository browser with questions answered by a model | Ask the agent in its session |
| Model and reasoning analyzer | The harness's own model and effort commands |
| Plan interview and Quick Plan, with `AIW_PLAN.md` | Claude Code's plan mode, Codex's plan mode |
| AI review of a pull request, review memory, review destination and review questions | `/code-review` in Claude Code, `/review` in Codex |
| Final Review viewer, walkthroughs, co-review, batched review, PR submission, and review mode with its CLAUDE.md note block | The same review commands inside the agent |
| PR Triage and PR review: comment reader, reply drafts, investigations, companion features, and the screenshot-evidence flow | `gh pr view --comments` and a fix in the agent's own session |
| Session summaries, the fresh-context handoff, editable headless prompts and the headless runner | `/compact`, `--resume`, the harness's own prompts |

What stays is listed in sections 2 to 13: projects, features, worktrees and
sessions; the cockpit and the grid; badges and the needs-you list; queued
follow-ups; dashboard permission answers and the Vibeless edit popup; hooks
and the status line; context, cost and usage; dormancy; TODOs; bookmarks;
the prompt library; the diff and Markdown viewers; the issue browser; the
pull-request badge refresh; the configuration wizard, harness setup, themes,
doctor, fonts, automation and the IPC socket.
