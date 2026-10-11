+++
title = "Quick Start"
description = "Get a first project and feature running."
weight = 20
+++

1. Start AIW from a normal shell:

   ```bash
   aiw
   ```

2. On first launch, choose the agent CLIs you want AIW to use. AIW checks that
   each selected CLI is installed; press `c` to save. You can reopen this
   setup later with `A`.

3. Press `N` to add a project, then enter its name and directory. The
   directory may be a git repository or an ordinary folder.

4. Press `n` to create a feature. Choose a name, whether it gets a worktree,
   a harness, and a permission mode. The agent starts in its own tmux pane on
   the right.

5. Press `Enter` on the agent session to put the keyboard in its pane, or
   `Ctrl+Q` to flip between the pane and the sidebar.

6. Press `s` on a feature to add another agent, terminal, editor, TODO list,
   or custom session.

<figure>
  <img src="/images/dashboard.png" alt="AIW dashboard showing a project with four concurrent Claude agent sessions, each on its own git worktree">
  <figcaption>A project with several features running side by side, each in its own worktree.</figcaption>
</figure>

Press `?` at any time on the dashboard to see the complete, current
keybinding reference.
