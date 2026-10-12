# Dev scripts

Tooling for developing AIW itself — not shipped to end users, not
embedded in the binary. There is no release pipeline in this personal
fork: install with `cargo +1.99.0 install --path . --locked`.

- `test-thinking.sh` — manually pulse the "thinking" sentinel for a
  tmux session to test dashboard detection without a real agent
- `e2e/` — sandboxed end-to-end run of the TUI against fake agent CLIs
  (`e2e/run.sh`; see [development checks](../../docs/development/checks.md))
- `aiw/pr-checks.sh`, `aiw/pr-info.sh` — PR context helpers used by
  the `aiw:pr-*` Claude Code skills

For focused and full tests, runner prerequisites and debug logging, see [development checks](../../docs/development/checks.md).
Choose feature modules and test suites using the
[architecture guide](../../docs/development/architecture.md).
