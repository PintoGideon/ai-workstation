# Development checks

Use current stable Rust with rustfmt and Clippy. AIW uses edition 2024 and builds
bundled SQLite/tree-sitter C code, so a C compiler/linker is required. The minimum
compiler version is not separately tested; CI uses stable.

## Local setup and iteration

The full suite runs on Unix and uses temporary Git repositories, shell/process
fixtures, Unix sockets and FIFOs. Install `git`, `tmux`, `sh`, `bash`, `ps`, `sleep`,
`mkfifo` and a C toolchain. On Ubuntu, CI installs `build-essential pkg-config git
tmux bash coreutils procps`; on macOS install tmux with Homebrew and use the
Xcode command-line tools. No authenticated GitHub account or paid agent harness
is required for tests. Running AIW itself requires whichever of Claude Code,
Codex, Pi, Gemini CLI and Cursor agent you select.

```sh
cargo build --locked
cargo test --locked app::tests::feature_sessions
```

For a code milestone, run the normal parallel suite and both checks:

```sh
cargo test --workspace --locked
cargo fmt --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

The live `gui_terminal` tests run concurrently in separate child test processes.
Each child uses its own test-only tmux socket and a keeper session so fixture
setup cannot race the server's last-session exit. Only server startup is retried;
transport assertions run once. The parent enforces a 90-second timeout and kills
the child's private server and removes its socket directory on success, failure
or timeout. Child stdin is closed, exercising desktop-style terminal attachment
without changing the test runner's stdin. No test uses the user's AIW tmux server.

The WSL-only clipboard integration test accesses the real Windows clipboard.
Run with functioning WSL interop. A restricted sandbox can prevent Unix socket
binding or clipboard access; rerun in an environment providing those facilities
and report the restriction rather than ignoring or serializing tests.

## Adding or moving code and tests

Use [architecture.md](architecture.md) to choose the owner. Add App integration
coverage to the appropriate `src/app/tests/` suite; put pure unit coverage beside
the behavior it tests. Share only fixtures with actual multi-suite consumers in
`tests/support.rs`, with test-only narrow visibility. Keep temporary directories,
DB handles and process guards alive for the whole test. Use `App::new_for_test`
with MockTmuxOps/MockWorktreeOps; new feature groups should use the same defaults
in production and tests. Do not make production APIs public to accommodate moves.

Keep pure transformations explicit about their inputs. App orchestration owns
mode changes and calls existing DB/external-operation boundaries. When changing
worker ownership, exercise closing with pending work, switching target/session,
reopening, and stale or disconnected results. Tests for an existing regression
should remain intact; do not add tests merely for a file's location.

For this refactor's preservation check, capture `cargo test --workspace --locked
-- --list`. Transform the original names in [baseline/tests.txt](baseline/tests.txt)
through [the M2 mapping](baseline/app-tests.tsv) and
[the state mapping](state-test-paths.tsv), then compare the full sets and counts.
All 2,511 originals were accounted for at the time of that refactor; the
2026-10-11 removal of the harness-duplicating features deleted their suites.

Never use `println!`/`eprintln!` in TUI code. Use App's `log_debug`, `log_info`,
`log_warn`, or `log_error` with a short context such as `sync` or `worktree`.
Dashboard `D` opens the log; the file is `~/.local/state/ai_workstation/debug.log`.

## CI jobs

[main.yml](../../.github/workflows/main.yml) is the only workflow. It runs one
Linux job, `Test`, on pushes to `main` and `ai-workstation`, on pull requests,
and on demand. The job mirrors the local gate: `cargo fmt --check`,
`cargo clippy --all-targets --locked -- -D warnings`, `cargo test --locked` and
`scripts/check-no-network.sh`. Workflow permissions are `contents: read`.

This is a personal fork, so the upstream pipelines are gone: no macOS matrix,
no release workflow (binaries are built locally with `cargo install --path .`),
no site deploy, no scheduled RustSec audit and no crates.io publish. Run
`cargo audit` by hand when you update dependencies; the last local scan is in
[advisories.md](advisories.md). The desktop GUI crate needs WebKitGTK and is
checked locally with `cargo clippy --workspace` and `cd gui && npm test`.

## End-to-end run (fake agents, sandboxed)

Pass the binary as an absolute path, and on a machine whose `/tmp` has a tight
per-user quota run both the unit suite and the e2e with `TMPDIR` and the sandbox
under `/var/tmp`:

```sh
export TMPDIR=/var/tmp/aiw-tests
scripts/dev/e2e/run.sh /var/tmp/aiw-e2e "$PWD/target/release/aiw"
scripts/dev/e2e/multi-project.sh /var/tmp/aiw-e2e-mp "$PWD/target/release/aiw"
```

`multi-project.sh` (DESIGN.md §19.11) runs every orchestration feature together
in one aiw across six repository shapes (a Rust crate, a Node package, a Python
package, a monorepo, a `trunk` repository and one with a bare remote and a fake
`gh`), including the cross-feature cases: landing a fan-out sibling, deleting a
group that holds a fan-out pair, a schedule on a forked session, a message to a
background session, and the watchdog leaving a waiting session alone.

A real-harness smoke run (real `claude` and `codex`) is manual and costs a
little; DESIGN.md §19.11 records how it was run and what it found. Point aiw at
sandboxed `XDG_CONFIG_HOME` / `XDG_STATE_HOME` / `XDG_DATA_HOME` and create
`$XDG_CONFIG_HOME/ai_workstation` first (otherwise aiw falls back to an existing
`~/.config/ai_workstation`), give Codex a sandboxed `CODEX_HOME`, and note that
aiw's Claude hook scripts always live under `$HOME/.config/ai_workstation/hooks`.

### The main run


`scripts/dev/e2e/run.sh` drives the release `aiw` binary through tmux against
five fake agent CLIs (`claude`, `codex`, `gemini`, `cursor-agent`, `pi`) that
behave like interactive REPLs and fire the hooks AIW installs into the checkout
(`.claude/settings.local.json`, Codex `-c notify=…`, `.gemini/settings.json`,
`.cursor/hooks.json`) with realistic payloads. Typed commands make them
deterministic: `/ask` (permission request), `/done`, `/work`, `/tool`,
`/crash`, `/spam`, `/wide`, `/slow`, `/exit`.

```sh
cargo +1.99.0 build --release --locked
scripts/dev/e2e/run.sh            # sandbox in /tmp/aiw-e2e-$USER, prints PASS/FAIL
```

Everything the run writes stays under the sandbox (`HOME` and `XDG_*` point at
it; it must be a short path because tmux and IPC sockets live under it). It
walks the README quick start (first run, projects, features with every agent,
sessions, hooks → header pills and `i`, the grid with resizes, rename, stop/
start, resume, delete, quit/relaunch, `doctor`), then adversarial cases (killed
windows, deleted worktrees, corrupt `aiw.json` and its toast, ghost hooks,
removed IPC socket, a second `aiw` instance against the lock, an agent that
exits at its shell (`/crash` → ■ exited → `c` relaunches in place), a feature
name with a space, a malformed notification file, `Esc` on the dashboard, odd
names, random keys, small terminals) and the AMF migration cases
(`migration.sh`: plain, locked database, no database, OpenCode-only). Text
captures for UI review are left in `<sandbox>/captures`. The pieces are
reusable on their own: `setup-sandbox.sh` builds the sandbox, `lib.sh` has the
tmux driver helpers (`aiw_start`, `keys`, `cap`, `wait_for`, `snap`, …).

## Verification status

After the 2026-10-11 removal of the harness-native duplicates (DESIGN.md
§18.1) the full suite is 1,697 tests (4 ignored), with formatting, strict
locked all-target Clippy, `scripts/check-no-network.sh`, a release build and
the GUI's `tsc` and vitest all passing.

The earlier final local validation on 2026-09-08 passed all 2,512 tests (22.82s, no failures
or ignored tests), formatting, strict locked all-target Clippy, Actionlint 1.7.7,
and local documentation-link checks. The patched lockfile audit exited 0 with
zero blocking findings and five documented nonfatal warnings.

Local Linux/WSL baseline and inventory evidence are in
[baseline/README.md](baseline/README.md). Hosted Linux/macOS/audit runs must be
inspected for the published commit before CI is called green. Workflow linting
and a local audit do not establish macOS execution or hosted success. Publishing
this work remains a separate authorized step.
