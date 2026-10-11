# AGENTS.md

Guidance for AI coding agents working in this repository.

## Build and validation

The package is `ai-workstation`; its binary is `aiw`. Use the current stable
Rust toolchain (edition 2024), with a C compiler for bundled SQLite/tree-sitter.

```bash
cargo build --locked
cargo run --locked
cargo test --locked app::tests::feature_sessions
cargo test --locked app::pr_review
cargo test --workspace --locked
cargo fmt --check
cargo clippy --workspace --all-targets --locked -- -D warnings
```

The repository has unit and integration tests. Choose a focused suite while
iterating; run the full parallel suite, formatting and strict Clippy at a code
milestone boundary. Do not ignore or serialize tests to hide a regression.
See [development checks](docs/development/checks.md) for prerequisites, CI,
dependency-advisory triage and the exact baseline comparison procedure.

## Runtime and architecture

AIW is a Rust TUI built on ratatui/crossterm/vt100. It supports Claude Code,
Codex, Gemini CLI, Cursor agent and Pi sessions in tmux. Source installations need `tmux` on PATH;
install the harnesses you intend to use. Tests mock paid harness execution.

Use [the architecture guide](docs/development/architecture.md) to choose a module.
`cli.rs` owns startup, event deadlines and polling; `handlers/` dispatches input;
`ui/` renders; `app/` orchestrates feature workflows; `db/` owns persistence.
PR Triage and Final Review each have a directory with owned state.
Central AppMode remains the routing boundary. Feature test suites live under
`src/app/tests/`, with shared fixtures in `support.rs`; small unit suites stay
with their implementation.

Preserve persistence formats, keybindings, worker identity checks and process
lifetimes during refactors. Keep pure transformations independent of App. Inject
tmux/worktree operations through `TmuxOps`/`WorktreeOps`; do not add persistence or
process launching to renderers. Keep temporary directories, databases and child
guards owned by each test, using `App::new_for_test` and group defaults.

State is persisted in the global SQLite store at `~/.config/ai_workstation/aiw.db` for every
checkout. Tmux sessions use the `aiw-` prefix. Configuration controls leader-key
and refresh timings; consult the code/config instead of assuming fixed intervals.

### Debug Logging

**NEVER use `println!` or `eprintln!` in TUI code** - it corrupts
the terminal display. Use the built-in debug log instead.

To view the debug log at runtime, press `D` from the dashboard.

**Log file location:** `~/.local/state/ai_workstation/debug.log`

You can tail this file in a separate terminal:
```bash
tail -f ~/.local/state/ai_workstation/debug.log
```

**Usage in code:**

```rust
// From anywhere with access to `app`:
app.log_debug("context", format!("value: {}", value));
app.log_info("context", "operation completed".to_string());
app.log_warn("context", "something unexpected".to_string());
app.log_error("context", format!("failed: {}", err));
```

**Log levels** (color-coded in UI):
- `DEBUG` (gray) - detailed tracing
- `INFO` (green) - normal operations
- `WARN` (yellow) - unexpected but handled
- `ERROR` (red) - failures

**Context strings** should be short identifiers like:
- `"aiw"` - app lifecycle
- `"sync"` - status sync operations
- `"tmux"` - tmux interactions
- `"worktree"` - git worktree operations
- `"hooks"` - lifecycle hooks

Errors from `show_error()` are automatically logged to the
debug log with level ERROR.

### Local harness configuration

- **Never modify `~/.claude/settings.json` (global) or
  `~/.codex/config.toml` (global Codex config) to inject
  hooks or settings.** Instead, write to the worktree's
  local `.claude/settings.local.json` (or the agent's per-worktree equivalent)
  via `ensure_notification_hooks()`. For non-worktree
  features (first feature that uses the repo dir directly),
  write to `{repo}/.claude/settings.local.json`. On startup,
  `cleanup_global_hooks()` actively removes any
  previously-injected global entries.

## Screenshot proof

Use the repository `aiw-screenshot` skill only when the user explicitly asks
for visual proof. Captures are local (`scripts/dev/screenshot/aiw-capture.sh`,
rendered with `render_ansi.py` / `assemble_gif.py`); this fork has no PR
screenshot publisher, so deliver the PNGs/GIF as files.
