# AI Workstation — fork maintenance

AI Workstation (AIW) is our own fork of the upstream project credited in [README.md](README.md), MIT licensed.
Design and roadmap: [DESIGN.md](DESIGN.md).

## License obligations

The MIT license lets us copy, modify, rename, use at work and redistribute. One obligation: **keep `LICENSE` with
Ryan Eldridge's copyright notice** in every copy (source or binary bundles). Add our own copyright line below it for
our changes; never remove theirs. The README keeps a one-line credit to the original project.

## Build and install

```bash
cd ~/ai_workstation
cargo +1.99.0 build --release --locked          # ~5–10 min the first time
./target/release/aiw -V                         # AI Workstation (aiw) 0.49.0
./target/release/aiw doctor                     # read-only health report
cargo +1.99.0 install --path . --locked         # daily use: puts `aiw` in ~/.cargo/bin
```

Run it in a **real terminal window** (Ghostty, iTerm, GNOME Terminal) — it is an interactive TUI. The desktop GUI is
the `aiw-gui` package in `gui/` (`cd gui && npm ci && npm run tauri build`).

**On the work Mac:** install `tmux` and Rust (`rustup`), `xcode-select --install`, clone *our* fork, then the same
commands. A local source build avoids the unsigned-download Gatekeeper warning.

## What AIW writes, and where

| Where | What | When |
|---|---|---|
| `~/.config/ai_workstation/` | settings (`config.json`), database (`aiw.db`), port reservations, notifications | first launch |
| `~/.config/ai_workstation/hooks/` | Claude hook scripts | first launch |
| `~/.local/state/ai_workstation/` (Linux) · `/tmp/ai_workstation/` (macOS) | private tmux socket, IPC socket, debug log — your own tmux is untouched | first launch |
| `~/.local/share/ai_workstation/worktrees/` | feature worktrees | when a feature starts |
| inside each project repo | `.aiw/`, `.claude/settings.local.json`, `.claude/skills/aiw-*`, `.codex/aiw-codex-notify.sh`, `.agents/skills/aiw-*`, `.gemini/`, `.cursor/hooks.json` — all added to `.git/info/exclude` | when features start |

AIW never edits global agent config (`~/.claude/settings.json`, `~/.codex/config.toml`); Codex gets its `notify`
hook per launch. Coming from the upstream tool? The first launch copies its setup; see the migration section of
the README.

**Remove everything** (after `d` on each feature and `q`):

```bash
tmux -S ~/.local/state/ai_workstation/tmux.sock kill-server 2>/dev/null   # macOS: /tmp/ai_workstation/tmux.sock
rm -rf ~/.config/ai_workstation ~/.local/state/ai_workstation ~/.local/share/ai_workstation
```

## Upstream: cherry-pick, don't merge

We no longer merge upstream. The rename touched almost every file, so a merge would conflict everywhere. Instead we
read upstream's history now and then and **cherry-pick the commits worth having**, by hand.

The `upstream` remote stays, for reference only (on a fresh clone, add it again with the repository URL from the
README credit). Make sure it can never be pushed to:

```bash
git remote -v                                     # upstream  <original repository> (fetch)
git remote set-url --push upstream no_push        # one-time: pushing to upstream is an error
```

Picking a fix:

```bash
git fetch upstream
git log --oneline --no-merges main..upstream/main          # what is new upstream
git show <sha>                                              # read it first
git switch -c pick/<topic>                                  # never cherry-pick straight onto main
git cherry-pick -x <sha>                                    # -x records "(cherry picked from commit …)"
# Conflicts are expected: upstream still uses its old names. Translate them while resolving
# (DESIGN.md §11 has the full map): its crate → ai_workstation, its env-var prefix → AIW_,
# its tmux prefix → aiw-, its config folder → ~/.config/ai_workstation, its repo files → aiw.json / .aiw/,
# its command → aiw.
git cherry-pick --continue
cargo +1.99.0 fmt && cargo +1.99.0 test --locked && cargo +1.99.0 clippy --all-targets --locked
scripts/check-no-network.sh                                 # upstream code may bring network calls back
```

Several commits in a row: `git cherry-pick -x <first>^..<last>`. Give up on one: `git cherry-pick --abort`.
Note what was picked (and what was deliberately skipped) in the PR description; `git log --grep "cherry picked
from"` lists everything picked so far.

What not to pick: anything that adds network calls (self-upgrade, remote control, telemetry — see
[NO_CLOUD.md](NO_CLOUD.md)), OpenCode support (removed, DESIGN.md §14), and renames back to the old names.

## Rules for our own code

1. **Add, don't edit.** New code lives in new files under `src/fork/` (and `src/ui/dialogs/fork_*.rs`).
2. **Small, marked hooks into inherited code.** When inherited code must call ours, change as few lines as possible
   and mark them `// FORK: <reason>`. `git grep "FORK:"` lists every touch point — the places to check first when a
   cherry-picked upstream commit touches the same code.
3. **Legacy names are read, never written.** Code that still accepts an old upstream name (config files, theme,
   hook paths, tmux session names) is marked `FORK:` and listed in DESIGN.md §11.
