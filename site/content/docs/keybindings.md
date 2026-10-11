+++
title = "Keybindings"
description = "The essential dashboard and embedded-session controls."
weight = 35
+++

This covers the controls you need to get around AIW day to day. Feature-specific
keys — Learning Mode, PR Triage, TODOs — are documented on their own pages.
Press `?` at any time on the dashboard, or `Ctrl+Space` then `?` in a session,
for the complete, current keybinding reference built into that version of AIW.

<figure>
  <img src="/images/docs/keybindings-help.png" alt="The full keybindings help overlay, listing dashboard actions and their keys">
  <figcaption><code>?</code> opens the complete keybinding reference for the running version of AIW.</figcaption>
</figure>

## Dashboard

| Key | Action |
| --- | --- |
| `j` / `k` or arrow keys | Move through projects, features, and sessions |
| `h` / `l` | Collapse or expand the selected item |
| `Enter` | Open the selected session or expand/collapse an item |
| `N` / `n` | Create a project / feature |
| `s` | Add a session to a feature |
| `c` | Start the selected feature |
| `v` | Split view: tile the selected feature's running agents (on a project row: the first agent of each feature) |
| `x` | Stop a feature, or remove the selected session |
| `r` | Rename a feature or session |
| `d` | Delete a project, feature, or session |
| `/` | Search and jump |
| `i` | Show agents needing attention: questions first, then finished work |
| `I` | On a TODOs session row: start an agent on the next TODO in priority order, across the lists currently showing |
| `z` | Show dormant features: idle and unattended |
| `Z` | Generate a short AI summary of the selected feature's recent activity |
| `G` | Open GitHub PR triage |
| `W` | Run AIW's AI review of a PR diff |
| `K` | Open Learning Mode: read the code and ask about it |
| `L` | Open the prompt library |
| `E` | Edit headless AI prompt templates (overrides) |
| `T` | Choose a theme |
| `A` | Manage installed agent harnesses |
| `?` | Show all keybindings |
| `q` / `Esc` | Quit |

Search (`/`) accepts every letter, including `j` and `k`. Use arrow keys or
`Tab` / `Shift+Tab` to select a result, `Enter` to jump, and `Esc` to cancel.

`Ctrl+Space` also opens a leader-command menu from the dashboard, the same
way it does inside a session (below) — that's where bookmarks and a few
other extras live; see
[More Session Tools](@/docs/session-tools.md).

## Embedded session

Most keys go directly to the active session. These controls belong to AIW:

| Key | Action |
| --- | --- |
| `Ctrl+Q` | Return to the dashboard |
| `Ctrl+Space` | Open the leader-command menu |
| `Ctrl+Space`, then `w` | Switch sessions |
| `Ctrl+Space`, then `i` | Jump to an agent needing attention |
| `Ctrl+Space`, then `f` | Start final diff review |
| `Ctrl+Space`, then `p` | Open the prompt library |
| `Ctrl+Space`, then `E` | Edit headless AI prompt templates (overrides) |
| `Ctrl+Space`, then `N` | Add a TODO to this worktree's list |
| `Ctrl+Space`, then `g` | Generate a short AI summary of this feature's recent activity |
| `Ctrl+Space`, then `?` | Show all leader commands |

## Split view

`v` on a feature tiles its running agent sessions: 1 fills the screen, 2 sit
side by side, 3 are one large plus two stacked, 4 are a 2×2 grid; more than 4
are paged. `v` on a project row tiles the first running agent of each of its
features. Every tile is live; the focused one (thick, bright border) gets all
your keystrokes and pastes. A tile whose agent needs you has a warning-coloured
border. Each tile's tmux window is resized to the tile while it is shown.

| Key | Action |
| --- | --- |
| any key | Goes to the focused tile's agent |
| click | Focus the tile under the pointer |
| `Ctrl+Space`, then arrows (or `h` `j` `k` `l`) | Move focus between tiles |
| `Ctrl+Space`, then `z` | Zoom the focused tile into the full session view; `Ctrl+Space`, `z` there comes back |
| `Ctrl+Space`, then `[` / `]` | Previous / next page (more than 4 sessions) |
| `Ctrl+Space`, then `i` | Focus the tile that has needed you longest |
| `Ctrl+Space`, then `v` (or `Ctrl+Q`) | Back to the dashboard |
| `Ctrl+Space`, `Ctrl+Space` | Send `Ctrl+Space` to the agent |

Status (working / needs you / finished) is tracked per feature, so tiles of the
same feature share it.

### `aiw grid` — the same agents in any terminal

`aiw grid <feature>` (or `aiw grid`) shows a feature's agents as real tmux
panes in the current terminal, at native speed — handy on a second monitor.
`aiw grid <project>` shows the first agent of each feature; use
`project/feature` when a feature name exists in several projects, `--all` to
include terminals, `--list` to print what it would show. Click a pane to focus
it; `Alt+q` or closing the terminal ends the grid. Each pane is a grouped view
session on AI Workstation's tmux socket, so the grid never moves your sessions'
windows and leaves nothing behind when it closes.

## Where the rest live

- [Understanding a Codebase](@/docs/learning-mode.md) — the file-tree, ask,
  and act-on-answer keys inside Learning Mode.
- [Reviewing Changes and PR Feedback](@/docs/review-and-pr-feedback.md) —
  PR Triage's investigate/fix/reply keys.
- [Prompts and TODOs](@/docs/prompts-and-todos.md) — the TODO list's own
  scope-toggle, move/copy, and work-the-queue keys.
- [More Session Tools](@/docs/session-tools.md) — bookmark slots, resuming
  a harness's saved transcripts and the fresh-context handoff keys.
