# AI Workstation (AIW)

Based on [agent-mainframe](https://github.com/eldridgerdev/agent-mainframe) by Ryan Eldridge (MIT). See
[LICENSE](LICENSE) for the original notice, [FORK.md](FORK.md) for how this fork is maintained and
[DESIGN.md](DESIGN.md) for what it adds.

Run many AI coding agents in parallel, each on its own branch and in its own
terminal, without losing track of them.

AIW is a terminal dashboard for managing
[Claude Code](https://docs.anthropic.com/en/docs/claude-code),
[Codex](https://github.com/openai/codex), Gemini CLI, Cursor agent and Pi
sessions. It organizes work by project and feature, creates isolated git
worktrees when needed, and shows which agents are waiting for you. It is a
workstation, not a second agent: planning, code review and summaries are
left to the harnesses, which do them better in their own sessions.

**The full guide is [docs/PRODUCT.md](docs/PRODUCT.md)**: every concept, every
key, every setting, each harness's capabilities, the command line, what lands
on disk, and what was removed in favour of the harnesses' own features.

<img width="1896" height="1030" alt="AIW dashboard showing several agent sessions" src="https://github.com/user-attachments/assets/d8160bc6-49ea-4b2b-839a-7ec056897ffc" />

## What AIW does

- Runs several coding-agent sessions side by side from one dashboard, each in
  its own tmux window on AIW's private tmux server.
- Runs Claude in the background under Claude Code's own supervisor
  (`claude --bg`) when you want it to outlive the terminal: the row shows its
  state, `Enter` attaches, detaching sends it back.
- Starts one prompt on several agents at once, in one shared worktree or one
  worktree each, grouped in the sidebar and tiled with `v`. The prompt is
  delivered when each agent is ready, unsent unless you ask.
- Keeps concurrent features isolated with git branches and worktrees created
  outside the repository. The worktrees Claude makes itself (`--worktree`,
  subagents, workflows) are created the same way, with your `.env` and a port
  block, and listed under their feature.
- Shows the selected agent's real terminal beside the sidebar. Typing is
  native; `Ctrl+Q` flips the keyboard between the two.
- Marks which agents need you, answers their permission prompts from the
  dashboard, holds Vibeless file edits for approval, and queues follow-ups
  that are delivered when a turn ends.
- Watches for agents that exited, look stuck (no hooks and no output for ten
  minutes), sit idle with something queued, or sit at the prompt with a full
  context, and offers the fix on `!`: restart resuming the conversation,
  interrupt, `/compact`, replay the prompt, or deliver the queued item now.
- Runs prompts and commands on a schedule (`every 30m`, `daily 09:00`, a
  cron rule, or once): a prompt goes to an idle agent at once, waits in the
  queue while it works, and is held while it waits on you; a command runs in
  a terminal only when it is at its prompt.
- Lands a finished feature with `P`: a preflight, then a rebase onto its
  base, the project's check command, and a fast-forward of the base or a
  pushed branch and a `gh` PR. A conflict, a failing check or a base that
  moved stops it with nothing merged; the feature's worktree and branch are
  cleaned up after a local merge.
- Shows context use, cost and usage windows per session, read from the
  harnesses' own files and status line, and what each session just did:
  compacted, switched model, failed a turn, open tasks, background work.
- Adds terminals, Neovim, VS Code, scoped TODO lists and custom commands to a
  feature; keeps a prompt library and past prompts; starts features from
  GitHub issues and shows pull-request badges.
- Warns before the machine fills up, lists dormant features, and closes the
  editors it opened when a feature stops.
- Makes no network calls of its own. See [NO_CLOUD.md](NO_CLOUD.md).

## Requirements

Install at least one agent CLI and sign in to it before using AIW:

- [Claude Code](https://docs.anthropic.com/en/docs/claude-code)
- [Codex](https://github.com/openai/codex)
- [Gemini CLI](https://github.com/google-gemini/gemini-cli)
- [Cursor agent](https://cursor.com/cli)
- Pi (`pi` on `PATH`)

AIW is built from source and needs `tmux` on `PATH`. Git is needed for
branches and worktrees; plain directories work without it.

Optional: an authenticated [GitHub CLI](https://cli.github.com/) for the issue
browser and pull-request badges, a [Nerd Font](https://www.nerdfonts.com/) for
icons, the `code` command for VS Code sessions, and a C compiler plus git for
installing optional tree-sitter syntax parsers.

## Installation

Building needs Rust via [rustup](https://rustup.rs) with the 1.99.0 toolchain,
a C compiler, git and `tmux`.

```bash
# macOS
xcode-select --install && brew install tmux
# Ubuntu / Debian
sudo apt install build-essential tmux git

rustup toolchain install 1.99.0
```

```bash
cd ~/ai_workstation                                # your clone of this fork
git switch ai-workstation                          # the branch with the AIW work
cargo +1.99.0 install --path . --locked --force    # installs `aiw` into ~/.cargo/bin
aiw -V                                             # AI Workstation (aiw) 0.49.0 (<commit>)
```

Make sure `~/.cargo/bin` is on your `PATH`. There is no self-upgrade: pull
the fork and run the install command again. Running agents are not affected.

For a contributor setup see [docs/development/checks.md](docs/development/checks.md)
and [docs/development/architecture.md](docs/development/architecture.md).
For a container, see the [Docker guide](docs/docker-no-tmux.md).

## Quick start

1. Open a real terminal window and run `aiw`. It builds a small tmux cockpit
   for itself: the sidebar on the left, the selected agent's real terminal
   on the right. Closing the window stops nothing; the next `aiw` attaches
   again. `q` in the sidebar shuts the dashboard down and leaves the agents
   running.
2. On first launch AIW probes each agent CLI on your `PATH` and ticks the
   ones it finds. Press `c` to confirm. `A` reopens the screen later; there,
   `Enter` ticks a row and `c` saves.
3. Press `N` to add a project: a name and a directory.
4. Press `n` to create a feature: a name, worktree or not, harness, mode, the
   Chrome flag for Claude and an optional task prompt. `Space` on the harness
   list ticks more agents for the same prompt. The feature starts when the
   dialog closes.
5. Move onto the agent's row; the right pane shows it. `Enter` or `Ctrl+Q`
   puts the keyboard there. `Ctrl+Q` brings it back.
6. Press `s` on the feature to add another agent, a terminal, an editor, a
   TODO list or a custom session.

`?` shows the complete keybinding reference for the running version.

## Permission modes

| Mode | Claude Code | Codex | Gemini CLI | Cursor agent |
| --- | --- | --- | --- | --- |
| **Vibeless**: every file edit is held for your approval | `--permission-mode default` plus AIW's edit-approval hook | refused | `--approval-mode default` | default |
| **Vibe**: edits accepted, other prompts still ask | `--permission-mode acceptEdits` | `--sandbox workspace-write --ask-for-approval on-request` | `--approval-mode auto_edit` | default |
| **SuperVibe**: no prompts at all | `--dangerously-skip-permissions` | `--sandbox danger-full-access --ask-for-approval never` | `--approval-mode yolo` | `--force` |

Pi receives no mode flags.

## Keybindings

| Key | Action |
| --- | --- |
| `j` / `k`, arrows | Move through projects, features and sessions |
| `h` / `l` | Collapse or expand |
| `Enter` | Keyboard into the selected running session; expand or collapse otherwise |
| `Ctrl+Q` | Flip the keyboard between the sidebar and the agent pane |
| `N` / `n` | Create a project / feature |
| `B` / `F` | Create several features at once / fork a feature onto a new branch |
| `g` | On a project row: browse GitHub issues and start a feature from one |
| `s` / `S` | Add a session (including "Claude (background)", `claude --bg`) / resume a saved Claude or Codex transcript |
| `f` | Fork the selected Claude or Codex conversation: a new window in the same feature, or a new worktree |
| `E` | Claude sessions started outside AIW in your projects, with the command to attach |
| `G` / `C` | Feature groups: move the feature into one, create, rename, recolour, delete / fold or unfold its group |
| `#` | Note on the selected session or feature, shown in the info block and found by `/` |
| `c` / `x` | Start a feature or stopped session / stop a session or feature |
| `v` | Tile the feature's agents (a fan-out group: every sibling) in the right pane; `Alt+q` there comes back |
| `i` | List everything that needs you: questions first, then finished work |
| `a` / `X` | Allow / deny the permission the selected agent is waiting on |
| `e` | Queue a follow-up, delivered when the agent's turn ends |
| `W` | Worktrees Claude made itself (`--worktree`, subagents), created in AIW's layout: terminal, copy path, remove |
| `@` | Message an agent session: straight into a Claude session's inbox, otherwise at its next turn end |
| `!` | Recover an agent the watchdog flagged (exited, stuck, idle with something queued, context critical): restart resuming, interrupt, `/compact`, replay, deliver now |
| `t` | Schedules: recurring prompts for an agent and commands for a terminal (`every 30m`, `daily 09:00`, cron, once) |
| `P` | Land the feature: rebase onto its base, run `land.check_command`, then fast-forward the base or open a PR |
| `Enter` / `c` / `x` / `d` | On a background Claude row: attach / restart / stop / `claude rm` |
| `V` | Reopen a pending Vibeless edit approval |
| `I` | On a TODOs row: start an agent on the next TODO |
| `z` | Dormant features: idle and unattended |
| `R` | Refresh statuses and pull-request badges (runs `gh`) |
| `r` / `d` / `y` / `u` | Rename / delete / mark ready / preferred harness |
| `L` / `T` / `A` / `p` / `w` / `D` / `O` | Prompt library / theme / harnesses / syntax parser / context settings / debug log / open AIW's config dir as a project |
| `/` / `?` / `q` | Search / help / quit (agents keep running) |
| `Ctrl+Space`, then a key | Commands on the selected session: `d` diff viewer · `m` Markdown · `l` past and unsent prompts · `N` capture a TODO · `z` complete the linked TODO · `/` command picker · `a` local actions · `c` config wizard · `h` `H` `M` `1-9` bookmarks · `r` refresh |

Every key is described in [docs/PRODUCT.md](docs/PRODUCT.md#4-every-dashboard-key).

## Harnesses

| | Claude Code | Codex | Gemini CLI | Cursor agent | Pi |
| --- | --- | --- | --- | --- | --- |
| Needs-you / finished badges | question, completed, waiting | waiting only | all three | working, completed | none |
| Answer permissions from the dashboard | ✓ | ✓ | – | ✓ | – |
| Vibeless edit approval | ✓ | refused | – | – | – |
| Context and cost on the row | ✓ | ✓ | tokens only | – | – |
| Queued follow-ups | ✓ | ✓ | ✓ | ✓ | – |
| Resume a saved session | ✓ | ✓ | ✓ | ✓ | – |

AIW reaches each harness through the hooks the CLI itself offers. Each hook
runs a small script that calls an `aiw` subcommand, which talks to the
dashboard over a local Unix socket. AIW never edits a harness's global
configuration; Claude's hooks travel in a per-session `--settings` file and
Codex gets its notify hook per launch. The exact contracts are in
[docs/development/harness-hooks-reference.md](docs/development/harness-hooks-reference.md).

## Configuration

Global settings live in `~/.config/ai_workstation/config.json`. A repository
can add `aiw.json` at its root for custom sessions, feature presets,
lifecycle hooks, keybindings, allowed agents and prompt templates; project
settings override global ones. `Ctrl+Space` then `c` opens the wizard. `T`
picks one of thirty-three themes. Set `"nerd_font": false` for ASCII icons.

Resource guards, dormancy and context thresholds are plain keys:

```json
{
  "max_concurrent_agents": 4,
  "low_memory_warn_mb": 1536,
  "dormant_idle_minutes": 60,
  "dormant_last_accessed_hours": 4,
  "waiting_stale_minutes": 30,
  "context_warning_percent": 70,
  "context_critical_percent": 85,
  "permission_wait_seconds": 300,
  "confirm_quit": false
}
```

Every key, its default and the `aiw.json` schema are in
[docs/PRODUCT.md](docs/PRODUCT.md#9-configuration). AIW also injects the
`aiw:configure`, `aiw:add-session`, `aiw:add-hook`, `aiw:add-preset` and
`aiw:add-prompt` skills into Claude Code and Codex workspaces so an agent can
edit the project config for you.

## Command line

| Command | What it does |
| --- | --- |
| `aiw` | Open the cockpit, or attach to the running one |
| `aiw grid <feature>` | The feature's agents as live tiled panes in this terminal; `Alt+q` closes |
| `aiw doctor [--json]` | Read-only report: agents against the limit, memory, orphaned sessions and worktrees, hook mode, no-network check |
| `aiw fonts` | A font and icon config snippet for your terminal |
| `aiw automation …` | `create-project`, `create-feature`, `create-batch-features` and `queue` from JSON; see [docs/automation](docs/automation/README.md) |

## No cloud

AIW makes no network connections. No HTTP client or server library is linked
into the binary; `scripts/check-no-network.sh` verifies the dependency tree
and the sources. Agents talk to their own providers. `git` runs for branches
and worktrees, `gh` only when you press a key that needs GitHub, and a syntax
parser install clones a grammar only after telling you. Details in
[NO_CLOUD.md](NO_CLOUD.md).

## Troubleshooting

- `aiw doctor` reports what AIW has on the machine and changes nothing.
- `D` on the dashboard opens the debug log, also at
  `~/.local/state/ai_workstation/debug.log`.
- Icons wrong? Set `"nerd_font": false`, or run `aiw fonts` for a terminal
  font snippet.
- A harness missing from the new-feature dialog? Press `A`, tick it, press `c`.
- Coming from agent-mainframe (`amf`)? The first launch copies its settings
  and database into AIW's own folders and leaves AMF untouched; old features
  show as stopped until started here. See [FORK.md](FORK.md).

## License

AIW is released under the [MIT License](LICENSE).
