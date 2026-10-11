+++
title = "Installation"
description = "Build and install AIW from source."
weight = 10
+++

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
  triage, posting final-review feedback, and the dashboard's
  open/merged/closed PR badge.
- A [Nerd Font](https://www.nerdfonts.com/) provides the best icon rendering.
- The `code` command enables VS Code sessions.

## Build from source

AIW publishes no release bundles. Building needs Rust (via
[rustup](https://rustup.rs)), a C compiler, and `tmux`:

```bash
cd ~/ai_workstation                      # your clone of the fork
cargo +1.99.0 install --path . --locked  # installs `aiw` into ~/.cargo/bin
aiw -V
```

## Upgrade

AIW has no self-upgrade: pull the fork and run the same `cargo install`
command again. See `CHANGELOG.md` in the repository for release notes.

