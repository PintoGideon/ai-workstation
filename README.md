
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
- Supports guided planning, supervised edits, final diff review, and GitHub PR
  review workflows.
- Explains code you didn't write: browse a repository read-only and ask an
  agent about any file, hunk, or line range, with the answers kept per project.
- Provides reusable prompts, scoped TODO lists, themes, lifecycle hooks, and
  workspace presets.
- Supports ordinary directories as well as git repositories.

The desktop GUI also [views PR and agent-validation screenshots](docs/gui-screenshots.md)
with source and producing-session attribution.

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

- An authenticated [GitHub CLI](https://cli.github.com/) (`gh`) enables PR
  triage, AI review, posting final-review feedback, and the dashboard's
  open/merged/closed PR badge.
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
database, port reservations, custom themes, syntax parsers and review memory
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

1. Start AIW from a normal shell or an existing tmux session:

   ```bash
   aiw
   ```

2. On first launch, choose the agent CLIs you want AIW to use. AIW checks that
   each selected CLI is installed. You can reopen this setup later with `A`.

3. Press `N` to add a project, then enter its name and directory. The directory
   may be a git repository or an ordinary folder.

4. Press `n` to create a feature. Choose a branch name, agent, permission mode,
   and whether to use the guided plan interview. The feature starts when setup
   is complete.

5. Select the agent session and press `Enter` to work in its embedded terminal.
   Press `Ctrl+Q` to return to the dashboard.

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

### Plan mode

Plan mode interviews you about a feature before launching its agent. You can
review and edit the resulting plan, ask an agent to improve it, or cancel it.
AIW does not save the plan or launch the feature until you accept it. Press `P`
on an existing feature to run the interview again.

Multiple-choice questions in the interview also take your own answer: press `e`
to type into the "Your own answer" box under the options. You can answer with
your own text alone or add it alongside a picked option; `Enter` in the box
returns to the options without submitting, and `Enter` on the option list
submits.

To let the interview read a ticket the brief links to (Asana, Linear, Jira,
and so on), see [Issue trackers in plan interviews](#issue-trackers-in-plan-interviews).

## Essential controls

### Dashboard

| Key | Action |
| --- | --- |
| `j` / `k` or arrow keys | Move through projects, features, and sessions |
| `h` / `l` | Collapse or expand the selected item |
| `Enter` | Open the selected session or expand/collapse an item |
| `N` / `n` | Create a project / feature |
| `s` | Add a session to a feature |
| `c` | Start the selected feature, or the selected stopped session (an agent that exited at its shell shows `■` "exited" and is relaunched in the same window) |
| `v` | Split view: tile the feature's agents live, 2–4 per screen (on a project row: one per feature) |
| `b` | Toggle the dashboard layout: the workspace (default: a session sidebar beside the selected session, live — see below) ⇄ the project tree |
| `x` | Stop a feature or a single session. A stopped session stays listed (`■`) and stays stopped when its feature starts again |
| `r` | Rename a feature or session |
| `d` | Delete a project, feature, or session |
| `/` | Search and jump |
| `i` | Show agents needing attention: questions first, then finished work |
| `a` / `X` | Allow / deny the permission prompt the selected agent is waiting on (Claude, Codex, Cursor); in the `i` list too, where `Enter` shows the tool and its full input |
| `e` | Queue a follow-up for the selected agent: delivered only when its current turn ends (`Ctrl+Space`, then `Q`, from the pane or a view) |
| `I` | On a TODOs session row: start an agent on the next TODO in priority order, across the lists currently showing |
| `z` | Show dormant features: idle and unattended |
| `G` | Open GitHub PR triage; on a project row, review a PR yourself |
| `W` | Run AIW's AI review of a PR diff |
| `K` | Open Learning Mode: read the code and ask about it |
| `L` | Open the prompt library |
| `E` | Edit headless AI prompt templates (overrides) |
| `T` | Choose a theme |
| `A` | Manage installed agent harnesses |
| `?` | Show all keybindings |
| `q` | Quit (`Esc` does nothing on the dashboard) |

Search accepts every letter, including `j` and `k`. Use arrow keys or
`Tab` / `Shift+Tab` to select a result, `Enter` to jump, and `Esc` to cancel.

### Embedded session

Everything you type goes straight to the agent; there is no compose box
unless you turn it on with `Ctrl+Space`, then `e`. These controls belong to AIW:

| Key | Action |
| --- | --- |
| `Ctrl+Q` | Return to the dashboard |
| `Ctrl+Space` | Open the leader-command menu |
| `Ctrl+Space`, then `Enter` | Attach: hand the terminal to this agent's tmux window (native typing); `Ctrl+Q` detaches |
| `Ctrl+Space`, then `w` | Switch sessions |
| `Ctrl+Space`, then `i` | Jump to an agent needing attention |
| `Ctrl+Space`, then `f` | Start final diff review |
| `Ctrl+Space`, then `p` | Open the prompt library |
| `Ctrl+Space`, then `E` | Edit headless AI prompt templates (overrides) |
| `Ctrl+Space`, then `N` | Add a TODO to this worktree's list |
| `Ctrl+Space`, then `e` | Turn the compose box on or off for this session |
| `Ctrl+Space`, then `Q` | Queue a follow-up for this agent (see "Harness hooks" below) |
| `Ctrl+Space`, then `?` | Show all leader commands |

In split view, `Ctrl+Space` then an arrow key moves focus between tiles, `z`
zooms the focused tile, `[` / `]` page through more agents, `i` jumps to the
next agent needing you and `v` leaves split view.

### Workspace layout

The dashboard opens in the **workspace** layout: a session sidebar on the
left and the selected session live on the right. You move between agents by
moving the cursor; the pane follows the selection (after 150 ms, so scrolling
through rows is free). `b` swaps it for the project tree and back; the choice
is saved (`dashboard_layout` in `config.json`, and an existing `"tree"`
setting is kept). Below 72 columns the tree is shown with a note until the
window is wider.

```
╭ Sessions ──────────────────╮╭ RS › hx-12 › claude ─────────────────────────── ⛔ question 0:40 ╮
│ ▾ RS               ⛔1 ●1  ││                                                                  │
│  ▾ ⛔ hx-12           0:40 ││  ⏺ Bash(git push origin hx-12-sso)                               │
│▎   ⛔ claude          0:40 ││  Do you want to proceed?                                         │
│▎     permission: Bash      ││  ❯ 1. Yes   2. Yes, and don't ask again   3. No                  │
│    ●  codex             6m ││                                                                  │
│    ✓  gemini            3m ││                                                                  │
│  ▾ ●  hx-14            12m ││                                                                  │
│    ●  claude           12m ││                                                                  │
│ ▾ BA                   ⛔1 ││                                                                  │
│  ▾ ⛔ ba-7              2m ││                                                                  │
╰────────────────────────────╯╰──────────────────────────────────────────────────────────────────╯
╰ Tab type here  o full view  i next needing you  v tiles  n feature  c start  x stop  b tree ───╯
```

Every row carries one badge, the same one the tree, the header pills, the
split-view tiles and the needs-you list use:

| Badge | Meaning |
| --- | --- |
| `⛔` | needs you: a permission prompt, a question, or a stop nothing explained (the reason is on the next line; `permission: Bash — cargo test` can be answered with `a` / `X`) |
| `●` | working (the activity line, when the agent reports one: `working 6m · 42% ctx · 2 subagents · Edit src/auth.rs`) |
| `✓` | finished a turn; bright until you look at it, dim after |
| `○` | idle (running, nothing to report) |
| `■` | stopped; `■ exited` when the agent died at its shell (`c` relaunches it) |

Project rows show their counts (`⛔1 ●7 ✓2`); feature rows show their most
pressing session. The pane border turns amber while the shown agent needs
you. A session row's right edge shows its context use (`42%`) when the
harness reports it, in the warning / critical colour past
`context_warning_percent` / `context_critical_percent`; `⏭ 2 queued` on the
reason line counts the follow-ups waiting for it.

| Key | Action |
| --- | --- |
| `b` | Toggle tree ⇄ workspace layout (saved at once) |
| `j` / `k`, arrows | Pick a session; the pane follows after 150 ms |
| `Enter` on a session or feature | **Attach**: your terminal is handed to the agent's tmux window, so typing is native (no relay, no lag). `Ctrl+Q` detaches and returns to the dashboard |
| `Tab`, a click on the pane | Type through the in-app pane instead: keys are relayed to the agent as in the full view |
| `Ctrl+Q` | In the pane: back to the sidebar (not to the tree) |
| `o` | Open the selected session in the full view (sidebar, compose box, scroll mode) |
| `Ctrl+Space` | Leader in the pane, as in the full view: `z` full view, `q` sidebar, `i` needs you, `f` final review, `p` prompts, `t` / `T` next / previous session |
| `v` | Tile the feature's agents (split view); leaving it returns to the workspace |
| everything else | The dashboard keys keep working with the sidebar focused: `n`, `s`, `c`, `x`, `d`, `r`, `i`, `z`, `/`, `B`, `G`, `T`, `?`, … |

Below 100 columns the sidebar shows glyph and name only; below 72 the
dashboard shows the tree (with a note in the footer) until the window is
wider.

### Attach: typing natively

The in-app pane and the full view relay every key through tmux and redraw
from a capture, which costs a few tens of milliseconds per keystroke. When
you are going to type for a while, attach instead: `Enter` on a session in
the workspace layout (or `Ctrl+Space`, then `Enter`, in a view) hands your
terminal to the agent's tmux window. Your terminal emulator then talks to
Claude Code (or Codex, Gemini, Cursor, Pi) directly, with native latency,
mouse and scrollback. A tmux status line shows where you are
(`AIW project › feature › session`) and `Ctrl+Q` brings you back to the
dashboard, which picks up where it left off. The agent keeps running either
way; attaching changes nothing about the session.

## User workflows

### Understand a codebase you didn't write

Select a feature and press `K` to open Learning Mode. It is a read-only reader
for the project's code with an agent attached: browse the files, point at the
part you don't understand, and ask about it. **Nothing in Learning Mode changes
your files.** The only key that can lead to an edit is `S`, which hands the
answer to an ordinary agent session and says so.

If you don't know where to start, you don't have to. On a project you have not
asked anything about yet, the file list opens on a pinned **Start here** group:
a ready-made question asking for a tour of the whole project, with the cursor
already on it, followed by whichever of the README, the contribution guide, the
entry point, and the manifest that project actually has. Press `t` at any time
for starter questions ("Explain this line by line", "What would break if I
deleted this?") that load into the prompt so you can edit them before asking.

Below that group the project's files are a **folder tree**, so the list shows
you the layout rather than a wall of paths. Folders open at the top level with
the way down to the Start here files already unfolded, and each closed folder
says how many files are inside it, so you can skip whole areas you have no
business in yet.

| Key | Moves you |
| --- | --- |
| `l` | Open the folder under the cursor, or step into an open one |
| `h` | Close the folder, or step out to the one above |
| `Z` | Open every folder in the project, or fold them all back |
| `Enter` | The same, on a folder — and opens the file, on a file |

Resting on a folder changes nothing about the question you have lined up;
folders are only for finding files.

Point at what you want to ask about with `f` (the whole file), `v` (start a line
range), `P` (the whole project), or `x` (the change under the cursor, in
branch-changes scope). Then ask one of two ways:

| Key | Asks for |
| --- | --- |
| `e` | *Explain this to me* — a teaching answer; no change is proposed |
| `c` | *Ask for a change* — a concrete proposal you can act on later |

Asking never blocks: keep browsing and asking while earlier answers arrive, and
the header counts what is still generating. Answers are written for a
**newcomer** by default — terms defined on first use, ending with what to read
next — and `L` switches to **familiar** for denser answers. `s` switches between
all files in the project and only the files changed on this branch. `m` picks
which agent answers, and is already set to one that works.

Once an answer is on screen, five keys act on it:

| Key | Action |
| --- | --- |
| `F` | Ask a follow-up; the agent keeps the question and answer you just read |
| `D` | Ask again with the repository readable — slower, but it checks (except on Codex, see below) |
| `i` | Re-file the entry as the other kind; the answer text is left alone |
| `a` | Keep the answer as a to-do on this feature's TODO list |
| `S` | Hand it to a live agent session, with the prompt filled in and unsent |

`D` is worth knowing about: an ordinary answer only sees the code on screen, so
it can name files or line numbers that don't exist. `D` re-asks the same
question with the repository open and keeps both answers so you can read them
against each other.

Codex is the exception: it has no way to answer without reading the repository,
so every Codex answer already read it — the row says *read the repo* from the
start, and `D` tells you there is nothing deeper to ask for and points you at
`F` instead. If you want the on-screen-only answer and the repository-checked
answer side by side, ask with an agent that offers both (`m`).

Questions and answers are kept per project, so reopening `K` brings back what
you asked before, with follow-ups still under the question they continue. A
question remembers the lines it was about, and the code moves on — so on the way
in AIW checks each one against the project as it is now. An entry whose code has
shifted is marked **moved** and the answer tells you where it went; one whose
code is gone is marked **anchor lost**. Either way the question and the answer
are kept exactly as they were: the point of the mark is that a stale line number
should not read as an answer that was always wrong. Press `?` inside Learning
Mode for the full key list; it opens by itself the first time you enter a
project.

### Review changes before shipping

From an embedded session, press `Ctrl+Space`, then `f`. AIW presents the
feature's diff file by file and lets you attach line comments or suggested
changes. When you finish, AIW writes the feedback and hands it to an agent.
With an authenticated `gh` CLI, AIW can also post the feedback to the branch's
pull request.

The optional AI co-reviewer that drafts line comments for the current file
reviews an oversized file hunk group by hunk group rather than sending one
truncated prompt; the status line reports if any group could not be reviewed.

### Work through pull-request feedback

With an authenticated `gh` and a GitHub remote, a feature whose branch has a
pull request shows a `[PR #N · M open]` badge on its dashboard row and in an
embedded session's header. AIW never polls GitHub in the background: badges
refresh when you press `R` (or leader `r`) or open PR Triage, and otherwise
show the last-known state.
Once that PR is merged or closed without merging, the badge switches to
`[PR #N merged]` / `[PR #N closed]` instead of disappearing, so a finished
feature stays visually distinct from one that never had a PR. A branch
without `gh`, without a GitHub remote, or without any PR shows no badge at
all.

Select a feature and press `G` to open PR Triage. You can inspect review
threads, send an individual or batched fix prompt to an agent, reply, and mark
threads done. Press `W` to have AIW run its own review of the PR diff. GitHub
actions are presented for confirmation before AIW writes to the pull request.
When `f` or `B` targets a dedicated session, AIW lets you name it so multiple
PR Triage agents can run simultaneously; leave the name blank to reuse the
default `PR Triage` session.

A comment that asks a question rather than requesting a change can be
investigated instead of fixed: press `v` to run a strictly read-only headless
pass on the selected comment — it inspects the repo but changes nothing — and
the overlay blocks until it returns. Its answer persists per PR and reopens in
the detail panel. Before pressing `v`, press `e` to attach an optional note —
a hypothesis the investigation verifies against the PR and repo (for example,
"I think this is already handled in `foo.rs` — double-check"), not a fact it
assumes. The note shows in a banner so you can confirm or clear it; leaving it
empty runs the investigation exactly as before. `f` and `B` still fix and batch a comment normally whether or
not it has an investigation — and when it does, the investigation's findings are
appended to the fix prompt as a starting point. Press `a` on a finished
investigation to act on it:
post an editable reply, ask a follow-up (re-runs read-only with the prior answer
as context), dismiss it, or keep it as a TODO.
An unchanged AI-drafted reply discloses the harness, best-effort model,
estimated tokens, and estimated cost of the session that wrote it. AIW's own
AI review (`W`) carries the same attribution: once a run finishes, the pane
shows which harness and model produced it and the run's token usage and
estimated cost, and that line is included above the `— AI review via AIW`
marker on the posted summary and every inline comment. A harness that reports
no usage degrades to model-only attribution rather than showing a fake `$0.00`.

When a PR diff is too large to review in one prompt, `W` splits it into
per-file batches, reviews each on its own (splitting an oversized file hunk by
hunk), and combines the findings with a synthesis pass — the running screen
shows the batch progress. Coverage stays complete: any slice that still will
not fit is listed rather than dropped, and the review summary is prefixed with
a "⚠ Partial coverage" note whenever the diff had to be split. The size
threshold is a per-harness default; set `review_prompt_budget_tokens` in
`~/.config/ai_workstation/config.json` (or per repo in `aiw.json`) to change it, or to
`0` to turn pre-send splitting off. For a sprawling refactor, reviewing the
branch commit by commit still gives the sharpest results — per-slice review
cannot see across files.

To seed review memory from earlier reviews, open the PR picker and press `b`.
If `G` opened the feature's pull request directly, press `g` from PR Triage to
return to the picker first. Choose a lookback of 20, 50, 100, or all recent
closed and merged pull requests with `j`/`k`, then press `Enter`. The default is
50. Press `g` in the lookback dialog to switch between the project's
`.aiw/review-memory.md` and the cross-project memory at
`~/.config/ai_workstation/review-memory.md`; project memory is the default. This requires
authenticated `gh` and an available Claude CLI. Fetching the review history
does not use agent tokens; AIW then makes one agent pass to distill recurring
findings and appends only new ones. You can press `Esc` while it runs to return
to the picker without cancelling the background job.

### Review a teammate's pull request

To review a pull request yourself, rather than triage the comments on your own,
press `G` on a project row, or press `G` on a feature and then `Tab`. The
**Review** tab lists every open pull request in the repository, including your
own and drafts, with its number, title, author, and branch (`owner:branch` for
a fork). The list loads in the background; if `gh` isn't installed or signed
in, the tab says so, and `r` retries.

Press `Enter` to open a pull request in the same diff viewer as the final
review. It shows exactly the pull request's changes, from where it branched to
its latest commit. No agent is started and no feature is needed. AIW fetches
the pull request into private `refs/aiw/review/` refs and reads every file from
git objects: your checked-out branch, staged and unstaged changes, untracked
files, and stash are never touched, and the refs are removed when you leave.

Review as you would a feature: approve or reject files, and leave line
comments, ranges, and suggested changes. `Esc` pauses and saves your draft;
`Enter` on the same pull request resumes it, and the list marks it with
`● N comments`. If the pull request has new commits by then, AIW flags only
the files whose changes differ, clears your verdicts on those files, and keeps
every comment. A comment whose code is gone is listed as outdated in the notes
panel, and `↻ updated` marks the row.

Press `q` to submit. Choose **Comment**, **Approve**, or **Request changes**
(GitHub doesn't allow the last two on your own pull request), and write a
summary with `e`. Inline comments are pinned to the commit you reviewed. If the
pull request has moved on since you opened it, AIW won't post; press `o` to
reopen it at its new commit with your draft intact. Comments that can't sit on
the diff, such as outdated ones, are included in the summary instead of being
dropped. The walkthrough, AI co-review, and changeset-overview passes, and the
keys that need a local checkout (`E`, `b`, `t`, `X`), aren't available in this
mode.

### Ask questions during review

Press `Q` in Final Review, manual PR review (`G`), or AI PR review (`W`) to ask
about the reviewed code. `Ctrl+S` sends a question (`Ctrl+Enter` also works in
terminals that report it); `Ctrl+H` changes the harness. As with AIW's other
AI calls, a notice first shows which prompt will run and lets you view or edit it
(`review.question`, and `review.question_draft` for comment drafts). While a
request runs, the overlay shows a spinner, the harness, and the elapsed time. The AI can inspect repository code beyond the selected file or lines,
including unchanged helpers. PR questions require a clean local checkout at the
reviewed PR head. If that checkout is unavailable, AIW explains the mismatch and
keeps the review and question intact.

After an answer, `e` starts a follow-up, `r` retries, `[`/`]` browse the
conversation, and `j`/`k` scroll the answer. `Esc` returns to the same review
position; a running question continues while the overlay is closed. `Ctrl+X`
cancels its request. Conversation history clears when the review closes.

Use `i` to draft an inline comment at the question's selected diff line, or `g`
to draft a general review comment. Edit the generated text, then `Ctrl+S`
opens the existing comment editor. `Esc` discards the generated draft. Existing
comments are preserved, changed code invalidates old draft anchors, and posting
still requires the review's explicit submit action.

### Reuse prompts and track TODOs

Press `L` to manage reusable prompt templates, or open them from a session with
`Ctrl+Space`, then `p`. Templates may include `{{placeholder}}` fields that AIW
asks you to fill before injection.

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

From any session, press `Ctrl+Space`, then `N` to capture a TODO without
leaving your current work. It lands in that feature's worktree list (the
project's if the feature sits on the repo root), and the capture box names the
list it is writing to.

Existing TODO lists are unchanged by the upgrade: they stay project-scoped,
keep their host feature and their links, and show up in the project pane. New
worktree lists start empty.

Press `Enter` on a TODO to start work on it. AIW asks how:

| Choice | What happens |
| --- | --- |
| **Start an agent on this TODO** | Opens a session with the TODO in the composer, unsent — in this feature for a worktree TODO, or in a feature you pick for a project or global one. |
| **Start an agent in a new feature** | Opens the ordinary create-feature wizard, pre-filled with a branch name from the TODO title and plan mode off; once the feature exists, the TODO is linked to it and its agent is seeded with the TODO, unsent. Needs a git repository. |
| **Plan this TODO first** | Runs the guided plan interview, with the TODO's title, notes, and the list scratchpad already filled in as the feature brief — editable before the first question. |

If you choose to plan it, AIW then asks where the plan should land: **here**, in
the feature the list lives in, or in a **new feature and worktree** created for
it. Picking a new feature opens the ordinary create-feature wizard, pre-filled
from the TODO, so the branch, agent, and permission mode are still yours to
change; the worktree is created before the interview starts, so the plan is
written into a real checkout.

Either way the accepted plan is saved and an agent is started on it. A plan that
lands in an existing feature is written to `AIW_PLAN.todo-<name>.md` rather than
`AIW_PLAN.md`, so it sits beside that feature's own plan instead of replacing
it, and the agent is told which file is its own.

A TODO planned into a new feature stays open on the list, marked as linked, and
`Enter` afterwards jumps to that feature rather than asking again. If the
feature is later deleted the TODO survives, the link is dropped, and `Enter`
offers the choice again. An interrupted interview is saved as a draft and
offered back the next time you press `Enter` on that TODO.

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

### Override the AI prompts AIW sends

Behind the plan interview, Learning Mode, the final-review diff helpers, the AI
PR review, and the review-memory bootstrap/compaction, AIW makes one-shot
("headless") AI calls with prompts it builds for you. Press `E` on the
dashboard — or `Ctrl+Space`, then `E` from a session — to open the
**prompt-override manager**. It lists every template with its effective source
(`built-in`, `feature`, `project`, or `global`) and `[F][P][G]` flags for which
scopes already carry an override.

`Enter` or `e` opens an editor on the effective template. `Ctrl+S` moves to a
scope picker, then a harness picker, then saves:

| Scope | Where it lives | Applies to |
| --- | --- | --- |
| **This feature** | `aiw.db` | just this checkout |
| **This project** | `aiw.json` `prompt_overrides` key | the repo — committed, shared with everyone |
| **Global** | `aiw.db` | every project on this machine |

The nearest scope wins — feature → project → global → built-in — and within the
winning scope a per-harness template beats the shared one. `d`, `d` clears the
effective override. Templates carry visible `{{token}}` placeholders that AIW
re-fills with live context (the diff, the question, the interview answers) each
time the prompt runs. **There is no validation**: if you delete a required
token or add one AIW does not supply, it is saved and rendered exactly as
written.

Project-scope overrides sit in `aiw.json` under `prompt_overrides`, keyed by
the stable prompt id (shown in the manager):

```json
{
  "prompt_overrides": {
    "pr_review.ai_review": {
      "template": "You are reviewing a diff... {{annotated_diff}} ..."
    },
    "learning.answer": {
      "template": "shared text with {{question}}",
      "harnesses": { "codex": "codex-specific text with {{question}}" }
    }
  }
}
```

`.aiw/` is generated and gitignored, so these overrides live in `aiw.json`
(the tracked repo config) rather than a file under `.aiw/`.

Before each user-initiated headless call, a **pre-call notice** names the
prompt and target harness: `v` shows the exact rendered prompt, `e` jumps to
the manager for that prompt (continue afterwards and the override applies to
this run), `Enter` makes the call, `Esc` cancels it. There is no "don't ask
again". Calls that run without you watching — the Learning Mode answer queue
and session summaries — announce with a toast instead of the modal.

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
`Enter` opens the session, `x` dismisses the row.

AIW does not capture the agent's message — open the session to read the
question or the summary.

Fidelity depends on the harness. Claude Code and Gemini CLI report all three
states. Codex fires one hook when a turn ends and cannot say whether it
finished or is asking, so its sessions show as **Waiting** either way. Pi has
no hook mechanism, so its sessions carry no state at all. States are not saved: after an AIW restart, sessions
show as ordinary active until their next event.

### Keep the machine from filling up

Agents and the editors they sit alongside are the bulk of what AIW puts on your
machine, and nothing warns you before the last gigabyte goes.

Before starting an agent, AIW checks how many are already running — across every
project, plus any headless review or plan run in flight — and how much memory is
left. If either is past its threshold, one dialog says which, and `y` starts it
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
- workspace-specific agent restrictions;
- plan interview questions; and
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

### Issue trackers in plan interviews

Plan interviews can read tickets from an issue tracker such as Asana, Linear,
or Jira through the same MCP tools Claude Code uses. When the brief links a
ticket, the interview fetches it and treats its contents as part of the
brief, so you don't have to paste it in. This works only when the interview
runs on Claude. Other harnesses ignore the setting and log that they did.

This is a **global-only** setting in `~/.config/ai_workstation/config.json`. A
repository's `aiw.json` can't turn it on, because MCP servers run programs.

**claude.ai connectors** (tools named `mcp__claude_ai_<Name>__…`) and
servers you added with `claude mcp add` load automatically. You only list
the tools the interview may call:

```json
{
  "plan_interview_mcp": {
    "allowed_tools": [
      "mcp__claude_ai_Asana__get_task",
      "mcp__claude_ai_Asana2__get_task"
    ]
  }
}
```

`allowed_tools` takes **exact** tool names in the form
`mcp__<server>__<tool>`. Wildcards such as `mcp__claude_ai_Asana__*` are
rejected, because most tracker tools can also create and edit tickets. List
only tools that read. The interview sees every connected tool, but anything
not on this list is denied.

The tool names above are placeholders. To print the real ones, run this
command, which makes one small model call:

```sh
echo hi | MCP_CONNECTION_NONBLOCKING=false claude -p --output-format stream-json \
  --verbose --setting-sources "" --tools Read --model haiku \
  | grep -o '"mcp__claude_ai_Asana[^"]*"' | sort -u
```

If you have more than one connector for the same tracker (for example two
Asana workspaces, `Asana` and `Asana2`), allow the read tools from each one
so tickets from either workspace can be read.

**A server that isn't in Claude Code yet** can be supplied as a file with
`config`, which takes an absolute or `~/` path to a file in the
`{"mcpServers": {...}}` format of `.mcp.json`:

```json
{
  "plan_interview_mcp": {
    "config": "~/.config/ai_workstation/plan-mcp.json",
    "allowed_tools": ["mcp__linear__get_issue"]
  }
}
```

```json
{
  "mcpServers": {
    "linear": {
      "type": "http",
      "url": "<the server's MCP URL>"
    }
  }
}
```

The server key (`linear` here) is the middle part of each tool name.

With this configured, the adaptive rounds, the synthesis pass, the Expert
plan review, and its follow-up run with read-only repository tools plus the
listed MCP tools. Directed revisions and isolated investigations don't use
them. The interview still can't edit files or run shell commands, and the
repository's own Claude settings, hooks, `.mcp.json` servers, `CLAUDE.md`,
skills, and subagents are not loaded (the interview can still read those
files, like any other file in the repository). AIW passes `--setting-sources ""` for that, plus
`MCP_CONNECTION_NONBLOCKING=false` so a run waits for claude.ai connectors
instead of sometimes starting without them. If a tool name or the config
file is invalid, the interview runs without MCP and shows why.

To test a setup outside AIW, run the same command the interview uses from
the feature's directory. Like AIW, it sends the prompt on stdin: a prompt
argument after `--allowedTools` can be read as another tool name. Add
`--mcp-config <path>` before `--allowedTools` if you set `config`, and
separate several tools with commas.

```sh
echo "Fetch Asana task 1201234567890 and summarize it" |
  MCP_CONNECTION_NONBLOCKING=false claude -p --output-format text \
    --setting-sources "" --tools Read,Glob,Grep \
    --permission-mode dontAsk --no-session-persistence \
    --allowedTools mcp__claude_ai_Asana__get_task
```

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
