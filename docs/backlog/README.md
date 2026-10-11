# Backlog

Design docs for work that is **not in active development** — whether
planned but not yet started, or started, paused, and not yet finished.
Each file captures the reasoning, shape, and open questions for one
feature so the decision and context survive until someone picks it up
again — and so "why haven't we finished this" has a written answer.

For partially-built work, record **what's done and what's left** so the
next person can resume without re-deriving it. When a doc describes work
that is actively being implemented right now, keep the live plan next to
the code if you prefer, and trim the backlog entry to a pointer.

## Conventions

- One feature per file, named in kebab-case and **ending in `-plan`**
  (e.g. `remote-control-server-mode-plan.md`). The `plan` in the name
  matters: AIW's in-app Markdown viewer filters to files whose path
  contains "plan", so this is what makes a backlog doc show up there.
- Keep each doc **self-contained**: a reader should understand the
  proposal without opening other files, though linking to related plans
  or code is encouraged.
- Start each doc with a short status block (see the template).
- Add the doc to the Index below.

## Status values

- `Backlog` — captured, not scheduled, not started.
- `Designing` — actively being fleshed out.
- `Ready` — design settled, ready to implement.
- `Partial` — implementation started then paused; not finished. Record
  what's done and what's left in the doc.
- `In progress` — actively being implemented right now (consider keeping
  the live plan next to the code and trimming this entry to a pointer).
- `Dropped` — decided against; keep the doc with the reason.

## Template

Use a checklist under **Progress** so the state of each item is explicit
— `[x]` done, `[ ]` not done. Keep it current as work lands so a paused
doc always says exactly what remains.

```markdown
# <Feature name>

- **Status:** Backlog
- **Owner:** <who, or unassigned>
- **Relates to:** <links to shipped features, plans, code, issues>

## Why / problem

## Proposed design

## Progress

- [ ] <implementation item>
- [ ] <implementation item>

## Open questions

## Reasoning / when to build
```

## Index

- [Docs site feature coverage](docs-site-coverage-plan.md) — _All 7 epics
  shipped._ Audit of `site/content/docs/` against `src/` and `CLAUDE.md`
  turned up shipped features with no site coverage: two (`aiw doctor`, the
  automation CLI) already documented in README and just missing a site
  page, plus context-window/usage tracking, a multi-harness capability
  matrix, the `aiw.json` project-config schema, session bookmarks/saved-
  transcript resume/Fresh Context handoff, Remote Control, task-prompt
  coaching, and combined-batch
  fix-cost disclosure. Landed as three new pages —
  [Choosing a Harness](../../site/content/docs/harnesses.md),
  [Project Config Reference](../../site/content/docs/project-config.md),
  [More Session Tools](../../site/content/docs/session-tools.md) — plus a
  `troubleshooting.md` page and new sections in `attention-and-limits.md`,
  `core-concepts.md`, `prompts-and-todos.md`, `review-and-pr-feedback.md`,
  and `keybindings.md`, all cross-linked. Verifying Epic 1 turned up and
  fixed a separate, pre-existing bug that had the whole site failing to
  build (see [bug backlog](bug-backlog-plan.md)). One follow-up open
  question: nothing was checked against a live `aiw` session across all
  four harnesses, only against source. _Several of the features those
  pages covered (review, PR feedback, custom prompts, Learning, Remote
  Control, Fresh Context, prompt coaching) were removed on 2026-10-11
  and their pages deleted._
- [Crate package size](crate-package-size-plan.md) — _Shipped._ The
  published crate was 20MB/703 files (11.1MiB compressed) — over
  crates.io's 10MB compressed cap, which is what turned every publish
  attempt into an opaque `503` instead of a clean rejection. Trimmed via
  a targeted `Cargo.toml` `exclude` list checked against the handful of
  files `src/` actually pulls in via `include_str!`, down to 8.2MiB/1.7MiB
  compressed.
- [Premium-model efficiency options](premium-model-efficiency-options-plan.md) —
  _Backlog._ Planning reference for eight ways AIW could make Astra/Fable more
  affordable, with worked savings calculations, implementation boundaries,
  evaluation criteria, and dependencies for later feature plans. OPT-07's
  model/reasoning analyzer shipped and was removed again on 2026-10-11
  (the harnesses' own `/model` and `/effort` replace it); the rest remains
  open.

- [Project config location](project-config-location-plan.md) — _Shipped._
  Moved hand-authored project config out of the generated
  `.aiw/` directory to `aiw.json` at the repo root, so `.aiw/` is purely
  generated and gitignorable with no force-tracked exception. Legacy
  `.aiw/config.json` is still read and migrates on first write; the
  directory gains an injected README explaining what it is.

- Plan Mode: guided feature discovery — _Removed 2026-10-11._ The native
  TUI interview, Quick Plan, `AIW_PLAN.md` and the per-project question
  banks were taken out; Claude Code's and Codex's own plan modes replace
  them.
- [Feature TODOs](feature-todos-plan.md) — _All epics shipped._ Scoped
  TODO lists added as a `SessionKind::Todos` session via the `s` picker
  (one per feature, native UI, SQLite-backed). A feature's editor opens
  on its own **worktree** list, with the **project** list (`p`) and a
  machine-wide **global** list (`g`) each independently toggled on or off
  as side panes; items move or copy between scopes. Each TODO carries
  priority, notes, and done state. Full editing
  (add/edit/notes/done/priority/reorder/delete
  + a per-list scratchpad), spawning an agent from a TODO with a
  pre-filled composer prompt, "implement next" across the visible
  scopes, and quick-capture from any session view
  are all in place. Deleting a feature asks what happens to the
  unfinished work in its worktree list.


- [Needs Attention](needs-attention-plan.md) — _In progress._ The
  dashboard says *why* an agent stopped instead of one flat "waiting for
  input": Question, Completed, or an unexplained Waiting, read from the
  harnesses' lifecycle hooks and layered in memory over the persisted
  status. Rows, header count, the `i` list, and the session sidebar all
  read from one ordered query, questions first. Built and shipping; what
  remains is the live four-harness walkthrough and two fidelity gaps —
  Pi reports no signal at all, and Codex cannot separate a question from
  a completion.
- [Bug backlog](bug-backlog-plan.md) — _Backlog._ Running list of known
  bugs not yet scheduled for a fix, one section per bug. Currently
  tracks recently fixed dashboard, sidebar, and composer-pane regressions.
- [Markdown viewer completeness](markdown-viewer-completeness-plan.md) —
  _Backlog._ Fix the Markdown viewer's missing table headers first, then
  tighten coverage for table alignment, uneven rows, prefixed tables,
  inline styling inside table cells, long cells, footnotes, and math.
  Links and image fallbacks remain.
- [Prompt library](prompt-library-plan.md) — _In progress._ Save
  reusable prompts and inject them into a session (composer (removed 2026-10-11, DESIGN.md §18) when on,
  paste without sending when off). Phases 1–2 have shipped: SQLite-backed
  save & inject, a multi-source picker (`User` / `Project` / `Global` /
  `Worktree` with badges), export to `config.json`, fill-in
  `{{placeholders}}`, and select-menu slots authored inline as
  `{{name|a|b|c}}`. Tags/grouping, an `aiw-add-prompt` skill, export/display
  location unification, and showing the on-disk destination path remain.
- [Startup performance](startup-performance-plan.md) — _In progress._
  Parser repair and sidebar concurrency fixes shipped in PR #308;
  indexed Codex transcript access and sequenced startup I/O are
  implemented, with provider consolidation and deeper instrumentation
  still planned.
- [Per-session agent usage](per-session-usage-plan.md) — _Partial._ Split
  usage scopes so feature rows show aggregate feature usage while
  dashboard harness rows and sidebars show usage for the selected agent
  session. Exact provider-session binding and safer inference fallback
  have shipped for Claude, Codex, and opencode; feature-row aggregation
  remains. Pi remains unsupported until it exposes usable per-session
  usage metadata.
- [Token-efficient agent sessions](token-efficiency-plan.md) — _Partial._
  Move AIW from cumulative usage accounting to active usage management:
  provider/model-aware costs, Economy/Balanced/Deep session profiles,
  context-pressure warnings and soft budgets, deliberate
  resume/compact/fresh rotation, bounded cross-provider handoffs,
  token-aware utility inference and caching, removal of the per-edit
  Review Mode note tax, prompt/instruction/output hygiene, and a
  capability recheck/report for fast-moving model, effort, context,
  pricing, CLI, and usage-schema changes. Sequenced from correctness and
  zero-token quick wins through context lifecycle and local efficiency
  analytics.
- [Vim mode](vim-mode-plan.md) — _Partial._ Ranked checklist of vim
  features for the in-house editor (`src/editor.rs`). Tier 1 core editing
  largely shipped; change operators and Tiers 2-3 remain.
- [Remote Control — companion app](remote-control-companion-app-plan.md) —
  _Dropped (no-cloud build, DESIGN.md §6; the last `--remote-control` launch flag went 2026-10-11)._ Epic 1 (on-demand tokio/axum server skeleton, toggled
  from the dashboard leader menu) has shipped. Monitor and, eventually,
  fully control AIW agent sessions
  (any harness, any feature) from a phone: read-only status/notifications
  first, then answering blocked-agent prompts, then full terminal
  control — over LAN and/or an existing tunnel tool. Client is a Flutter
  native app (iOS + Android) rather than a PWA, paired to AIW via a
  QR-code exchange for a per-device token. A new remote-control
  server (tokio/axum) runs on-demand on its own thread, marshalling all
  App-state access through the existing main-loop channel pattern. Ten
  epics across four priority tiers, with server skeleton, device
  storage, and native-app groundwork startable in parallel from day one.
  Distinct from the shipped per-Claude-session Remote Control below,
  which mirrors to claude.ai/mobile via Anthropic's own infrastructure.
- [Remote Control — server mode](remote-control-server-mode-plan.md) —
  _Dropped (no-cloud build)._ Spawn new Claude sessions on demand from web/mobile, each
  in its own worktree. Deferred; AIW's role would shift from
  drive-the-agent to provision-and-review.
- [Remote Control — QR code overlay](remote-control-qr-overlay-plan.md) —
  _Dropped (no-cloud build)._ Render the Remote Control session URL as a scannable QR in
  a TUI overlay. Pays off most with server mode, which prints a stable
  URL.
- [Expanded keybindings](expanded-keybindings-plan.md) — _Backlog._
  Grow config wizard keybinding support beyond dashboard actions to
  leader commands and other scoped command surfaces.
- Final Review and its enhancements — _Removed 2026-10-11._ The review
  viewer, walkthroughs, co-review, review questions and PR submission
  duplicated `/code-review` and `/review`; the plan was deleted with the code.
- PR Triage and its Investigate follow-ups — _Removed 2026-10-11._ The
  comment reader, fix-prompt injection, AI reply drafts, read-only
  investigations and companion triage features duplicated what the agents
  do in their own session with `gh`; the plans were deleted with the code.
- [Screenshot & video review harness](screenshot-review-plan.md) —
  _All epics shipped._ Lets an agent run AIW in an isolated scratch
  instance (private `XDG_CONFIG_HOME`/`XDG_STATE_HOME` + tmux session, no
  Rust changes) and, only when asked, return screenshots proving a
  feature works. A Python + Pillow ANSI→PNG renderer, a tmux driver
  script, scenario/seed examples, and a screenshot skill; GIF on request,
  high-fidelity `vhs` path left as future work.
