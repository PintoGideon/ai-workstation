# Dev scripts

Tooling for developing and releasing AIW itself — not shipped to end
users, not embedded in the binary.

- `release.sh` — bump the version, validate `CHANGELOG.md`, tag, and
  push a release
- `package-release-bundle.sh` — used by the release CI workflow to
  package a built binary into a release archive
- `package-no-tmux-test-bundle.sh`, `run-no-tmux-docker.sh` — build
  and run the no-tmux Docker test image (see
  [`docs/docker-no-tmux.md`](../../docs/docker-no-tmux.md))
- `test-thinking.sh` — manually pulse the "thinking" sentinel for a
  tmux session to test dashboard detection without a real agent
- `e2e/` — sandboxed end-to-end run of the TUI against fake agent CLIs
  (`e2e/run.sh`; see [development checks](../../docs/development/checks.md))
- `aiw/pr-checks.sh`, `aiw/pr-info.sh` — PR context helpers used by
  the `aiw:pr-*` Claude Code skills

For focused and full tests, runner prerequisites, dependency audits and debug
logging, see [development checks](../../docs/development/checks.md).
Choose feature modules and test suites using the
[architecture guide](../../docs/development/architecture.md).
