#!/usr/bin/env bash
# Create an isolated sandbox for an AIW end-to-end run.
#
#   scripts/dev/e2e/setup-sandbox.sh <sandbox-dir> [aiw-binary]
#
# Writes <sandbox-dir>/env.sh (source it to get HOME/XDG_* pointed at the
# sandbox and the fake agents first on PATH), five fake agent CLIs under
# <sandbox-dir>/bin, three demo git repos + one plain folder under
# <sandbox-dir>/repos, and nothing anywhere else. The real ~/.config,
# ~/.local/state and the user's agent folders are never touched.
set -euo pipefail

S=${1:?sandbox dir}
AIW_BIN=${2:-}
HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
ROOT=$(cd "$HERE/../../.." && pwd)
[ -n "$AIW_BIN" ] || AIW_BIN="$ROOT/target/release/aiw"
[ -x "$AIW_BIN" ] || { echo "aiw binary not found: $AIW_BIN (cargo +1.99.0 build --release --locked)" >&2; exit 1; }
# The sandbox links and exports this path, and hooks run it from other
# directories: a relative path would silently fall back to whatever `aiw` is on
# the real PATH (e.g. ~/.cargo/bin/aiw).
AIW_BIN="$(cd "$(dirname "$AIW_BIN")" && pwd)/$(basename "$AIW_BIN")"

mkdir -p "$S"/{bin,log,repos,.config,.local/state,.local/share,captures,.codex/sessions,.claude/projects}
S=$(cd "$S" && pwd -L)
# Unix sockets are limited to ~107 bytes; AIW's lives at
# $S/.local/state/ai_workstation/tmux.sock and the driver's at $S/drive.sock.
if [ "${#S}" -gt 60 ]; then
    echo "sandbox path is ${#S} chars; use a shorter path (or a symlink) so tmux/IPC sockets fit" >&2
    exit 1
fi

for name in claude codex gemini cursor-agent pi; do
    cp "$HERE/fake-agent.py" "$S/bin/$name"
    chmod +x "$S/bin/$name"
done
ln -sf "$AIW_BIN" "$S/bin/aiw"

cat >"$S/env.sh" <<EOF
# source me
export S="$S"
export HOME="$S"
export XDG_CONFIG_HOME="$S/.config"
export XDG_STATE_HOME="$S/.local/state"
export XDG_DATA_HOME="$S/.local/share"
export PATH="$S/bin:\$PATH"
export AIW_E2E_LOG="$S/log"
export AIW_BIN="$AIW_BIN"
export GIT_AUTHOR_NAME=e2e GIT_AUTHOR_EMAIL=e2e@example.invalid
export GIT_COMMITTER_NAME=e2e GIT_COMMITTER_EMAIL=e2e@example.invalid
export RUST_BACKTRACE=1
unset TMUX TMUX_PANE
EOF
# shellcheck disable=SC1090
source "$S/env.sh"

make_repo() {
    local dir=$1 name=$2
    rm -rf "$dir"
    mkdir -p "$dir/scripts"
    git -C "$dir" init -q -b main
    printf '# %s\n\nDemo repo for the AIW e2e run.\n' "$name" >"$dir/README.md"
    printf '.env\n*.secret\nnode_modules/\n' >"$dir/.gitignore"
    printf 'SECRET=%s-token\n' "$name" >"$dir/.env"
    printf 'local\n' >"$dir/local.secret"
    printf '.env*\n*.secret\n' >"$dir/.worktreeinclude"
    cat >"$dir/scripts/hook.sh" <<'H'
#!/usr/bin/env bash
# aiw.json lifecycle hook: records that it ran, in the checkout it ran in.
echo "hook ran at $(date +%s) in $PWD choice=${AIW_HOOK_CHOICE:-}" >>"$PWD/.aiw-hook-ran"
exit 0
H
    chmod +x "$dir/scripts/hook.sh"
    cat >"$dir/aiw.json" <<J
{
  "lifecycle_hooks": {
    "on_worktree_created": "scripts/hook.sh",
    "on_start": "scripts/hook.sh"
  },
  "custom_sessions": [
    {
      "name": "logs",
      "description": "tail the demo log",
      "command": "echo custom-session-$name; tail -f README.md",
      "window_name": "logs"
    }
  ]
}
J
    git -C "$dir" add -A
    git -C "$dir" commit -qm "init $name"
    printf 'fn main() {}\n' >"$dir/main.rs"
    git -C "$dir" add -A && git -C "$dir" commit -qm "add main"
    printf 'v2\n' >>"$dir/README.md"
    git -C "$dir" add -A && git -C "$dir" commit -qm "readme v2"
}

make_repo "$S/repos/alpha" alpha
make_repo "$S/repos/beta" beta
make_repo "$S/repos/gamma" gamma
# a branch "already checked out elsewhere" scenario helper
git -C "$S/repos/alpha" worktree add -q "$S/repos/alpha-other" -b taken-branch >/dev/null
# non-git folder
rm -rf "$S/repos/plain" && mkdir -p "$S/repos/plain"
printf 'not a git repo\n' >"$S/repos/plain/notes.txt"

echo "sandbox ready: $S"
echo "  source $S/env.sh"
