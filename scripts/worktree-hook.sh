#!/bin/bash
# AIW WorktreeCreate / WorktreeRemove hook for Claude Code (DESIGN.md §19.6).
#
# Claude runs this instead of its own `git worktree add` when it creates a
# worktree (`--worktree`, the EnterWorktree tool, a subagent with
# `isolation: worktree`, a workflow agent, background-session isolation), so
# the worktree lands in AIW's worktree folder with the repo's included files
# and its own port block, and shows up under the feature in the sidebar.
#
# argv[1] is `create` or `remove`. The hook JSON is passed straight through on
# stdin. For `create`, `aiw worktree-hook create` prints the worktree's
# absolute path as the last line on stdout (what Claude reads) and everything
# else on stderr. Unlike AIW's other hooks this one must not swallow output or
# the exit status: a non-zero exit fails the create, or refuses the removal.
# No JSON is parsed here.

# $AIW_BIN names the aiw that started this session. A rebuilt or deleted
# build leaves it pointing at nothing, so fall back to `aiw` on PATH.
[ -x "${AIW_BIN:-}" ] || AIW_BIN=aiw
exec "$AIW_BIN" worktree-hook "${1:-}"
