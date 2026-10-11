+++
title = "Keybindings"
description = "The dashboard keys, the Ctrl+Space menu, and the grid."
weight = 35
+++

Every key on this page has a handler in the current source. Press `?` on the
dashboard for the same list inside the running version of AIW. Keys that
belong to the TODO view and to the other overlays are documented on their own
pages.

<figure>
  <img src="/images/docs/keybindings-help.png" alt="The full keybindings help overlay, listing dashboard actions and their keys">
  <figcaption><code>?</code> opens the complete keybinding reference for the running version of AIW.</figcaption>
</figure>

## The cockpit

`aiw` draws a small tmux layout: the sidebar on the left, the selected
agent's real tmux window on the right. Typing in the right pane is native.
`Ctrl+Q` flips the keyboard between the two panes and works from either side.
Dialogs zoom the sidebar over the whole window and un-zoom when they close.

## Moving around

| Key | Action |
| --- | --- |
| `j` / `k`, arrows | Move through projects, features and sessions |
| `Ctrl+j` / `Ctrl+k` | Jump to the next or previous feature |
| `h` / `l`, left / right | Collapse or expand the selected project or feature |
| `Enter` | Put the keyboard in the selected running session's pane. On a project or feature row, expand or collapse. On a vanished session, open the resume dialog |
| `Ctrl+Q` | Flip the keyboard between the sidebar and the agent pane |
| `/` | Search and jump. Every letter is a search character; arrows or `Tab` pick a result, `Enter` jumps, `Esc` cancels |
| `?` | Show the keybinding reference |
| `q` | Quit the dashboard; the agents keep running. With `confirm_quit` on, a second press within five seconds confirms |
| `Esc` | Take back a pending quit confirmation |

## Projects and features

| Key | Action |
| --- | --- |
| `N` | Create a project |
| `n` | Create a feature in the selected project |
| `B` | Create several features at once: a project path, a count, a name prefix, one harness and mode for all |
| `F` | Fork the selected feature onto a new branch, with its own harness and mode |
| `g` | On a project row, browse its open GitHub issues and start a feature from one |
| `u` | Set the project's preferred harness |
| `y` | Toggle the feature's ready mark |
| `r` | Rename the selected feature or session |
| `d` | Delete the selected project, feature or session |
| `O` | Open AIW's own config directory as a project, so an agent can edit your settings |

## Sessions

| Key | Action |
| --- | --- |
| `s` | Add a session: an agent, terminal, Neovim, VS Code, TODOs or a custom session |
| `S` | Pick a saved Claude or Codex transcript to resume |
| `c` | Start the selected feature or stopped session; relaunch an exited agent in its window |
| `x` | Stop the selected session or feature. The row stays |
| `v` | Tile the feature's agents in the right pane; `Alt+q` there comes back |
| `i` | List everything that needs you: questions first, then finished work |
| `a` / `X` | Allow or deny the permission the selected agent is waiting on |
| `e` | Queue a follow-up for the selected agent |
| `V` | Reopen a pending Vibeless edit approval |
| `I` | On a TODOs row, start an agent on the next unstarted TODO |
| `z` | List dormant features |
| `R` | Refresh statuses and pull-request badges. This is the only key that runs `gh` on its own |

## Tools and settings

| Key | Action |
| --- | --- |
| `L` | Open the prompt library |
| `A` | Manage harnesses: probe the CLIs, tick the ones to offer, `c` saves |
| `T` | Pick a theme |
| `p` | Pick a syntax parser for the diff viewer |
| `w` | Context window settings |
| `D` | View the debug log |

## `Ctrl+Space`, then a key

Commands on the selected session.

| Key | Action |
| --- | --- |
| `d` | Diff viewer: all current changes, or one commit from the feature's history |
| `m` | Pick and read a Markdown file from the worktree |
| `l` | Past and unsent prompts. Pick one and it is pasted again, unsent. `y` copies it, `s` saves it as a template |
| `N` | Quick-capture a one-line TODO for this worktree |
| `z` | Mark the TODO this session was started from as complete |
| `/` | Command picker: the slash commands under `.claude/commands` plus AIW's local actions |
| `a` | The local actions alone: debug log, refresh, pending edit approval |
| `c` | Configuration wizard |
| `h` / `H` / `M` / `1`-`9` | Bookmarks: open the picker, bookmark the current session, remove it, jump to a slot |
| `r` | Refresh statuses and pull-request badges |

## The grid

`v` on a feature replaces the right pane with all of that feature's agents
tiled as live tmux panes. On a project row it tiles the first agent of each
feature. Click a pane to focus it. `Alt+q` closes the grid and the pane
returns to the selected agent.

### `aiw grid` — the same agents in any terminal

`aiw grid <feature>` (or `aiw grid`) shows a feature's agents as real tmux
panes in the current terminal — handy on a second monitor.
`aiw grid <project>` shows the first agent of each feature; use
`project/feature` when a feature name exists in several projects, `--all` to
include terminals, `--list` to print what it would show. Click a pane to focus
it; `Alt+q` or closing the terminal ends the grid. Each pane is a grouped view
session on AI Workstation's tmux socket, so the grid never moves your sessions'
windows and leaves nothing behind when it closes.

## Where the rest live

- [Prompts and TODOs](@/docs/prompts-and-todos.md) — the TODO list's own
  scope-toggle, move/copy, and work-the-queue keys.
- [More Session Tools](@/docs/session-tools.md) — bookmark slots and resuming
  a harness's saved transcripts.
