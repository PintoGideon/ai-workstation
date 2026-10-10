---
name: aiw-screenshot
description: Capture PNG screenshots or an optional GIF of AIW's TUI in an isolated throwaway instance and return viewable visual proof. Use only when the user explicitly asks Codex to show, capture, or prove an AIW UI behavior with screenshots, images, a GIF, or visual evidence; do not invoke automatically after routine UI changes.
---

# Capture AIW screenshots

Use the repository's screenshot harness to exercise the changed UI without touching the user's real AIW database or tmux sessions.

## Preserve isolation

Run `scripts/dev/screenshot/aiw-capture.sh`. It creates private `XDG_CONFIG_HOME` and `XDG_STATE_HOME` directories under `${AIW_SHOT_DIR:-/tmp/aiw-shots}`, launches a dedicated fixed-size tmux session, and removes the scratch instance afterward. It also detects and removes any tmux sessions spawned by the scenario.

Keep these constraints:

- Leave `HOME` unchanged so agent authentication and git identity remain available.
- Use the default `120x40` geometry unless the UI needs another size.
- Use `--keep` only while debugging; use an explicit persistent `--out-dir` for deliverables.
- Treat GitHub reads from PR Triage as real read-only operations. Do not confirm actions that reply, resolve, or otherwise write to GitHub unless the user explicitly requested that write.

## Author the scenario

Inspect the UI change and read these examples before authoring a scenario:

- `scripts/dev/screenshot/scenarios/dashboard-tour.txt`
- `scripts/dev/screenshot/scenarios/create-project-flow.txt`

Write one step per line. Separate multiple steps on a line with `|`:

- `key:<name>` sends a tmux key such as `Enter`, `j`, or `Escape`.
- `text:<literal>` types literal text.
- `wait:<ms>` waits for a redraw or asynchronous operation.
- `note:<text>` explains what the immediately following `shot:` proves; use a
  complete reviewer-facing sentence. This is documentation only — it is never
  checked against the pane, so on its own it does not catch a scenario
  claiming one thing and showing another.
- `expect:<text>` requires this literal substring to appear in the next
  `shot:`'s captured pane (checked against the escape-free `.txt`). Stack
  several before one `shot:` to require all of them. `expect_not:<text>` is
  the inverse: the substring must be absent. Either kind fails the whole run
  immediately if violated — no further steps run — and prints the offending
  shot plus the actual captured pane to stderr.
- `shot:<label>` captures numbered `.ansi` and plain-text `.txt` files, then
  checks that shot's pending `expect:`/`expect_not:` assertions before
  continuing.

Add a shot at each frame that materially demonstrates the requested behavior.
Put a `note:` immediately before every published shot—state the visible state
and why it proves the flow—**and pair it with at least one
`expect:`/`expect_not:`** for any shot whose point is proving a specific
title, label, or state. A scenario ran once with no assertions and captured
18 "successful" shots that were all, in fact, the syntax-parser picker: it
assumed a seeded project/feature that the publish invocation never actually
provided, so the intended key sequence fell through to unrelated
dashboard-level keybindings instead of driving the flow the scenario
narrated, and nothing caught it before the wrong screenshots were published
to a PR. `expect:`/`expect_not:` on the shots that mattered is what turns
that into a loud CI failure instead of a silently-wrong gallery. A scenario
ending with a staged `expect:`/`expect_not:` that no later `shot:` consumes
is itself a hard error. (A purely navigational shot with no specific claim
can skip it.) Prefer a scratch scenario file unless it is broadly
reusable.

When the scenario needs existing data, pass `--seed` and optionally `--seed-feature`. Use the payload shapes in `scripts/dev/screenshot/scenarios/seed-project.json`, `seed-feature.json`, and `docs/automation/`.

For a screenshot of an already-completed AI Review, use
`scenarios/ai-review-completed-fixture.txt`. It uses AIW's deterministic
`seed-ai-review` fixture; CI must never start a live `A` review or depend on a
logged-in Claude/Codex harness for visual proof.

## Capture and render

Run the scenario with a persistent output directory:

```bash
scripts/dev/screenshot/aiw-capture.sh \
  --scenario <scenario-file> \
  --out-dir <output-directory>
```

For normal screenshot requests, render each captured ANSI file:

```bash
python3 scripts/dev/screenshot/render_ansi.py <capture>.ansi \
  --out <capture>.png --cols 120 --rows 40
```

Match `--cols` and `--rows` to the capture geometry. Pass `--gif` to the driver only when the user asks for a GIF or animation; the driver then renders the frames and assembles the GIF itself.

## Verify and deliver

If the scenario's `expect:`/`expect_not:` assertions are in place, a clean
`aiw-capture.sh` exit already means every one of them held — a nonzero exit
means the run is broken and must not be rendered or published; go fix the
scenario or the feature and re-run instead. Assertions only cover what was
thought to check, so still search the escape-free `.txt` twins for anything
else the frame should show (a dialog title, entered text, a status line). Do
not read `.ansi` files into context. Then use the local image viewer on one
or two representative PNGs to check layout, clipping, and color. A scenario
with no `expect:`/`expect_not:` at all cannot be trusted on its `note:` text
alone — verify its `.txt` twins by hand before treating it as evidence.

This fork has no PR screenshot publisher. Link every local PNG or GIF in
story order and give each one a short caption describing what it proves.

Do not claim the UI is proven when the text assertions or representative visual inspection fail. Fix the scenario or implementation and rerun the capture first.
