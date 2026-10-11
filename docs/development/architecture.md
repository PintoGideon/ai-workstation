# Architecture

AIW is a single Rust executable. It retains the existing event loop and persistence
formats; feature extraction does not introduce a new framework or library.
See [checks.md](checks.md) for setup, focused suites, CI and contribution guidance.
The [baseline inventory](baseline/README.md) records the pre-refactor ownership map
and complete test preservation evidence.

## Runtime and dependency directions

```text
main (startup, events, deadlines, worker polling)
  -> handlers (key/mouse dispatch) -> App feature methods
  -> ui (dashboard, pane, pickers, dialogs) -> display state/helpers
App feature methods -> project/domain data + feature state
                    -> db::AiwDb + feature persistence modules
                    -> external managers / TmuxOps / WorktreeOps
workers -> channels/mailbox -> feature poll/apply methods -> AppMode/display state
```

`src/main.rs` is a thin binary that calls `ai_workstation::cli::run`;
`src/cli.rs` owns CLI dispatch, terminal setup/teardown and `run_loop`. The loop
reads terminal/IPC events, polls workers, reconciles session state, schedules
pane refreshes and redraws. It uses deadlines and mode-dependent work. The PR
badge refresh has its own cadence.

`src/handlers/mod.rs` routes by AppMode. Handlers translate input into feature
operations and sometimes edit dialog fields directly. `src/ui/dashboard.rs`
dispatches renderers; `ui/dialogs/` renders overlays. Neither layer should gain
persistence or process-launch responsibilities.

`src/app/mod.rs` constructs App/AppConfig, declares features and owns shared
pane/sidebar infrastructure, logging and configuration access. Features remain
`impl App` orchestration. `src/app/state.rs` keeps AppMode and shared routing,
configuration, session and TODO/plan state, with compatibility re-exports of the
three extracted features' types. Cross-feature mode transitions remain on App.

The desktop prompt-library adapter in `src/gui_prompts.rs` reads the same
source merge as the TUI, and calls the shared search,
placeholder/default and substitution helpers in `src/prompt_library.rs`.
Content keys exclude generated config IDs/dates and include checkout paths;
insertion reloads templates and allowed session targets before returning an
unsent string. `PromptLibraryPanel.tsx` owns only local selection/fill state,
while `App.tsx` appends the handoff to the session draft after a final workspace
cache check. Neither browsing nor preparation launches a process or writes the
library. Library authoring remains a separate workflow.

`src/gui_syntax.rs` highlights GUI diffs and reviews
through the TUI's `highlight` service (detection, installed tree-sitter
parsers, cache). It highlights each diff file's whole base and current
contents and sends per-line token spans that must reproduce the line text
exactly, so multi-line strings and comments are classified with full-file
context. Per-file size limits and a per-projection byte budget bound the
first, uncached load. `SyntaxInfo` explains plain-text fallbacks, and
`install` runs the TUI picker's installer on a background thread. The
frontend's `SyntaxCode.tsx` renders spans as `syn-*` classes coloured by the
`--syn-*` tokens in `syntax.css`.

## Feature modules

The harness-duplicating feature directories (PR Triage, Final Review,
Learning, the plan interview) were removed on 2026-10-11; the workstation
workflows live in the flat `src/app/*.rs` modules.

Feature roots expose consumed entrypoints and compatibility re-exports. Free
helpers are imported from their owning sibling module; new private helper access
is bounded to the original feature. Extracted modules import explicit App types.
Pure transformations take data rather than an all-purpose App/context object.
State/domain code does not import handlers or renderers. Orchestration may use
existing cross-feature APIs for TODO/session handoffs.

Shared DiffScope/layout concepts belong to ordinary diff browsing
(`app/diff.rs`, `app/diff_state.rs`).

## Runtime ownership and stale results

The background-work ownership groups (`PrReviewWork`, `AiReviewRun`,
`LearningRuns`) went with their features on 2026-10-11; the
[app-fields.tsv](baseline/app-fields.tsv) baseline predates that. Shared routing/store/configuration,
manager injection, IPC/watcher/observer guards, pane/sidebar queues and debug/perf
remain on App. Refresh caches and cross-feature notification state remain
available for later incremental ownership work.

## Data and side effects

`src/project.rs` models projects, features and sessions. `db/mod.rs` owns the
SQLite connection and open/seed entrypoints; `migrations.rs` owns schema changes;
`store.rs` persists projects. Other DB modules persist session status, tokens,
debug logs, editors, prompt templates/overrides, TODOs,
the merged/closed PR badge state. DB code knows
project/domain types and uses WorktreeManager to resolve legacy stores.

`src/traits.rs` provides mockable TmuxOps and WorktreeOps, implemented by
`tmux.rs` and `worktree.rs`. GitHub calls live in `github.rs`; its
`GithubTransport` trait is the shared boundary for GitHub-backed workflows,
with `GhCli` as the production adapter for the issue browser and the PR badge
sweep. The compact feature setup state the issue browser uses lives in
`app/feature_setup.rs`. Harness
integration supports Claude Code, Codex, Pi, Gemini CLI and Cursor agent. IPC, filesystem
watching, tmux observation, editors and resource inspection have dedicated
modules. Injection is partial: App workflows still call concrete managers and
filesystem APIs. Preserve those boundaries and process lifetimes; introduce an
adapter only where it establishes a concrete dependency boundary.

## Tests

`src/app/tests/mod.rs` declares behavior suites and shared `support.rs`:

| Suite | Coverage |
| --- | --- |
| `startup_navigation`, `pane_input` | Startup, dashboard navigation and pane input |
| `status_sidebar`, `attention` | Sync, IPC status, sidebar caches, badges and attention reconciliation |
| `feature_sessions`, `hooks_setup`, `automation` | Creation/recovery/start/stop, local hooks and automation entrypoints |
| `prompts_configuration`, `todos` | Configuration/prompts and scoped TODO workflows |
| `resources`, `editor_lifecycle` | Admission, process identity and editor stop races |

Preserve the old/new path maps when auditing coverage.
Shared helpers belong in support only when multiple suites consume them. Temporary
repositories, DBs and process guards remain per-test, with their original lifetimes.

## Deferred reliability work

Store overwrite, migration atomicity, repository cache identity, IPC socket
ownership, startup resource handling and synchronous GitHub-call findings remain
separate reliability work. This refactor changes no persistence schema or user
workflow to address them. No extraction blocker was found; inspect exact identity
and cleanup guards before any future ownership change.
