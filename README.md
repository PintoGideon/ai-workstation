
# AI Workstation (AIW)

Based on [agent-mainframe](https://github.com/eldridgerdev/agent-mainframe) by Ryan Eldridge (MIT). See
[LICENSE](LICENSE) for the original notice, [FORK.md](FORK.md) for how this fork is maintained and
[DESIGN.md](DESIGN.md) for what it adds.

Run multiple AI coding agents in parallel—each on its own branch and in its
own terminal—without losing track of them.

AIW is a terminal dashboard for managing
[Claude Code](https://docs.anthropic.com/en/docs/claude-code),
[Codex](https://github.com/openai/codex), Gemini CLI, Cursor agent and Pi
sessions. It organizes work by project
and feature, creates isolated git worktrees when needed, and shows which
agents are asking a question and which have finished work waiting to be
reviewed.

<img width="1896" height="1030" alt="AIW dashboard showing several agent sessions" src="https://github.com/user-attachments/assets/d8160bc6-49ea-4b2b-839a-7ec056897ffc" />


## What AIW does

- Runs several coding-agent sessions side by side from one dashboard.
- Keeps concurrent features isolated with git branches and worktrees.
- Embeds agent terminals, shells, Neovim, VS Code, and custom sessions.
- Surfaces which agents need attention, session status, token usage, and
  estimated cost.
- Answers Vibeless per-edit approvals from the dashboard and shows each
  feature's pull-request badge.
- Provides reusable prompts, scoped TODO lists, themes, lifecycle hooks, and
  workspace presets.
- Supports ordinary directories as well as git repositories.

## Requirements

Install at least one supported agent CLI and sign in to it before using AIW:

- [Claude Code](https://docs.anthropic.com/en/docs/claude-code)
- [Codex](https://github.com/openai/codex)
- [Gemini CLI](https://github.com/google-gemini/gemini-cli)
- [Cursor agent](https://cursor.com/cli)
- Pi (`pi` must be available in `PATH`)

AIW is installed from source and requires `tmux` in `PATH`. Git is required for branch and worktree features, but AIW can manage
non-git directories without it.

Optional tools:

- An authenticated [GitHub CLI](https://cli.github.com/) (`gh`) enables the
  issue browser and the dashboard's open/merged/closed PR badge.
- A [Nerd Font](https://www.nerdfonts.com/) provides the best icon rendering.
- The `code` command enables VS Code sessions.
- A C compiler and git are required only for installing optional tree-sitter
  syntax parsers.

## Installation

AIW is built from source; there are no release bundles. Building needs Rust
(via [rustup](https://rustup.rs)) with the 1.99.0 toolchain, a C compiler, git
and `tmux`.

**One-time setup**

```bash
# macOS
xcode-select --install && brew install tmux
# Ubuntu / Debian
sudo apt install build-essential tmux git

rustup toolchain install 1.99.0
```

**Build and install** (run from any terminal):

```bash
cd ~/ai_workstation                                # your clone of this fork
git switch ai-workstation                          # the branch with the AIW work
cargo +1.99.0 install --path . --locked --force    # installs `aiw` into ~/.cargo/bin
aiw -V                                             # AI Workstation (aiw) 0.49.0 (<commit>)
```

Make sure `~/.cargo/bin` is in your `PATH`.

### Run it in a new terminal

AIW is an interactive TUI, so open a **new terminal window** (Ghostty, iTerm2,
WezTerm, Kitty or GNOME Terminal), not an editor's output panel. It runs its
own private tmux server, so it works the same from a plain shell or from
inside your own tmux.

```bash
aiw doctor     # optional: read-only report of what AIW has on this machine
aiw            # open the dashboard
```

The first launch copies an existing AMF setup (projects, settings) into AIW's
own folders and leaves AMF untouched; see "Coming from AMF" below. Then follow
the [Quick start](#quick-start).

Other commands:

| Command | What it does |
| --- | --- |
| `aiw grid <feature>` | Show a feature's agents as live tiled panes in this terminal (`project/feature` when ambiguous; `-p <project>` for one agent per feature; `--all` adds shells). `Alt+q` closes the grid; agents keep running |
| `aiw fonts` | Print a font + icon config snippet for your terminal (`--terminal ghostty\|iterm2\|wezterm\|kitty\|alacritty`, `--font jetbrains\|geist\|iosevka\|fira`) |
| `aiw doctor [--json]` | Read-only health report: running agents, memory and swap, orphaned tmux sessions, worktrees and editors, the no-network check. Changes nothing |
| `aiw automation …` | Machine-friendly actions against a running AIW (see [Automation](#automation)) |
| `aiw --help` | Everything else |

Closing the terminal does not stop your agents: they keep running in AIW's
tmux server. Run `aiw` again to pick up where you left off.

For a contributor setup, focused tests and CI commands, see the
[development checks](docs/development/checks.md) and the
[architecture guide](docs/development/architecture.md):

```bash
cargo +1.99.0 build --locked
cargo +1.99.0 test --locked
cargo +1.99.0 clippy --all-targets --locked
scripts/check-no-network.sh                        # the app makes no network calls
```

For a container installation, see the
[Docker guide](docs/docker-no-tmux.md).

### Upgrade

AIW (this fork) has no self-upgrade: it never downloads anything itself. Pull
the fork and rebuild; running agents are not affected. See
[NO_CLOUD.md](NO_CLOUD.md).

```bash
cd ~/ai_workstation && git pull
cargo +1.99.0 install --path . --locked --force
```

See [CHANGELOG.md](CHANGELOG.md) for release notes and migration guidance.

### Coming from AMF

AIW keeps everything in its own folders, so it runs side by side with an
agent-mainframe (AMF) install without touching it:

| | AMF | AIW |
| --- | --- | --- |
| command | `amf` | `aiw` |
| config + database | `~/.config/amf/` (`amf.db`) | `~/.config/ai_workstation/` (`aiw.db`) |
| hook scripts | `~/.amf/hooks/` | `~/.config/ai_workstation/hooks/` |
| tmux socket, debug log | `~/.local/state/amf/` (macOS `/tmp/amf/`) | `~/.local/state/ai_workstation/` (macOS `/tmp/ai_workstation/`) |
| env vars, tmux sessions | `AMF_*`, `amf-*` | `AIW_*`, `aiw-*` |
| repo files | `amf.json`, `.amf/`, `AMF_PLAN.md` | `aiw.json`, `.aiw/`, `AIW_PLAN.md` |

**First launch.** When `~/.config/ai_workstation` does not exist yet and
`~/.config/amf` does, AIW *copies* (never moves or edits) AMF's settings,
database, port reservations, custom themes and syntax parsers
into its own folder, and says so once in a toast. The database is copied only
while AMF is not writing to it (a busy AMF gets a few retries; if it keeps
writing, AIW stops and asks you to start it again) and is integrity-checked.
AMF's pending notifications, its OpenCode plugins and its generated helper
scripts are not copied. Projects and features from AMF show up on the
dashboard; old settings such as `"theme": "amf"` keep working.

**Old features.** They keep their `amf-…` tmux session names, but those
sessions live on AMF's tmux server, so in AIW they show as **stopped**. Start
one (or use `S` to resume a saved Claude/Codex conversation) and AIW recreates
it under the same name on its own server. Stop it in AMF first: otherwise the
same agent runs twice in one worktree. Starting a feature in AIW also rewrites
the worktree's Claude hooks to point at AIW, so an AMF session still running
in that worktree stops reporting to AMF.

**Repo config.** A repo without `aiw.json` is read from AMF's `amf.json` (or
the older `.amf/config.json`). The first time AIW saves project config it
writes `aiw.json` and leaves `amf.json` alone for AMF.

## Quick start

1. Start AIW from a terminal window:

   ```bash
   aiw
   ```

   `aiw` builds a small tmux **cockpit** for itself and attaches your terminal
   to it: the session sidebar on the left, the selected agent's real terminal
   on the right. Closing the terminal window stops nothing — the dashboard
   and the agents keep running, and the next `aiw` attaches to the same
   dashboard. `q` in the sidebar shuts the dashboard down (the agents stay).

2. On first launch, AIW probes every supported agent CLI and ticks the ones
   it finds on your `PATH`; untick any you do not want and confirm. `A`
   reopens this screen later: it shows what is installed, and `Enter` ticks
   or unticks a row without changing your earlier choices on its own.

3. Press `N` to add a project, then enter its name and directory. The directory
   may be a git repository or an ordinary folder.

4. Press `n` to create a feature. Choose a branch name, agent, and permission
   mode. The feature starts when setup is complete.

5. Select the agent session: it appears in the right pane as you move. Press
   `Enter` (or `Ctrl+Q`) to put the keyboard in that pane — typing is native,
   the same as `tmux attach`. `Ctrl+Q` brings the keyboard back to the
   sidebar.

6. Press `s` on a feature to add another agent, terminal, editor, TODO list, or
   custom session.

Press `?` at any time on the dashboard to see the complete, current keybinding
reference.

## Core concepts

### Projects, features, and sessions

- A **project** points to a directory that you want to work in.
- A **feature** represents one task or branch within that project.
- A **session** is an agent, terminal, editor, TODO list, or custom command
  attached to a feature.

For git projects, the first feature can use the repository directory directly.
Additional features use worktrees under `.worktrees/`, allowing their agents to
work concurrently without changing one another's files. Press `F` to fork a
feature or `B` to create several features at once.

### Permission modes

AIW asks you to choose how much autonomy an agent receives for each feature:

| Mode | Behavior |
| --- | --- |
| **Vibeless** | Shows supported file edits for approval before they are applied. Available for Claude Code. |
| **Vibe** | Allows edits without AIW's per-edit review gate while retaining the agent's normal permission controls. |
| **SuperVibe** | Skips agent permission prompts. AIW shows a warning before enabling it. |

Codex and Pi do not support AIW's Vibeless edit-review hooks. Pi also does not
receive AIW permission-mode flags.

## Essential controls

### Dashboard

| Key | Action |
| --- | --- |
| `j` / `k` or arrow keys | Move through projects, features, and sessions |
| `h` / `l` | Collapse or expand the selected item |
| `Enter` | Put the keyboard in the selected running session's pane; on anything else, expand/collapse |
| `Ctrl+Q` | Move the keyboard between the sidebar and the agent pane (a tmux key; it works from either side) |
| `N` / `n` | Create a project / feature |
| `s` | Add a session to a feature |
| `c` | Start the selected feature, or the selected stopped session (an agent that exited at its shell shows `■` "exited" and is relaunched in the same window) |
| `v` | Tile the feature's agents in the right pane (`aiw grid`; on a project row: one per feature). `Alt+q` there brings the selected agent back |
| `x` | Stop a feature or a single session. A stopped session stays listed (`■`) and stays stopped when its feature starts again |
| `r` | Rename a feature or session |
| `d` | Delete a project, feature, or session |
| `/` | Search and jump |
| `i` | Show agents needing attention: questions first, then finished work |
| `a` / `X` | Allow / deny the permission prompt the selected agent is waiting on (Claude, Codex, Cursor); in the `i` list too, where `Enter` shows the tool and its full input |
| `e` | Queue a follow-up for the selected agent: delivered only when its current turn ends |
| `I` | On a TODOs session row: start an agent on the next TODO in priority order, across the lists currently showing |
| `z` | Show dormant features: idle and unattended |
| `L` | Open the prompt library |
| `T` | Choose a theme |
| `A` | Manage installed agent harnesses |
| `?` | Show all keybindings |
| `Ctrl+Space` | Commands on the selected session (a menu lists them): `d` diff viewer · `m` Markdown files · `l` past and unsent prompts, pasted again · `N` capture a TODO · `z` complete the TODO this session came from · `h` `H` `M` `1-9` bookmarks · `/` command picker · `a` local actions · `c` config wizard · `r` refresh |
| `q` | Quit the dashboard; the agents keep running (`Esc` does nothing on the dashboard) |

Search accepts every letter, including `j` and `k`. Use arrow keys or
`Tab` / `Shift+Tab` to select a result, `Enter` to jump, and `Esc` to cancel.

### The cockpit

`aiw` is a thin launcher. It starts (or finds) a throw-away tmux server of its
own — `cockpit.sock` beside AIW's private tmux socket, `-f /dev/null`, so your
own tmux configuration is untouched — with one window and two panes: the left
pane runs the dashboard (`aiw sidebar`), the right pane shows the selected
agent's **real** tmux window through a grouped view session on AIW's socket,
exactly as `aiw grid` does. Nothing is relayed and nothing is captured and
redrawn: tmux draws the agent, AIW draws the sidebar.

```
── AIW ─────────────────────────────┬── RS › hx-12 › Claude 1 ───────────────────────────────
╭──────────────────────────────────╮│
│ AI Workstation v0.49.0    ? help │││ ⏺ Bash(git push origin hx-12-sso)
╰──────────────────────────────────╯││ Do you want to proceed?
╭ Sessions ────────────────────────╮││ ❯ 1. Yes   2. Yes, and don't ask again   3. No
│ ▾ RS                     ⛔1 ●1  ││
│  ▾ ⛔ hx-12                0:40  ││
│▎   ⛔ Claude 1             0:40  ││
│▎     permission: Bash           ││
│    ●  Codex 1                6m  ││
│  ▾ ●  hx-14                 12m  ││
╰──────────────────────────────────╯││
╭ Claude 1 ────────────────────────╮││
│ Fix the login redirect loop in … ││
│ Claude Code · Opus 4             ││
│ ctx 42% · $0.84 · 12m old        ││
│ ⎇ hx-12-sso                      ││
│ …/RS/hx-12                       ││
│ ⏭ 2 queued · ✓ allowed Bash      ││
╰──────────────────────────────────╯││
╰ Enter agent  i needs  v tiles  ? q╯│❯
```

- The pane follows the selection after 150 ms of rest, so scrolling through
  rows is free. A project or feature row shows the feature's first running
  agent; a stopped session shows a "select a session" placeholder.
- The sidebar is `sidebar_width` columns wide (`config.json`, default 36,
  minimum 28) at every terminal size; the agent gets the rest.
- A dialog (`n`, `?`, `i`, …) zooms the sidebar pane over
  the whole window and un-zooms when it closes. The agent keeps running
  underneath.
- A second `aiw` while one is open attaches a second client to the same
  cockpit; `aiw grid`, `aiw doctor` and the hook subcommands work alongside.
- `v` runs `aiw grid` for the selected feature in the right pane; `Alt+q`
  there closes the tiles and the pane returns to the selected agent.

### Typing is native

The right pane is the agent's own terminal. `Enter` on a running session (or
`Ctrl+Q` from the sidebar) puts the keyboard there; everything you type goes
to the agent exactly as in `tmux attach` — native latency, mouse and
scrollback, the agent's own key bindings. `Ctrl+Q` is the one key the cockpit
keeps for itself: it flips the keyboard between the sidebar and the agent.
Nothing is sent to an agent on its own.

Prompts AIW prepares for an agent — a TODO, a prompt-library template, a feature's startup
prompt, a past prompt from `Ctrl+Space` then `l` — are **pasted into the
agent's pane unsent** and the pane is focused: you read the text where the
agent will, edit it if you like, and press Enter yourself. The pasted text
is also saved as the checkout's `.claude/latest-prompt.txt`; a prompt that
could not be delivered (the agent was not running) is stashed and offered
again under `Ctrl+Space`, then `l`.

### Session info block

The bottom eight rows of the sidebar (when it is at least 24 rows tall)
describe the selected session from the harness's own files, read locally on
the five-second sweep and cached per session: the title (Claude's own session
summary when it wrote one, else the first prompt; Codex's and Gemini's first
user message; else the AIW task prompt or the feature name), harness and
model, context use in the threshold colours, cost (Claude), age, branch,
worktree, `⏭ n queued` follow-ups and the last permission decision answered
from the dashboard.

Every row carries one badge, the same one the header pills, the grid and the
needs-you list use:

| Badge | Meaning |
| --- | --- |
| `⛔` | needs you: a permission prompt, a question, or a stop nothing explained (the reason is on the next line; `permission: Bash — cargo test` can be answered with `a` / `X`) |
| `●` | working (the activity line, when the agent reports one: `working 6m · 42% ctx · 2 subagents · Edit src/auth.rs`) |
| `✓` | finished a turn; bright until you look at it, dim after |
| `○` | idle (running, nothing to report) |
| `■` | stopped; `■ exited` when the agent died at its shell (`c` relaunches it) |

Project rows show their counts (`⛔1 ●7 ✓2`); feature rows show their most
pressing session. A session row's right edge shows its context use (`42%`)
when the harness reports it, in the warning / critical colour past
`context_warning_percent` / `context_critical_percent`; `⏭ 2 queued` on the
reason line counts the follow-ups waiting for it. Below 28 columns the
sidebar shows glyph and name only.

## User workflows

### Pull-request badges

With an authenticated `gh` and a GitHub remote, a feature whose branch has a
pull request shows a `[PR #N · M open]` badge on its dashboard row. AIW never
polls GitHub in the background: badges refresh when you press `R` (or leader
`r`), and otherwise show the last-known state.
Once that PR is merged or closed without merging, the badge switches to
`[PR #N merged]` / `[PR #N closed]` instead of disappearing, so a finished
feature stays visually distinct from one that never had a PR. A branch
without `gh`, without a GitHub remote, or without any PR shows no badge at
all. Reviewing the pull request itself is the agent's job: Claude Code's
`/code-review` and Codex's `/review` run inside the session.

### Reuse prompts and track TODOs

Press `L` to manage reusable prompt templates. A template you pick is pasted
into the selected agent's pane, unsent, so you read it there and press Enter.
Templates may include `{{placeholder}}` fields that AIW asks you to fill
before injection.

Add a `TODOs` session with `s` to open a checklist — one per feature. The
editor shows up to three lists side by side:

| List | What it holds |
| --- | --- |
| **Worktree** | Work belonging to this feature's own checkout. Features on the repo root have no worktree list. |
| **Project** | Work belonging to the project as a whole, whichever checkout you are in. |
| **Global** | Work belonging to no project at all, shared across every repo AIW knows about. |

All three scopes start visible. Press `p` to hide or show the project list and
`g` to hide or show the global list independently; the worktree list is always
visible when the feature has one. Hidden scopes stay represented by compact
labeled placeholders. The choice is shared by every TODO view for the rest of
the current AIW run and resets the next time AIW starts. `Tab` / `Shift+Tab`
move between visible lists; each list keeps its own cursor, scroll, and
scratchpad while hidden. `M` moves the selected TODO to another visible list
and `C` copies it — a move carries whatever was already started for the item,
while a copy lands as fresh, unstarted work.

With a feature or session selected, press `Ctrl+Space`, then `N` to capture a
TODO without leaving the dashboard. It lands in that feature's worktree list
(the project's if the feature sits on the repo root), and the capture box
names the list it is writing to.

Existing TODO lists are unchanged by the upgrade: they stay project-scoped,
keep their host feature and their links, and show up in the project pane. New
worktree lists start empty.

Press `Enter` on a TODO to start work on it. AIW asks how:

| Choice | What happens |
| --- | --- |
| **Start an agent on this TODO** | Pastes the TODO into an agent's pane, unsent — in this feature for a worktree TODO, or in a feature you pick for a project or global one. |
| **Start an agent in a new feature** | Opens the ordinary create-feature wizard, pre-filled with a branch name from the TODO title; once the feature exists, the TODO is linked to it and the TODO is pasted into its agent's pane, unsent. Needs a git repository. |

A TODO started in a new feature stays open on the list, marked as linked, and
`Enter` afterwards jumps to that feature rather than asking again. If the
feature is later deleted the TODO survives, the link is dropped, and `Enter`
offers the choice again.

Press `I` to work the list rather than a particular item: AIW takes the
highest-priority TODO nobody has started, opens an agent on it, and marks the
item in progress (`[~]`) so the next `I` moves on. It considers whichever
lists are currently visible. Hidden project and global scopes are excluded;
at equal priority AIW prefers the narrower visible scope: worktree, then
project, then global. It works on the list itself and on the `TODOs` row on the
dashboard, and the composer is seeded but unsent, exactly as `Enter` leaves it.

A worktree TODO is worked in the feature that owns the checkout. A project or
global TODO belongs to no one checkout, so `Enter` and `I` ask which feature
should work it; the feature you pick supplies the agent and permission
mode exactly as a worktree TODO's own feature would. Press `i` on a TODO to
set or clear that in-progress mark by hand — useful when you abandoned a session
without closing it. When every remaining TODO is already underway, AIW offers
the next one anyway and asks whether to go to the work already started, start a
second agent, skip it, or cancel.

Deleting a feature deletes its worktree list along with the checkout, so if
that list still has unfinished items AIW asks first: move them to the project
list, move them to the global list, delete them with the worktree, or cancel
the deletion. Nothing is killed or removed until you answer.

### See which agents need you

A stopped agent has stopped for a reason, and "waiting for input" does not say
which. AIW reads its harnesses' lifecycle hooks and marks each stopped session
as one of:

| State | Meaning |
| --- | --- |
| **Question** | The agent asked something, or wants permission, and cannot continue without an answer. |
| **Completed** | The agent finished its turn; the work is waiting to be looked at. |
| **Waiting** | The session stopped, but its harness could not say why. |

The state shows on the feature's dashboard row and in the header count. Press
`i` for the full list, questions first and oldest first within each group;
`Enter` puts the keyboard in the session's pane, `x` dismisses the row.

AIW does not capture the agent's message — the question or the summary is
in the agent's pane, right beside the sidebar.

Fidelity depends on the harness. Claude Code and Gemini CLI report all three
states. Codex fires one hook when a turn ends and cannot say whether it
finished or is asking, so its sessions show as **Waiting** either way. Pi has
no hook mechanism, so its sessions carry no state at all. States are not saved: after an AIW restart, sessions
show as ordinary active until their next event.

### Keep the machine from filling up

Agents and the editors they sit alongside are the bulk of what AIW puts on your
machine, and nothing warns you before the last gigabyte goes.

Before starting an agent, AIW checks how many are already running — across every
project — and how much memory is left. If either is past its threshold, one dialog says which, and `y` starts it
anyway. It never refuses. Terminals, editors, and TODO sessions are not counted
and never raise it.

Creating a feature never raises that dialog: a batch create would queue one per
feature. The feature is created and left stopped instead, with a toast saying
why; `c` starts it.

Stopping a feature also closes the editor AIW opened for it, and the language
servers under it — usually the largest thing the feature was holding. AIW only
closes a window it opened itself and can still identify; anything else it
reports as left alone. Under WSL, where `code` hands off to the Windows side,
there is no local window for AIW to own, so it always reports a skip.

Press `z` to list dormant features — idle *and* unattended — with per-row stop,
close-editor, delete, and open.


## Harness hooks: what AIW does with them

AIW integrates with each agent through the hooks the agent CLI itself offers
(Claude Code, Codex, Gemini CLI, Cursor agent). The hooks run small shell
scripts under `~/.config/ai_workstation/hooks/` that call `aiw <subcommand>`,
which talks to the dashboard over its local Unix socket. Nothing here reaches
the network: the status line, the permission answers and the follow-ups all
stay on this machine (see [NO_CLOUD.md](NO_CLOUD.md)). The exact hook
contracts are in [docs/development/harness-hooks-reference.md](docs/development/harness-hooks-reference.md).

| | Claude Code | Codex | Gemini CLI | Cursor agent |
| --- | --- | --- | --- | --- |
| Needs-you / finished badges, activity line | ✓ | ✓ | ✓ | ✓ |
| **Answer permission prompts from the dashboard** (`a` / `X`) | ✓ `PermissionRequest` | ✓ `PermissionRequest` | – (its hooks can only observe: the `⛔` stays, answer in the pane) | ✓ `beforeShellExecution` + `beforeMCPExecution` |
| **Context used and cost on the row** | ✓ status line (model, `42%`, cost, 5h/7d limits) | ✓ rollout file (model, context of the last request, total tokens) | ✓ newest chat file (tokens only, no window) | – |
| **Queued follow-ups** (`e`, `Ctrl+Space Q`, `aiw automation queue`) | ✓ `Stop` | ✓ `Stop` | ✓ `AfterAgent` | ✓ `stop` (`loop_limit` 20) |
| **Subagent count on the working badge** | ✓ | ✓ | – | ✓ |
| **Launch context in the system prompt** (project, feature, branch, worktree, AIW session; "mention the branch in commits and PR titles") | ✓ `--append-system-prompt` | ✓ `developer_instructions` | ✓ `SessionStart` additional context | ✓ `sessionStart` additional context |
| Where AIW puts its hooks | `--settings <file>` at launch (default) or `.claude/settings.local.json` | `.codex/hooks.json` + `notify` at launch | `.gemini/settings.json` | `.cursor/hooks.json` |

How the pieces work:

- **Permission prompts.** When Claude, Codex or Cursor is about to ask, its
  hook sends the request to AIW and *waits* (up to `permission_wait_seconds`,
  default 300) for your answer. The row shows `⛔ permission: Bash — cargo
  test`; `a` allows, `X` denies, on the row, in the `i` list, or in the detail
  overlay (`Enter` in the `i` list: tool, full input, `p` lets the agent ask
  in its own pane, `o` opens it). If you do not answer in time, or AIW is not
  running, the hook prints nothing and the agent shows its own prompt, so
  nothing can get stuck. `y` was taken (mark ready), hence `a` / `X`. Every
  decision is in the debug log (`D`).
- **Context and cost.** Claude runs AIW's `statusLine` command (passed with
  `--settings`), which prints one line in Claude's footer (`Opus 4 · ctx 42% ·
  $0.84`) and sends the numbers to AIW; if you configured your own status line
  in `~/.claude/settings.json`, AIW runs it and shows its output instead.
  Codex rollouts and Gemini chat files are read locally on the 5-second sweep.
  The working badge says `working 6m · 42% ctx`; split tiles and the GUI
  contract carry the same numbers.
- **Queued follow-ups.** `e` on a session row opens "Queue a follow-up":
  type the next instruction, `Enter` queues it (`Shift+Enter` for a newline,
  `Tab` to the list to reorder with `J`/`K` or drop with `d`). The queue is
  stored in `aiw.db`, so it survives a restart, and is delivered one item per
  turn: when the agent's turn ends, its Stop hook asks AIW and gets the next
  text as a continuation. The task prompt is never sent on its own; only what
  you queued is. Claude stops after 8 continuations in a row, Cursor after
  `loop_limit` (AIW sets 20) — the queue draining bounds the loop anyway.
  `aiw automation queue --session <id> "text"` queues from a script.
- **Subagents.** `SubagentStart` / `SubagentStop` keep a count per session
  (`working 6m · 2 subagents`), cleared when the turn ends.
- **Launch context.** Every agent is told its project, feature, branch,
  worktree path and AIW session id, and asked to mention the branch name in
  commit messages and PR titles (`agent_context_prompt: false` turns it off).

One-time setup per harness:

- **Codex** trusts a project hook by the hash of its definition: the first
  Codex session in a worktree asks you to trust AIW's hooks once (`/hooks` in
  the Codex TUI). AIW never passes `--dangerously-bypass-hook-trust`; `aiw
  doctor` reminds you.
- **Gemini CLI** cannot answer a permission prompt from a hook. To stop being
  asked for safe commands, add them to `tools.allowed` in `.gemini/settings.json`
  (for example `["run_shell_command(git)"]`; Gemini needs a restart).
- **Cursor agent** runs project hooks from `.cursor/hooks.json` only in a
  trusted workspace; trust the worktree once when Cursor asks.

Claude settings: with `claude_hooks_via_settings_flag: true` (the default)
AIW builds its hooks and status line into
`~/.local/state/ai_workstation/claude-settings/<session>.json` and passes it
with `--settings`, so nothing of AIW's is written into the repo's
`.claude/settings.local.json` any more (entries an older AIW wrote there are
removed on the next refresh; your own entries stay). Claude merges hook
entries across settings levels rather than replacing them, so your project
hooks keep running alongside AIW's. Set the flag to `false` to go back to
writing `settings.local.json`; `aiw doctor` reports which mode is active.

```json
{
  "permission_wait_seconds": 300,
  "agent_context_prompt": true,
  "claude_hooks_via_settings_flag": true
}
```

## Configuration

AIW creates its global configuration at `~/.config/ai_workstation/config.json`. A project
can add `aiw.json` at its repository root; project settings override
matching global settings.

From an embedded session, press `Ctrl+Space`, then `c` to open the configuration
wizard. It covers the common customizations, including:

- feature presets;
- custom session commands;
- lifecycle hooks;
- workspace-specific agent restrictions; and
- key remapping.

Use `T` on the dashboard to choose an AIW theme. If your terminal does not have
a Nerd Font, enable ASCII fallbacks:

```json
{
  "nerd_font": false
}
```

AIW stores its projects, features, saved prompts, and other application state
in `~/.config/ai_workstation/aiw.db`.

These settings govern the resource guards described above:

```json
{
  "max_concurrent_agents": 4,
  "low_memory_warn_mb": 1536,
  "kill_editor_on_stop": true,
  "dormant_idle_minutes": 60,
  "dormant_last_accessed_hours": 4
}
```

Set `"confirm_quit": true` to have `q` ask first while agents are running:
the first press says how many, a second press within five seconds quits and
`Esc` keeps AIW open. Off by default, so `q` quits at once.

`0` disables either warning. The defaults come from measured idle memory: a
Claude harness settles near 380 MiB and Codex near 220 MiB, while a single
language server runs to roughly 1.8 GiB — which is why editors are reclaimed
when a feature stops but never counted as agents. A feature is dormant only
when both of its thresholds are past.

This setting governs how long a needs-attention state stays on the dashboard:

```json
{
  "waiting_stale_minutes": 30
}
```

A question nobody has answered for this long stops being news, so AIW drops it
back to plain idle and the session leaves the `i` list. `0` keeps states up
until the agent produces output again. Ageing out does not stop or change the
session, and a waiting session still counts toward `max_concurrent_agents` and
still qualifies as dormant.
### Built-in customization skills

AIW injects a small set of skills into Claude Code and Codex feature
workspaces so the agent can safely customize the project-local AIW config for
you. Ask the agent to use `aiw:configure` to explain the current setup,
`aiw:add-session` to add a dev server or other persistent command,
`aiw:add-hook` to automate feature lifecycle events, `aiw:add-preset` to create
a reusable feature setup, or `aiw:add-prompt` to add a shared prompt template.
You can invoke a skill by name or simply describe the customization you want.

## Automation

A running AIW instance accepts structured commands for creating projects and
features without scripting the TUI:

```bash
aiw automation create-project --file request.json
aiw automation create-feature --file request.json
aiw automation create-batch-features --file request.json
```

See the [automation guide](docs/automation/README.md) for request templates,
examples, dry runs, and response formats.

## Troubleshooting

- Run `aiw doctor` for a read-only report on what AIW is putting on the
  machine: agents against your limit, editor windows open alongside them,
  memory and swap, `aiw-*` tmux sessions and worktrees with no matching
  feature, editors still running for features you stopped, and any project
  configured only by an AMF `amf.json` (read until it has its own `aiw.json`), plus a line
  confirming AIW makes no network connections itself. `--json` emits
  the same findings for scripting. It changes nothing and always exits `0`.
- Press `D` on the dashboard to view AIW's debug log. The same log is available
  at `~/.local/state/ai_workstation/debug.log`.
- If icons render incorrectly, set `"nerd_font": false` in the global config.
- If an agent does not appear during feature creation, press `A` and confirm
  that its CLI is installed and enabled.

## License

AIW is released under the [MIT License](LICENSE).
