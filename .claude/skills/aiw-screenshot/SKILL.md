---
name: aiw:screenshot
description: >
  Capture screenshots (PNG) or a GIF of AIW's own TUI running in an
  isolated, throwaway instance, as visual proof a feature/UI change
  works. A terminal often will not render
  PNGs/GIFs inline, so raw files alone are not a usable deliverable. Use only when the user explicitly
  asks for visual proof ("show me a screenshot of X", "prove the
  dashboard renders Y") — not automatically after every UI change.
allowed-tools: Bash(scripts/dev/screenshot/*) Bash(python3 *) Bash(mkdir *) Bash(cat *) Bash(ls *) Write Read Skill Artifact
---

## When to use

Only when the user explicitly asks for visual proof that an AIW
feature or UI change works. Do not run this unprompted after routine
UI edits — it's for on-demand review, not a build step.

## Isolation guarantees

`scripts/dev/screenshot/aiw-capture.sh` launches a **throwaway
scratch AIW instance**, never the user's real one:

- Its own `XDG_CONFIG_HOME` / `XDG_STATE_HOME` under
  `${AIW_SHOT_DIR:-/tmp/aiw-shots}/<timestamp>/` — the real
  `~/.config/ai_workstation/aiw.db` and any real running `aiw` session are never
  touched.
- A dedicated tmux session (`aiw-shot-<timestamp>`), separate from any
  `aiw-*` session the user already has.
- `gh` still authenticates as the real user (`GH_CONFIG_DIR` is pinned
  to the real config before `XDG_CONFIG_HOME` is overridden) — needed
  for any scenario that opens PR Triage or the PR picker. Same idea as
  leaving `HOME` untouched for `claude` auth / git identity.
- Fixed geometry, `120x40` by default (`--geometry WxH` to override) —
  reproducible pane layout across runs.
- Teardown kills the scratch tmux session **and any other tmux session
  AIW itself spawned during the run** (e.g. starting a feature creates
  its own top-level `aiw-<project>-<feature>` session, outside
  `aiw-shot-*`) — found by diffing tmux's session list against a
  pre-run snapshot, not by name pattern, so it catches whatever the
  scenario/seed named things. The scratch root is also deleted on
  exit. Pass `--keep` to preserve both the scratch root and any
  spawned sessions for debugging.

**Not sandboxed:** reading PR comments (opening PR Triage / the PR
picker) is a real, read-only `gh` call against GitHub — safe by
default. But if a scenario also *confirms* a fix-target pick, posts a
reply, or resolves a thread, that's a real write against the real
repo — teardown cleans up the tmux session either way (see above), but
it can't undo a posted GitHub comment or a resolved thread. Drive up
to the interesting frame, `shot:`, then `key:Escape` out rather than
confirming, unless the scenario is deliberately meant to exercise a
write.

## Step 1: author a scenario for the feature you just built

Look at what you actually changed, then write a scenario file using
the driver's grammar — don't reuse a generic scenario blindly. Read
`scripts/dev/screenshot/scenarios/dashboard-tour.txt` and
`create-project-flow.txt` for the grammar in practice before writing
one.

Grammar (one step per line, `|`-separated, blank lines and `#`
comments ignored):

- `key:<name>` — `tmux send-keys` key name (`key:Enter`, `key:j`,
  `key:Escape`, `key:?`)
- `text:<literal>` — literal typed text (`text:my-feature-name`)
- `wait:<ms>` — sleep, use after keys that trigger a redraw or async
  work (harness checks, status sync)
- `note:<text>` — a complete sentence explaining what the immediately following
  `shot:` proves to a reviewer. This is documentation only — it is never
  checked against the actual pane, so it does not catch the scenario
  claiming one thing and showing another.
- `expect:<text>` — a literal substring that MUST appear in the next
  `shot:`'s captured pane. Stack several before one `shot:` to require all
  of them. `expect_not:<text>` is the inverse (must be absent). Either kind
  fails the whole run immediately — before any further steps — if violated,
  printing the offending shot and the actual captured pane.
- `shot:<label>` — capture the pane now, written as
  `NNN-<label>.ansi`, then check that shot's pending `expect:`/`expect_not:`
  assertions before continuing.

Put a `shot:` at every point worth showing (before the change, mid
interaction, after the change lands) rather than just first/last. Put a
reviewer-facing `note:` immediately before every shot so the gallery index
explains the visible state and why it matters — **and pair it with at least
one `expect:`/`expect_not:`** for any shot whose whole point is proving a
specific title, label, or state, so that claim is actually machine-checked
rather than trusted on sight. A shot deliberately checked into
`scripts/dev/screenshot/scenarios/` with no `expect:`/`expect_not:` at all is
a scenario nobody has hardened against silently drifting from what it
claims to show — treat a missing assertion on a "proof" shot as a gap to
fix, not a style choice. (A purely navigational shot with no specific claim
— "press Escape, capture the resulting dashboard" — doesn't need one.)

This is not optional ceremony: a scenario that ran without these once
captured 18 "successful" shots that were all, in fact, the syntax-parser
picker — because it assumed a seeded project/feature that the actual
capture invocation never provided, so `key:j`/`key:Q`/typed brief text fell
through to unrelated dashboard keybindings instead of driving the flow the
scenario narrated. Nothing in the pipeline caught it before it was
handed over as proof. `expect:`/`expect_not:` on the shots that mattered would
have failed that run loudly before anything was delivered. Author
every new "proof" shot as if this could happen again, because it already
did.

Author the file under `scripts/dev/screenshot/scenarios/` if it's
worth keeping as a reusable example, otherwise a scratch path (e.g.
your scratchpad dir) is fine for a one-off.

## Step 2: run the driver

```bash
scripts/dev/screenshot/aiw-capture.sh --scenario <your-scenario> --out-dir <dir>
```

Relevant flags:

- `--seed <automation-json>` — pre-populate demo state (a project
  and/or feature) via the automation IPC before the scenario runs, if
  the feature you're proving needs existing projects/features to be
  visible. See `scripts/dev/screenshot/scenarios/seed-project.json`
  and `seed-feature.json` for the payload shape (`docs/automation/`
  has the full schema); action is inferred from the payload's keys
  (`path` → create-project, `branch` → create-feature).
- `--gif` — only pass this if the user asked for a GIF/video, not for
  a plain "show me a screenshot" request. Screenshots (PNG) are the
  default deliverable.
- `--keep` — preserve the scratch root instead of deleting it on exit
  (useful while iterating on a scenario).

For a screenshot of an already-completed AI Review, use
`scenarios/ai-review-completed-fixture.txt`. It uses AIW's deterministic
`seed-ai-review` fixture; CI must never start a live `A` review or depend on a
logged-in Claude/Codex harness for visual proof.

This produces, per `shot:` step, a numbered `.ansi` dump and a
plain-text `.txt` twin (same capture, no escape codes) in `--out-dir`
(default `<scratch-root>/shots`).

## Step 3: render PNGs

**`aiw-capture.sh` does not render PNGs unless `--gif` is passed** —
without `--gif` you get raw `.ansi` dumps only, and must render each
one yourself:

```bash
python3 scripts/dev/screenshot/render_ansi.py <dump>.ansi --out <dump>.png --cols 120 --rows 40
```

Pass `--cols`/`--rows` matching the `--geometry` used for the capture
(120x40 by default) so every frame renders at the same size. Repeat
per `.ansi` file. If `--gif` was passed instead, the driver already
renders and assembles the GIF — nothing further to do.

## Step 4: verify cheaply, then return the result

If the scenario's `expect:`/`expect_not:` assertions are in place (Step 1),
`aiw-capture.sh` has already refused to produce a run where a shot doesn't
show what it claims — a nonzero exit here means the run is broken, full
stop; do not render or hand it over, go fix the scenario or the feature and
re-run. A clean exit means every assertion held, but assertions only cover
what you thought to check, so still spot-check: grep the `.txt` twins for
anything you didn't assert on but expect to be true, and Read **one or two
representative PNGs** as images to confirm layout/colors look right — not
every frame. Never read the `.ansi` files, whose escape codes waste tokens.

If you are looking at a scenario that predates `expect:`/`expect_not:` and
has none, do not trust it on the strength of its `note:` lines alone —
`note:` is unverified narration. Either add assertions to it before reusing
it for a "proof" shot, or verify its `.txt` twins by hand exactly as
described above before treating the capture as evidence of anything.

## Step 5: deliver

This fork has no PR screenshot publisher (the upstream Cloudflare Pages
gallery and its capture workflow are gone). Link every local PNG or GIF in
story order with a short caption saying what it proves; the `Artifact` tool
may be used for an in-conversation preview. Never paste raw `.ansi`/`.txt`
captures as the deliverable.
