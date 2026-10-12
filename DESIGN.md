# AI Workstation — design

**AI Workstation (AIW)** is our own fork of the upstream project credited in [README.md](README.md)
(MIT © Ryan Eldridge, see [LICENSE](LICENSE)). It keeps upstream's terminal dashboard and tmux-based engine and adds
what our own design work asked for (`~/ai_harness_ui/SPEC.md`, `DESIGN.md`, `TUI_DESIGN.md`). Maintenance and
cherry-picking from upstream: [FORK.md](FORK.md). What the app does today, in plain English:
[docs/PRODUCT.md](docs/PRODUCT.md). "Upstream" below means that original project.

Status legend: ✅ done · 🔨 in progress · ⏳ planned.

## 1. Principles

1. **Pure harness and tooling.** AIW itself makes **no network calls**. Agents talk to their own providers; `git` and
   `gh` run only when you ask for something that needs them. Nothing polls in the background.
2. **Never edit global agent config.** Everything an agent needs is passed per session (flags, env vars, files inside
   the worktree that git ignores).
3. **See many agents at once.** Switching one session at a time does not scale to 10+ agents.
4. **tmux underneath, terminal on top.** Agents live in tmux and survive AIW closing. Fonts belong to your terminal
   (Ghostty, iTerm, WezTerm); AIW owns colours, icons and layout.
5. **Add, don't rewrite.** New code goes in new files; small marked hooks into code inherited from upstream
   (`// FORK:`), so cherry-picking upstream commits stays easy.

## 2. Kept from upstream (no work needed)

Projects → Features (branch/worktree) → Sessions · Claude, Codex, Pi · permission modes (Vibeless / Vibe /
SuperVibe) · needs-you list `i` · dormant `z` · resource guards ·
prompt library · TODO lists · usage/cost panels · themes · config wizard ·
`doctor` / `automation` commands.

## 3. Gaps and what we add

| # | Gap upstream | What AIW adds | Priority | Status |
|---|---|---|---|---|
| G1 | One agent on screen at a time | **Split view**: 2–4 live agents tiled inside the TUI, plus `aiw grid` for an external terminal | P1 | ✅ grid in the cockpit pane (`v`) + `aiw grid`; the in-TUI captured split was replaced by the cockpit (§18) |
| G2 | No Gemini, no Cursor | **Gemini CLI** and **Cursor agent** as first-class agents | P1 | ✅ (untested live — CLIs not installed) |
| G3 | App makes its own network calls | **No-cloud build**: remove self-upgrade, phone remote control, GitHub screenshot fetching, background PR polling | P1 | ✅ (`NO_CLOUD.md`, `scripts/check-no-network.sh`) |
| G4 | Edits `~/.codex/config.toml` (and, before OpenCode was removed, `~/.config/opencode`) | per-session config only | P1 | ✅ |
| G5 | Worktrees inside the repo; no `.env` copy; port clashes; `~` not expanded; confusing branch error | work-safety fixes | P1 | ✅ |
| G6 | Basic theme list; fonts left entirely to the terminal | **Theming & fonts polish** | P1 | ✅ |
| G7 | Flat list for many projects | project tags/colours, summary counts, `0`–`9` scope | P2 | ⏳ |
| G8 | No tickets | board, start N agents on a ticket, compare, pick → PR → Linear (on demand only) | P3 | ⏳ |
| G9 | A session whose window closed on its own (`exit`, agent quit) stayed listed as running; opening it errored | closed terminals disappear; closed agents show as stopped (■) and reopen with their conversation; an agent that exits while its shell survives shows ■ "exited" (no "finished" attention) and `c`/`S` relaunch it in the same window; the single view returns to the dashboard when its window is gone | P1 | ✅ |
| G10 | Typing in an agent opened a composer | keys go **straight to the agent**; the optional composer was removed on 2026-10-11 (§18) (setting, or `Ctrl+Space e` per session) | P1 | ✅ |
| G11 | Dated look | **UI revamp** within a strict performance budget (§12) | P2 | ✅ dashboard, footer, help; project colour tags ⏳ (with G7) |
| G12 | OpenCode support we don't use | **OpenCode removed** (code, plugins, themes, GUI, docs); old data still loads — see §14 | P1 | ✅ |

---

## 4. G1 — Split view (no more switching)

### 4.1 Inside the TUI

> **History (2026-10-11).** The in-app split view (captured tiles, `Ctrl+Space` leader, pins) was removed with
> the in-app view; `v` now runs `aiw grid` (§4.2) in the cockpit's agent pane (§18).

From the dashboard, `v` on a feature opens **Split view** with that feature's agent sessions tiled. `v` on a project
tiles its features' first agents. You can also **pin** any session from any project (`P` on a session row) and `V`
opens the pinned grid.

```
┌ RS › HX-12 › claude ─ ⛔ waiting 0:40 ─────────────┬ RS › HX-12 › codex ─ ● working 6m ───────────────┐
│ (live Claude Code)                                 │ (live Codex)                                     │
│                                                    │                                                  │
│ Do you want to proceed?  ❯ 1. Yes  2. No           │ • Editing src/auth/sso.rs                        │
├ BA › BA-7 › claude ─ ✓ finished 3m ───────────────┼ WF › WF-31 › gemini ─ ● working 2m ──────────────┤
│ (live Claude Code)                                 │ (live Gemini)                                    │
│                                                    │                                                  │
└ Ctrl+Space ←↑→↓ focus · Ctrl+Space z zoom tile · Ctrl+Space v leave split view · 2 more need you: Ctrl+Space i ┘
```

- **Layouts:** 1 (zoomed), 2 side by side, 3 (one big + two stacked), 4 (2×2). More than 4 sessions → pages
  (`Ctrl+Space [` / `]`).
- **Focus:** one tile has focus (bright border); keystrokes go only to it. `Ctrl+Space` + arrows moves focus;
  clicking a tile focuses it.
- **Attention:** a tile whose agent needs you gets an amber border; `Ctrl+Space i` focuses the oldest one, opening it
  in a tile if it isn't shown.
- **Zoom:** `Ctrl+Space z` toggles the focused tile full-screen (today's single view) and back.
- **Header per tile:** project tag › feature › agent · status · time in state.

**Implementation notes.** Upstream already rendered an embedded session from tmux screen captures (`src/ui/pane.rs`,
`src/app/view.rs`, `src/tmux_observer.rs`). Split view generalises "one live session" to "N live sessions": one
capture/refresh worker per visible tile, a tiled renderer, input routed to the focused tile, and each tile's tmux
window resized to its tile. Resizing a window that is also open elsewhere (another tile, `aiw grid`) follows tmux's
`window-size latest` rule.

### 4.2 External grid (quick win)

`aiw grid <feature|project>` opens the same agents as real tmux panes in the current terminal window, at native
terminal speed, using grouped tmux sessions on AIW's private socket (tested 2026-10-09: views clean themselves up when
closed and never move AIW's own windows). Useful for a second monitor.

## 5. G2 — Gemini and Cursor

| | Gemini CLI | Cursor agent |
|---|---|---|
| Binary | `gemini` | `agent` (also `cursor-agent`) |
| Ask / auto-edit / full auto | `--approval-mode default` / `auto_edit` / `yolo` | default / default / `--force` |
| Status hooks (written inside the worktree, git-excluded) | `.gemini/settings.json` → `SessionStart`, `BeforeAgent`, `AfterAgent`, `BeforeTool`, `AfterTool`, `Notification`, `SessionEnd` | `.cursor/hooks.json` → `sessionStart`, `beforeSubmitPrompt`, `preToolUse`, `postToolUse`, `stop`, `sessionEnd` |
| Maps to AIW states | `BeforeAgent`→working, `AfterAgent`→completed, `Notification`→question | `beforeSubmitPrompt`→working, `stop`→completed; question via screen fallback |
| Resume | `gemini -r <id>` | `agent --resume <chatId>` |
| Instructions file | reads `GEMINI.md`; we set `context.fileName = ["AGENTS.md","GEMINI.md","CLAUDE.md"]` | reads `AGENTS.md`, `CLAUDE.md`, `.cursor/rules` |

Neither CLI is installed on the dev machine; both are built against their docs with tests, then verified live.

## 6. G3 — No-cloud build

AIW must be pure harness and tooling. Inventory of upstream's own network use and the decision for each:

| Upstream feature | Network use | AIW decision |
|---|---|---|
| self-upgrade command | GitHub releases API | **remove** (update by pulling our fork) |
| Remote Control (phone app) | local web server, Tailscale, Web Push to push services | **remove** |
| PR screenshot viewer | downloads images / CI artifacts from GitHub | **remove** |
| Background PR badge sync | `gh` every 5 minutes for active PRs | **off**; refresh only when you press refresh |
| Optional syntax-parser install | `git clone` of tree-sitter grammars | keep as an explicit, user-started action, clearly labelled |
| Issue browser, PR badges, issue comments | `gh` | keep — only when you start it |
| Agents (Claude, Codex, …) | their own providers | outside AIW — the agents' business |

Guarantee: no HTTP client or server library is linked into the binary (`ureq`, `axum`, `tokio` net, push/tailscale
code removed), checked by `scripts/check-no-network.sh` (inspects `cargo tree` and greps the sources).

## 7. G4/G5 — Work-safety fixes

1. No writes to `~/.codex/config.toml` (pass `notify` per launch).
2. Worktrees outside the repo: `~/.local/share/ai_workstation/worktrees/<project>/<feature>`
   (macOS `~/Library/Application Support/...`); existing `.worktrees/` features keep working.
3. Copy files a fresh worktree needs: gitignored files matching `.worktreeinclude` (default `.env*`); never symlink
   `node_modules`.
4. A block of 10 ports per feature (`AIW_PORT`), reserved under a lock.
5. Files AIW writes into a repo (`.claude/skills/aiw-*`, `.codex/…`, `.gemini/…`, `.cursor/hooks.json`) are added to
   `.git/info/exclude` so they can never be committed.
6. Repo hook scripts (`aiw.json` lifecycle hooks) run only after a one-time approval pinned to their content hash.
7. `~` expanded in the add-project path.
8. Clear message when a branch is already checked out (current git wording), new-feature branch never defaults to the
   current branch, no empty `.worktrees/` left after a failure.
9. Feature names are checked against `git check-ref-format --branch` rules in the dialog (no spaces, `..`, leading
   `-`, `~^:?*[\`, trailing `/` or `.lock`, `@{`, control characters; max 120 chars); tmux session and worktree
   slugs are capped at 48 chars per component.
10. One TUI per config dir (`aiw.lock`, advisory `flock`): a second `aiw` prints who holds it and exits 1;
    `doctor`/`grid`/`fonts`/`automation`/`notify` are unaffected.
11. Loud failures: an invalid `aiw.json` or unreadable `.worktreeinclude` is a toast plus a log line once per
    project load; a lifecycle hook exiting non-zero toasts `Hook on_start failed (exit N) in <feature>`; a worktree
    hook still running after 60 s can be skipped with `Esc`; a project whose folder vanished is flagged
    (`App::missing_project_repos`) with a one-time toast; unparsable notification files are quarantined under
    `<state>/notifications/bad/` (50 kept).
12. `Esc` on the dashboard is a no-op (`q` quits; `confirm_quit` makes `q` ask first while agents run); `Ctrl+Q`
    leaves the view from the composer too; per-session compose toggles persist in the store.
13. Thinking markers live in `<state>/thinking` (the old `/tmp/aiw-thinking` is read for one release); the Codex
    sessions root is derived from `$HOME` only.

## 8. G6 — Theming and fonts

**Themes**
- Add our palettes as built-ins: **Ember** (warm dark, the new default), **Graphite**, **Midnight**, **Paper** (warm
  light), **Porcelain** — every text/background pair checked for readable contrast in tests.
- **Truecolor with fallback:** detect `COLORTERM=truecolor`; otherwise map every theme colour to the nearest of the 256
  terminal colours so themes still look right in basic terminals.
- **Live preview** in the theme picker (`T`): the dashboard restyles as you move through the list; `Esc` reverts.
- **Follow light/dark:** optional pair of themes chosen from the terminal background (`COLORFGBG` / OSC 11 query).
- Status colours are consistent everywhere: working = blue, needs you = amber, finished = green, failed = red,
  idle = grey.

**Fonts** (the terminal draws the text, so AIW makes the terminal's font work well)
- **Icon check on first run:** shows a sample icon and asks "do you see a symbol here?"; sets `nerd_font` on/off
  instead of showing broken boxes.
- **`aiw fonts`:** reports the detected terminal, whether a Nerd Font and truecolor are available, and prints a ready
  config snippet for Ghostty / iTerm2 / WezTerm / Kitty / Alacritty with recommended fonts (JetBrains Mono, Geist Mono,
  Iosevka, Fira Code + Symbols Nerd Font). No downloads — instructions only.
- **Glyph width safety:** icons used in tight columns are single-width, with ASCII fallbacks.

## 9. G7 — Many projects (from `TUI_DESIGN.md`)

Project tag + colour on every row · collapsed rows show `⚠2 ●6 ✓1 ○1` · `0` all / `1`–`9` one project for tree, `i`,
search and split view · header totals (projects · agents · need you · working · to review).

## 10. G8 — Tickets (later)

Task above features (N attempts, one agent each, same base) · board `b` · compare `c` · pick ✓ → push → `gh pr create`
with `Fixes <KEY>` · Linear by personal API key, **only on explicit actions, never polled**.

## 11. Naming

**Full rename done (2026-10-10).** Product **AI Workstation**, one command **`aiw`** (the second upstream binary is
gone), crate `ai-workstation` / lib `ai_workstation`, GUI `aiw-gui` (Tauri product "AI Workstation"), env vars
`AIW_*`, tmux sessions `aiw-*` on AIW's own socket, config/data in `~/.config/ai_workstation` (database `aiw.db`,
hook scripts in `~/.config/ai_workstation/hooks`), state in `~/.local/state/ai_workstation` (`/tmp/ai_workstation`
on macOS), repo files `aiw.json` / `.aiw/`, injected skills `aiw-*`, theme `classic`.
We no longer merge upstream; useful commits are cherry-picked by hand ([FORK.md](FORK.md)).

**Data migrated by copy.** On the first start with no `~/.config/ai_workstation`, an existing upstream setup
(`~/.config/amf`) is *copied* in (`src/fork/migrate.rs`): settings, ports, themes, parsers, and the
database (copied while unchanged, WAL folded into the copy, integrity-checked). Upstream's folders, hooks and tmux
socket are never touched, so it keeps running side by side. A one-time toast reports the copy.

**Legacy names still accepted** (read only, never written): repo config `amf.json` → `.amf/config.json` when a repo
has no `aiw.json`; theme `amf`; diff viewer `amf`; plan file `AMF_PLAN.md`; upstream hook paths (`~/.amf/hooks`,
`<config root>/amf/…`) and `amf-codex-notify.sh` are recognised and replaced; `amf-*` tmux session names of migrated
features; git-exclude patterns keep hiding upstream's helper files. Migrated features keep their stored `amf-*`
session names; they show as stopped (their tmux sessions live on upstream's socket) and are restarted or resumed
from AIW under the same name on AIW's socket.

## 12. G11 — UI revamp (performance first)

Stay on **ratatui** (AIW's Rust TUI library). It redraws only the cells that changed and renders a full frame in well
under a millisecond, so the look can be modernised without costing speed. Other stacks (Go Bubble Tea, Python
Textual, Ink/React, OpenTUI) would mean a rewrite and, for Textual/Ink, a slower runtime.

**Look:** rounded borders and padding; a clear type hierarchy (bold titles, dim metadata); header with status pills
(`⚠ 2 · ● 6 · ✓ 1`); tree guide lines; project colour tags; one accent colour for focus; consistent status colours;
designed empty states; a contextual key-hint footer; a redesigned help overlay.

**Budget (measured with AIW's built-in `perf` timers, before vs after):** dashboard frame render ≤ 2 ms at 200×60;
no extra `tmux` processes per frame; idle CPU unchanged; typing latency in an agent unchanged. Anything that misses
the budget is dropped.

**Where speed actually goes** (to improve in the performance pass): the agent view repaints from `tmux capture-pane`
snapshots (a new process per refresh) — fewer/cheaper captures and keeping tmux calls off the input path matter far
more than drawing.

### 12.1 What shipped (2026-10-10)

```
╭──────────────────────────────────────────────────────────────────────────────────────────────────────╮
│ AI Workstation v0.49.0 | ~/ai_workstation  [1 question, 1 to review | <leader i>]  ⚠ 1 · ● 2 · ✓ 4  ? help │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────╯
╭ Projects ─────────────────────────────────────────────────── 3 projects · 12 features · 30 sessions ╮
│  ▾ harbor-api  ~/code/harbor-api                                                                     │
│   ├─ ⣧ ▾ harbor-12 [vibeless] just now [3]  ~/.local/share/ai_workstation/worktrees/harbor-api/…      │
│   │   ├─ ● * claude                                                                                  │
│   │   │   Editing src/auth/sso.rs — running cargo test                                               │
│   │   └─ ● > terminal                                                                                │
│▎  ├─ ● ▾ harbor-13 [PR #482 · 2 open] [vibe] [review] 7m ago [3]  ~/.local/share/…                   │
│  ▾ docs  ~/code/docs  · no features yet — press n to add one                                         │
╰──────────────────────────────────────────────────────────────────────────────────────────────────────╯
╭──────────────────────────────────────────────────────────────────────────────────────────────────────╮
│ harbor-13 [feature/harbor-13]  ~/.local/share/ai_workstation/worktrees/…        [claude] 5h ███░ 0 msgs │
╰ n feature  Enter expand  c start  x stop  y ready  s switch  S resume  u config  d delete  q quit ──╯
```

- **Panels:** rounded borders in the quiet `border` colour; the list has one column of padding each side. The
  selected row gets an accent bar (`▎`, theme `primary`) in the left padding as well as the selection background.
- **Type hierarchy:** project names bold, feature names regular (bold when selected), all metadata (version, cwd,
  paths, age, summary) in `text_muted`; tree guide lines (`├─ │ └─`) in the `border` colour; `▸`/`▾` disclosure.
  Amber is no longer used for ages and summaries, so it means only "needs you".
- **Header pills** `⚠ needs you · ● working · ✓ finished`: questions + generic waits + unexplained pending inputs;
  running features whose agent is working; finished turns + features marked ready. Counted in one pass over the
  features from state the dashboard already has. They sit right-aligned and give way before the `<leader i>` badge
  on a narrow terminal (the badge is the clickable route; hit-testing measures the same layout).
- **Footer:** the panel is three rows tall, so upstream's second line of key hints never reached the screen. The
  contextual hints (per selection and per mode) now sit in the footer's bottom border; keys in the accent colour,
  labels dim.
- **Empty states:** no projects (centred explanation + `N` / `?` / `q`), a project with no features (inline
  `press n`), a feature with no sessions (`· no sessions`).
- **Help overlay:** rounded panel, bold section titles, keys right-aligned in the accent colour, continuation lines
  dim, `Esc close · j/k scroll` in the bottom border. Same bindings.
- **Split view:** unfocused tiles rounded (focused stays thick); tile status uses the theme's status colours
  (`status_waiting`, `status_working`, `success`, `status_idle`, `status_stopped`) like the dashboard.
- **Not done:** project colour tags — they belong with G7's tags (§9). Dialogs other than help keep their square
  borders (each is its own upstream file; restyling them all would add many `FORK:` touch points).

Code: `src/ui/fork_chrome.rs` (panels, pills, empty state), small `FORK:` hooks in `ui/header.rs`, `ui/list.rs`,
`ui/status.rs`, `ui/dashboard.rs`, `ui/dialogs/help.rs`, `handlers/mouse.rs`.

### 12.2 Measured

`src/ui/fork_bench.rs`: the dashboard rendered into a ratatui `TestBackend` at **200×60**, 3 projects × 4 features ×
10 sessions per project, mixed states (working, question, finished, stopped, PR badge, status lines), 2,000 frames
after warm-up, release build (`cargo +1.99.0 test --release --locked dashboard_frame_bench -- --ignored
--nocapture`). A second, always-on test renders through a `MockTmuxOps` with no expectations, so any tmux call while
drawing fails the suite.

| | mean | p95 | instructions / frame (callgrind) |
|---|---|---|---|
| before (upstream look) | 390–430 µs | 410–460 µs | 7.17 M |
| after (G11) | 360–375 µs | 387–435 µs | 6.76 M (−5.7 %) |
| ratatui 0.30 (same code, 3 runs each side) | 340–367 → 372–373 µs | 357–396 → 398–403 µs | not measured |
| polish pass (§12.4, 3 idle runs each side) | 360–363 → 357–361 µs | 384–401 → 375–384 µs | not measured |
| badge model + workspace layout (§15): tree frame, same machine and day, before → after | 393 → 426 µs | 432 → 457 µs | not measured |
| badge model + workspace layout (§15): workspace frame (`workspace_frame_bench`) | 540 µs | 589 µs | not measured |

The ratatui 0.30 row (2026-10-10, `chore: upgrade to ratatui 0.30 …`): three back-to-back runs on each side of the
upgrade on the same machine; the before runs spread 340–367 µs and the after runs sit at 372 µs, so the difference
is within one run's variance, not a regression. The live-pane bench (§12.3, `view_refresh_bench`) is identical to
the digit before and after (0.7 / 10.2 / 2.7 / 7.5 / 23.7 processes per second) — it counts tmux processes and
frames, which the drawing library does not touch.

Budget ≤ 2 ms: met with 5× headroom. No tmux or process calls per frame (before or after). About 70 % of a frame is
ratatui's own buffer reset/diff (computing each cell's display width); AIW's own row building is under 1 %. The
G11 changes pay for themselves by dropping two redundant whole-screen passes: the full-screen `Clear` in
`dashboard::draw` / split view (`Terminal::draw` already hands over a reset buffer), a `getcwd` per frame (cached),
a `Theme` clone per frame in the list, and a `$HOME` lookup per row (cached).

The polish-pass row (2026-10-10): the `…` fitting of every list row, the footer's hint cutting and the toast
placement cost nothing measurable — both sides sit at 360 µs on the same idle machine.

### 12.3 Performance and obsolete-code pass (2026-10-10)

**Live panes (agent view, split view).** Measured with `src/app/fork_view_bench.rs`: the real snapshot workers
against a real tmux server on the test-isolated socket, 4 s per case, counting tmux processes started (every tmux
call goes through `TmuxManager::command`, which now counts them; with `AIW_PERF=1` the total is logged with the perf
summaries). Busy pane = a spinner-like line rewritten every 100 ms.

| case | before (processes/s) | after (processes/s) | frames/s before → after |
|---|---|---|---|
| single view, idle, pipe-pane (macOS default) | 1.0 | 0.7 | 0.2 → 0.2 |
| single view, busy, pipe-pane | 15.2 | 10.2 | 9.7 → 9.7 |
| single view, idle, control mode (Linux default) | 9.5 | 2.7 (≈1/s once settled) | 4.2 → 1.7 |
| single view, busy, control mode | 13.7 | 7.5 | 6.5 → 6.7 |
| split view, 4 busy tiles, 1 focused | 62.0 | 24.0 | 40 → 22 |

- **Content + cursor in one tmux process** (`TmuxManager::capture_pane_ansi_with_cursor`: `capture-pane … ;
  display-message …`) wherever both are due — every control-mode reseed and most pipe-pane captures.
- **Idle heal backs off** (control mode): the 250 ms self-heal capture doubles to 500 ms, then 1 s, while
  consecutive heals capture an identical screen + cursor (content fingerprint) and the control stream reports
  nothing; any `%output`, layout/mode change or refresh request resets it to 250 ms.
- **Unfocused split tiles** capture at most every 250 ms and poll no cursor (only the focused tile draws one); the
  focused tile keeps the single view's cadence and gets an immediate refresh when it gains focus.
- **Key input path:** keys only send to tmux and flag a burst refresh for the worker thread; captures, cursor polls
  and drift checks never run while handling a key (checked, unchanged).

**Periodic work (5 s sweep, render path).** Profiled by reading; only behaviour-preserving fixes:

- Context-window collection re-read and re-parsed each Claude transcript / Codex rollout **in full** every sweep. It
  now folds only the bytes appended since the last sweep (`src/fork/line_fold.rs`; a shrunk, replaced or rewritten
  file is read again from the start; equivalence tests compare against the whole-file parse at every append). A 10 MB
  transcript: **56 ms → 0.03 ms** per sweep when unchanged (release).
- The sidebar's latest-model lookup parsed the whole Claude transcript whenever it changed: same incremental fold.
- Pi and Codex session discovery read every session file in full to look at its first lines: they now stop at the
  header (`session_meta`). Only difference: a file with invalid UTF-8 *after* its header is no longer skipped.
- The PR picker, compact-memory confirmation and new-project form ran `git rev-parse` and re-read the config files
  on **every frame**: memoised per dialog and path for one second (`src/app/fork_render_memo.rs`).
- A plan-file read on the main thread every 5 s (and on every prompt submit) filled `sidebar_plan_cache`, which
  nothing outside tests read: removed with the cache.
- tmux in the sweep was already one call each (`list-windows -a`, `list-sessions`); no per-session loops found.

**Removed (obsolete).**

- Dev scripts for removed features: `scripts/dev/check-gui-screenshots.py` (drove the removed remote screenshot
  Tauri commands), `scripts/dev/fixtures/screenshot-gh.py` (its fake `gh`), `scripts/dev/screenshot/
  capture-remote-dashboard.cjs` and the `remote-dashboard-collapse` scenario + config (the removed phone PWA).
- Dead code (unused even by tests): `GithubTransport::{resolve_pr, fetch_pr_by_number}`, `AiwDb::{prompt_overrides,
  delete_prompt_overrides_for_workdir, delete_pr_investigation}`, `pr_investigations::delete`,
  `PromptOverrides::{all, remove}`,
  `PlanQuestion::accepts_answer`, two unused prompt-version constants, `PromptContext::{insert, is_empty}`,
  `BatchReviewOutput::is_empty`, the never-constructed `TmuxInputWriter::Pipe` transport and the `Info`/`Error` startup
  validation levels left over from the removed parser re-download, a dead test helper, and `sidebar_plan_cache`
  (above). The plan-preview helpers it used are now `#[cfg(test)]`.
- No OpenCode / remote-control / push / upgrade code remains outside the `FORK:`-marked legacy readers (§11, §14).
  Upstream's planning docs under `docs/backlog/` and `CHANGELOG.md` keep their history.
- Kept on purpose: upstream items marked "exercised only by unit tests" or "ahead of its UI" (removing them would
  delete tests, not dead features), and struct fields that are deserialized but not yet shown.

### 12.4 Spacing and truncation polish (2026-10-10)

Findings from an end-to-end run at 80×24, 120×40 and 200×60 with three projects and some twenty sessions, and
what changed (`src/ui/fork_text.rs` holds the shared text fitting; everything else is a small `FORK:` touch):

- **`…` everywhere a line is cut.** Project, feature and session rows (`fork_text::fit_line`, width-aware so an
  emoji never spills over the border), the footer's selection line (which now also stops short of the usage
  readout), the help body, the theme picker's legend and the hook dialog's command/script preview. Footer and
  split-view hints are dropped a whole `key label` at a time and end in `…` (`fit_hints`), never `V pendi`.
- **Footer padding.** The footer's inside carries the same one column of padding as the list (`Padding`), so
  `Created project 'x'` and the selection line start one cell in like every row above them. A footer squeezed
  to two rows (40×10) draws no usage readout rather than painting it over its own border.
- **Toasts** are placed by the caller: on the dashboard they stack inside the list panel (one cell in from its
  border), elsewhere in the screen corner as before; rounded like the panels. The "stale dialog corner" seen in
  the e2e captures was a toast's bottom border sitting on the footer's message row, not a drawing residue —
  `closing_a_dialog_leaves_no_border_residue_on_the_dashboard` draws four dialogs and then the dashboard into
  one `TestBackend` and finds none.
- **Status glyphs** on a feature row are the header pills' (`⚠` needs you, `✓` finished) and the row's fallback
  rolls up its sessions: `●` while any session runs, `■` stopped, `○` otherwise — a feature no longer shows
  `○` over `●` sessions. The bare ` ?` after the session count is gone (the glyph already says it).
  *Superseded by §15: rows, pills and tiles now all draw the badge model's glyphs (`⛔` needs you).*
- **Narrow rows.** The agent view's right-hand badges have priority: the long direct-input hint shortens to
  `[leader+e: compose]` and whatever header text would run under the badges is elided with `…`. The dashboard
  header's attention badge falls back to `[N | <leader i>]` when the full one would be cut, and to none at all
  below that; the list's totals title gives way before it would overwrite ` Projects `.
- **Dialogs.** The New Project path field scrolls (`…` + the tail, cursor in view) with the browse hint on its
  own row; the hook-approval dialog wraps paths after `/` with a hanging indent (`wrap_path`); the help overlay
  wraps descriptions itself with a hanging indent under a key column that shrinks on a narrow panel.
- **Lists and flows.** Features list in creation order (oldest first); an empty dashboard offers only `N`, `?`
  and `q`; a clean feature delete closes its dialog on its own and toasts (an error still waits for Enter);
  `i` with nothing pending writes "Nothing needs you" to the footer.
- Split view skips any tile rect that is not inside the frame (a stale layout from a resize).

## 13. Delivery order

1. Name (**AI Workstation**, command `aiw`) + this doc ✅ · closed-session bug (G9) ✅ · direct typing (G10) ✅
2. In parallel: G3 no-cloud · G4/G5 safety fixes · G2 Gemini + Cursor · G6 theming & fonts · G1 split view ✅ merged
3. Merge, full test suite, try it
4. UI revamp (G11) within its budget ✅, then a performance and obsolete-code pass ✅ (§12.3, 2026-10-10)
5. Commit to a branch so you can run `/code-review ultra` on it; fix what it finds
6. G7, then G8
7. Productivity features P1–P18: see [docs/PRODUCTIVITY_PLAN.md](docs/PRODUCTIVITY_PLAN.md) (sprint order there)

## 14. OpenCode removal (G12)

AIW supports Claude Code, Codex, Gemini CLI, Cursor agent and Pi. OpenCode was removed entirely: its launch path,
`.opencode/plugins/*.js`, bundled themes, sidebar/usage/context/token readers, session picker, attention
capabilities, the GUI's OpenCode-only edit-review lease, and docs.

**Old data still loads.** An `aiw.db` / `config.json` / `aiw.json` written while OpenCode was supported never fails
to load:

- an **OpenCode session is dropped on load** (there is nothing left that could launch or resume it; a placeholder
  kind would mean a dead branch in every session match). The next save removes its row;
- a **feature whose agent was OpenCode** takes its project's preferred agent; a **project / config preference** of
  OpenCode reads as the default agent (Claude);
- `allowed_agents` and the available-harness list **drop** OpenCode (a list that named only OpenCode means "no
  restriction"); OpenCode **prompt overrides** and cached **token usage** are ignored (left in the table untouched);
- the old `opencode_theme` config key is ignored.

## 15. Workspace layout and badges (2026-10-10)

> **History (2026-10-11).** The in-app layout described here — a captured pane redrawn by AIW beside the sidebar,
> the tree layout, `b`, `Tab`, `o` — was replaced by the cockpit (§18), where tmux draws the agent's real pane.
> The badge model (`src/fork/badges.rs`) and the sidebar rows are unchanged and still described here.

With 10–30 agents the tree is a list of names, and reaching an agent means entering and leaving a view. The
workspace layout keeps the sidebar and shows the selected session live beside it, so moving between agents is
moving the cursor. It was the default layout; `b` toggled to the tree and back (saved under a layout key in
`config.json` — a config that already says `"tree"` keeps the tree). README "Workspace layout" has the key table.

```
╭ Sessions ──────────────────╮╭ RS › hx-12 › claude ─────────────────────────── ⛔ question 0:40 ╮
│ ▾ RS               ⛔1 ●1  ││                                                                  │
│  ▾ ⛔ hx-12           0:40 ││  ⏺ Bash(git push origin hx-12-sso)                               │
│▎   ⛔ claude          0:40 ││  Do you want to proceed?                                         │
│▎     permission: Bash      ││  ❯ 1. Yes   2. Yes, and don't ask again   3. No                  │
│    ●  codex             6m ││                                                                  │
│    ✓  gemini            3m ││                                                                  │
│  ▾ ●  hx-14            12m ││                                                                  │
│    ●  claude           12m ││                                                                  │
│ ▾ BA                   ⛔1 ││                                                                  │
│  ▾ ⛔ ba-7              2m ││                                                                  │
╰────────────────────────────╯╰──────────────────────────────────────────────────────────────────╯
╰ (footer of the time: type here · full view · i next needing you · v tiles · n feature · c start · x stop) ╯
```

**One badge model** (`src/fork/badges.rs`). Every place that says what a session is doing — tree rows, sidebar
rows, split-view tile titles, the needs-you picker, the header pills — derives a `SessionBadge` from state the
app already holds: the attention records (now carrying the harness's notification type and words, so the
reason can be shown without guessing), the per-feature and per-session thinking/tool markers, the stopped and
exited markers, the session's status line. Nothing in the derivation touches tmux or the filesystem.

| kind | glyph | tone | label | since |
|---|---|---|---|---|
| needs you | `⛔` bold | `status_waiting` | `permission: Bash` · `question: …` · `waiting: …` · `input: …` | attention record |
| working | `●` (tree: spinner) | `status_working` | `working 6m · Edit src/auth.rs` (status line, when there is one) | state began |
| finished, unread | `✓` bold | `success` | `done · 3m` | attention record |
| finished, read | `✓` | `text_muted` | `done · 3m` | attention record |
| exited | `■` | `danger` | `exited` | state began |
| idle | `○` | `status_idle` | `idle` (`open` for a terminal) | state began |
| stopped | `■` | `status_stopped` | `stopped` | state began |

Decisions:

- **A feature-level signal lands on the session of its harness.** Attention and thinking are tracked per
  feature; the record's `agent` picks the matching session (Claude's question does not mark the Codex window
  `⛔`), and a per-session IPC marker narrows "working" to that window. A feature's roll-up is its most pressing
  agent session (needs you > working > finished unread > exited > finished read > idle > stopped), plus `ready`
  as a read `✓`.
- **A pending input nothing explained** (`input-request`, a diff review, a change reason) lights every agent
  session of its feature, except a generic session wait while the feature has an attention record: that is the
  same stop seen through the older signal, which the needs-you list already folds into one row.
- **Counts cannot disagree.** The header pills are `badges::total_counts` — every project's `⚠n ●n ✓n` summed
  — and each project row shows its own counts. A feature counts once, under its roll-up (a feature that is
  both working and asking counts as needing you).
- **Unread finished.** `App::finished_seen` (not persisted) holds the sessions whose finished turn was looked
  at: focusing the workspace pane, sending it a key, opening the full view or focusing a split tile. A new
  finished event clears the id, so the next `✓` is bright again.
- **Ages** come from the attention record when there is one, else from `App::badge_ages`, refreshed once per
  main-loop iteration (`refresh_badge_ages`), so the pure derivation never mutates and a frame allocates no
  more than the tree did (one clock per frame, a label string per row).
- **`●` means an agent is working.** A running agent with nothing to report is `○`; a running terminal is
  `○ open`. Before, `●` meant "the window exists".
- **Dormant** has no badge: dormancy needs tmux activity times that the sweep does not cache; the `z` list
  stays the way to find them.

**Layout** (`src/app/fork_workspace.rs`, `src/ui/fork_workspace.rs`, `src/handlers/fork_workspace.rs`):

- The sidebar reuses `App::visible_items`, the selection, the scroll offset and the disclosure state, so every
  dashboard key keeps working; only `b`, `Tab`, `Enter` on a running session and `o` are new. Widths: 34 columns
  from 160, 30 from 120, 26 from 100, 22 (glyph and name only) from 72; below 72 the tree is drawn with a
  one-time footer note. A session row takes a second line, the badge label, while selected or needing you.
- **One capture target.** The pane reuses the single view's snapshot worker: `current_view_snapshot_target`
  answers with the live target while the dashboard is up in this layout, the main loop sizes that window to the
  pane's inner size exactly as it sizes the single view, and the capture-rate rules of §12.3 are unchanged.
  Opening split view (`v`) or a dialog stops the dashboard's capture; leaving them starts it again.
- **Debounced follow.** The selection is what the pane *wants*; it is committed after 150 ms of rest
  (`SELECTION_DEBOUNCE`), so holding `j` never restarts a capture per row. Focusing the pane (`Tab`, `Enter`, a
  click) commits a pending selection at once. Project and feature rows show the feature's first running agent.
- **Focus.** With the pane focused keys go through `forward_tmux_key` with the single view's compose rule
  (`compose_intercept_active`; the optional composer of the time, since removed).
  `Ctrl+Q` returns to the sidebar, never to the tree. `Ctrl+Space` reuses the view's `handle_leader_key`
  against the live session: `z` opens the full view, `q` the sidebar; `t`/`T` move the sidebar to the session
  they land on; overlays that would close into the full view (`i`, `?`) close into the workspace instead.
- The pane is a split-view tile: `fork_split::draw_live_tile` is the one live-tile renderer (focused = thick
  bright border, needs you = `status_waiting` border, badge right-aligned in the title). Everything is clipped
  to `frame.area()`; a resize past the layout's limits falls back to the tree until the next frame fits.
- `b` was chosen because `w` (the obvious key) is context settings and `l`/`d`/`s`/`p`/`t`/`v` are taken;
  `b` reads as "board".

**Measured** (`src/ui/fork_bench.rs::workspace_frame_bench`, release, 200×60, 3 projects × 10 sessions, the
selection alternating between a `⛔` row and a `●` row, a full pane of text, 2,000 frames): **mean 540 µs, p50 536 µs, p95 589 µs, max 872 µs** — under the 2 ms budget with 3× headroom.
The tree frame on the same machine and run: 393 → 426 µs mean, 432 → 457 µs p95 before → after on the same machine (the badge derivation per row and the pills' roll-up cost about 30 µs; still 4× under budget). `workspace_frame_makes_no_tmux_calls` renders the
layout through a `MockTmuxOps` with no expectations, as the tree test does.

### 15.4 Native attach (2026-10-10)

> **History (2026-10-11).** Superseded by the cockpit (§18): the right pane *is* the attach, there is nothing to
> hand the terminal to and take back. `src/fork/attach.rs` is gone.

The in-app pane and the full view relay keys with `send-keys` and redraw from a capture, so a keystroke costs a
process spawn each way plus the capture interval: tens of milliseconds, felt as lag when typing a prompt. For
sustained typing the dashboard hands the terminal over instead (`src/fork/attach.rs`, `src/app/fork_attach.rs`,
`cli::run_pending_attach`): `Enter` on a running session or feature in the workspace layout, `Ctrl+Space Enter`
in a view or the pane, records a `PendingAttach`; the main loop stops the capture worker, leaves raw mode and the
alternate screen exactly as it does for `$EDITOR`, runs `tmux -S <private socket> … attach-session -t
<session>:<window>` with `$TMUX` removed (AIW may itself run inside the user's tmux) and the terminal inherited,
and takes the screen back when the client detaches. One tmux invocation sets `window-size latest` on the window
(the dashboard had pinned it to the pane's size with `resize-window`), a status line (`AIW project › feature ›
session` / `Ctrl+Q: back to AIW`) and, on AIW's own server only, `bind-key -T root C-q detach-client`; a
user-owned server keeps its key tables and the status line says `prefix d`. On return the status line is
restored, `last_resize` is dropped so the pane is resized and captured afresh, and the session's unread `✓` is
cleared. While attached AIW is idle: no capture, no polling, nothing between the terminal and the agent.

## 16. Hook-driven productivity (2026-10-10)

With 10–30 agents the dashboard must answer and steer them without opening each one. The agent CLIs all
offer hooks (docs/development/harness-hooks-reference.md has the exact contracts), and AIW already reached
them the same way: a per-worktree hook file points at an embedded script in `~/.config/ai_workstation/hooks/`,
the script calls `aiw <subcommand>` with the hook's JSON on stdin, and the subcommand writes one JSON line to
the Unix socket `aiw.sock`. Shell scripts never parse JSON; the Rust side does. This section adds the
capabilities that need an *answer* to go back to the agent, and the ones that only need the agent to say more.

**No cloud.** Everything here is local: the scripts, the socket, the status line Claude pipes to AIW, the
files read for Codex and Gemini. No AIW code talks to a provider; nothing new was added. The status line restores the Claude 5-hour / 7-day meters from the
numbers Claude itself hands the command — the OAuth usage call stays removed (NO_CLOUD.md).

### 16.1 Requests that wait for an answer (`src/fork/ipc_reply.rs`)

The socket was fire-and-forget. A hook that needs the user's decision keeps its connection open instead: the
payload carries `"await_reply": true` and a `request_id`; the accept thread parks the stream in a registry
keyed by that id (`try_clone`, no second listener, no reply-socket directory) and hands the message to the
app as usual. The app answers with `ipc_reply::reply(request_id, json)`, one line on that stream, then closes
it. The main loop never blocks: the stream sits in the map until the user decides. When the client gives up
first (its own timeout, or it was killed) the accept thread sends `{"type":"reply_abandoned","request_id":…}`
so the pending UI state is dropped. The client side, `ipc_reply::request(path, payload, timeout)`, returns
`Ok(None)` on timeout — "print nothing, let the harness do its own thing" — and `Err` only when AIW is not
listening at all.

### 16.2 Permission approvals (P6)

Claude `PermissionRequest`, Codex `PermissionRequest` and Cursor `beforeShellExecution` /
`beforeMCPExecution` run `scripts/permission-request.sh` → `aiw permission-request --harness …`, which sends
`{"type":"permission_request", request_id, harness, session_id, tool_name, tool_input, aiw_*}` and waits up to
`permission_wait_seconds` (default 300). The app (`src/app/fork_permissions.rs`) keeps a `PendingPermission`
per request and records an attention `Question` with `notification_type: permission_request` and the reason
`<tool> — <input preview>`, so the existing badge model shows `⛔ permission: Bash — cargo test` everywhere
(Codex and Cursor get the question even though their older signals cannot ask one). Keys: `a` allow, `X`
deny — `y` is "mark ready" on the dashboard, so the obvious pair was taken; the same keys work on a selected
row, in the needs-you picker and in the detail overlay (`Enter` in the picker: tool, full input, `p` = let the
agent ask in its pane, `o` = open it). The reply is `{"behavior":"allow"|"deny"|"pass"}`; the hook prints the
harness's decision JSON (`hookSpecificOutput.decision.behavior` for Claude/Codex, `permission` for Cursor —
`ask` on pass, because Cursor treats an empty answer from a permission hook as invalid and would block).
Allowing clears the `⛔`; a timeout clears it too (the agent shows its own prompt). Gemini's hooks cannot grant,
so its `Notification(ToolPermission)` stays a `⛔` answered in the pane. Every decision is in the debug log.

### 16.3 Context and cost (P4, `src/fork/telemetry.rs`)

Claude: `--settings` (16.6) carries a `statusLine` command, `scripts/status-line.sh` → `aiw status-line`,
which parses Claude's status JSON, sends `status_line` (model, used %, window, cost, lines, rate limits,
and the raw `context_window` object) and prints `Opus 4 · ctx 42% · $0.84` for Claude's footer — or runs
the user's own status line from `~/.claude/settings.json` and prints that instead. The raw `context_window`
is fed to the existing context collector as a *direct* sample (`runtime_payload`), so the tree's `Ctx` label
stops estimating. Codex: the rollout's `token_count` (`last_token_usage` vs `model_context_window` for the
percentage, `total_token_usage` for the session total) and `turn_context.model`, folded incrementally by
`line_fold` on a background thread each 5-second sweep. Gemini: the newest `*.jsonl` under
`<GEMINI home>/tmp/<sha256(project root)>/chats/` (`getProjectHash` in gemini-cli's `paths.ts` is a plain
SHA-256 of the root path; the directory is only read when it exists), last `gemini` record's `tokens.total`
— no window, so tokens only. Cursor reports nothing usable. `App::telemetry` is keyed by feature session id;
the sidebar row shows `42%` at its right edge in the thresholds' tone, the badge label becomes
`working 6m · 42% ctx`, split tiles and the GUI contract (`SidebarSession.telemetry`) carry the same.

### 16.4 Queued follow-ups (P7, `src/fork/followups.rs`)

A per-session FIFO in `aiw_followups` (created by `FORK_SCHEMA`), mirrored in `App::followups` so drawing
reads no database. The dialog (`e` on a session row; `Ctrl+Space Q` in the pane or a view) is a multi-line
editor over the list: `Enter` queues, `Tab` reaches the list,
`J`/`K` reorder, `d` drops, `Enter` takes an item back to edit. `aiw automation queue --session <id> "<text>"`
queues from a script. Delivery is the harness asking: the Stop hooks (Claude `Stop`, Codex `Stop`, Gemini
`AfterAgent`, Cursor `stop`) run `scripts/stop-hook.sh` → `aiw stop-hook`, which sends `followup_take` and
waits ≤ 2 s for `{"followup": text|null}`; with a text it prints the continuation (`{"decision":"block",
"reason":…}`; Cursor `{"followup_message":…}`), otherwise nothing (Codex: `{}`, it requires JSON). The task
prompt is never sent on its own and nothing is ever typed into a busy agent. `stop_hook_active` is logged
only; the queue draining bounds the loop (Claude caps 8 continuations; Cursor's `loop_limit` is set to 20).
The badge reason line shows `⏭ 2 queued`.

### 16.5 Subagents and the launch context

`SubagentStart`/`SubagentStop` (Claude, Codex; Cursor `subagentStart`/`subagentStop`, whose start entry
answers `{"permission":"allow"}` itself) → `aiw subagent` → a count per feature session, cleared at
`thinking-stop` and on session end: `working 6m · 2 subagents`. At launch every agent is told
`AIW context: project …, feature …, branch …, worktree …, AIW session …. Mention the branch name in commit
messages and PR titles.` — Claude in `--append-system-prompt`,
Codex in `developer_instructions`, Gemini and Cursor through their session-start hooks (`aiw session-context`
rebuilds the paragraph from the store, read-only). `agent_context_prompt: false` turns it off.

### 16.6 Claude hooks via `--settings`

Claude's docs say hook entries merge across settings levels ("Hook entries merge across settings levels
rather than replacing each other"; "Lists merge instead of overriding"; the same handler in two files runs
once), so `claude_hooks_via_settings_flag` defaults to **on**: `fork::claude_settings::hook_entries` is the
one builder, `fork::launch` writes `<state>/claude-settings/<feature session id>.json` (`hooks` +
`statusLine`) at each launch and passes `--settings <file>`, and `ensure_notification_hooks` only removes the
entries an older AIW wrote into `.claude/settings.local.json` (the file keeps the user's entries and the
permissions diff review needs; the worktree copy still happens). With the flag off the legacy writer is used
unchanged. `aiw doctor` reports the mode and reminds about Codex's one-time `/hooks` trust.

Dropped on request (2026-10-10): the Obsidian vault log (P2b) — stability first; the PreCompact / SessionEnd
hooks it needed are not wired.

## 17. One prompt, several agents (fan-out) — design (2026-10-11)

> **Note (2026-10-11).** Written against the composer. With the cockpit (§18) the unsent seed is pasted into
> each agent's pane with `paste_text` and no Enter (`App::paste_prompt_unsent`); the GUI collects it with
> `take_unsent_prompt`. Everything else here stands. **Built 2026-10-11: see §19.3** for how it maps onto the
> cockpit and what changed from this design.

**Ask.** "Give me the option to start several sessions for the same prompt, either all in one worktree or one
worktree each." Not a split view, not a comparison tool: the launch step only. Today the `n` dialog picks one
agent, `s` adds a second agent by hand, `B` makes N features for one agent, and the prompt has to be typed
into each pane. Everything the feature needs already exists as a primitive; this section wires them.

### 17.1 What the user sees

1. `n` on a project, as now: branch, source, worktree.
2. **Agents** (the step that picked one agent): `j`/`k` move, `Space` ticks any number of the enabled
   harnesses, `Enter` continues. The first ticked one is the primary (its mode and review flags apply to
   all; Codex refuses Vibeless, so a mixed set falls back to Vibe with a note).
3. **Layout** (only when more than one agent is ticked):
   - `one worktree` — one feature, one tmux session, a window per agent (`claude`, `codex`), labels
     `Claude 1`, `Codex 1`. The dialog says: *agents share the files; good for questions, review and
     planning; for code changes pick one worktree each.*
   - `one worktree each` — sibling features `<branch>-claude`, `<branch>-codex`, each its own worktree
     from the same base, same flags, tagged with a shared `fanout_group` so the sidebar shows them as a
     group (`⇶ auth-fix · claude / codex`) and `v` on the project row tiles them side by side as it does now.
4. **Prompt** — the `TaskPrompt` step that is already drawn and handled but never reached (`CreateFeatureStep::TaskPrompt`):
   type or paste, `Ctrl+L` picks from the prompt library, `Ctrl+O` opens `$EDITOR`. Empty is allowed.
5. **Send** — `[ ] send the prompt to every agent as soon as it is ready` (default off, keeps the rule
   that the task prompt is never sent on its own). Off: every session gets the prompt **pre-filled and
   unsent**; opening a session (`Enter`, `o`) showed it in the composer, `Enter` sent. On: the prompt
   is pasted and sent when the agent's own SessionStart hook reports it ready — never typed blind into a
   pane that is still starting.
6. The dashboard lands on the primary session as today; the others show `○` with `⏸ prompt ready` in the
   reason line until their prompt is sent.

CLI: `aiw automation create-feature` gains `"agents": ["claude","codex"]` (replaces `agent`, which stays as
the one-element form), `"fan_out": "shared" | "worktree_each"`, `"prompt"` or `"prompt_file"`, `"send": bool`.
Feature presets (`extension.feature_presets`) gain the same `agents` and `fan_out` fields.

### 17.2 How it maps onto what exists

| Step | Existing piece | Change |
| --- | --- | --- |
| Agent multi-select | `CreateFeatureState.agent` / `agent_index`, Mode step with `mode_focus == 0` (`src/handlers/feature_creation.rs:339-389`) | `agents: Vec<AgentKind>` (ordered, first = primary); `agent` becomes a view of `agents[0]` so nothing downstream changes |
| Layout step | none | `CreateFeatureStep::FanOut` between Mode and SessionName; skipped when one agent |
| Prompt step | `CreateFeatureStep::TaskPrompt` + `task_prompt` + `prompt_analysis`, dead today (`state.rs:2518`, `feature_creation.rs:456-484`, `ui/dialogs/feature.rs:925`) | reached from SessionName; `startup_prompt` already flows into `PreparedFeatureLaunch` |
| Shared layout | `initialize_feature_sessions` makes the primary (`feature_ops.rs:916`); `create_agent_session_labeled_identified(pi, fi, label, Some(harness), intent)` adds one more agent window in the same session and workdir (`session_ops.rs:1570`) | after `ensure_feature_running`, loop over `agents[1..]` |
| Worktree-each layout | `create_feature` itself (`feature_ops.rs:269`); naming rules in `project.rs` (`slugify`, `worktree_name`, `tmux_session_name`) | loop `create_feature` with branch `<branch>-<agent slug>`; new `features.fanout_group` column (migration) for the sidebar grouping |
| Unsent delivery | `compose_drafts: HashMap<target, ComposeDraft>` restored by `open_compose_from_view` (`compose.rs:457`); `open_compose_seeded` seeds only the current view | seed a draft per session at launch; persist in a new `aiw_pending_prompts(session_id, text)` table so a restart keeps them; delete on send |
| Sent-on-ready | SessionStart hooks already reach `aiw session-context` for Gemini/Cursor; Claude and Codex fire SessionStart too (`docs/development/harness-hooks-reference.md`) | a `session_ready` IPC line from the session-start script; the app pastes the pending prompt with `tmux.paste_text` + Enter once, then deletes it |
| Follow-ups | `aiw automation queue` / Stop hook (§16.3) | unchanged: not usable for the first prompt, since Stop fires only after a turn |

Two commits: (1) agent multi-select + shared layout + prompt step + unsent drafts; (2) worktree-each + grouping
+ automation/presets + optional send-on-ready. Tests: dialog state transitions and naming (unit), draft
seeding and restart survival (unit, DB), an e2e scenario with the fake agents (`/perm`, `/done`) that creates
a two-agent feature both ways, checks the two windows / two worktrees, opens each session and finds the prompt
in each pane unsent, then ticks "send" on a second run and checks both panes received it exactly once.

**Out of scope here:** comparing the two results (a `git diff` between sibling worktrees in a dialog) and the
split view; both come after this lands.

## 18. Cockpit: tmux draws the dashboard (2026-10-11)

**Decision.** The dashboard is one layout, drawn by tmux: a session sidebar on the left (AIW) and the selected
agent's *real* tmux pane on the right. Typing in the right pane is native — the same as `tmux attach` — with
no key relay and no capture-and-render loop. Everything that existed only to relay keys or redraw an agent's
screen went: the in-app full view (`AppMode::Viewing`, `app/view.rs`, `handlers/view.rs`, `ui/pane.rs`'s
renderer and its "Claude sidebar" panels), the composer, the tree layout and `b`, `Tab`-typing through the
pane, the snapshot/capture workers, the startup mask, the `VIEW_*` refresh constants, the in-app split tiles
and the attach hand-over (the right pane *is* the attach). `aiw grid` stays and backs `v`.

**Launch (`src/fork/cockpit.rs`).** `aiw` with no subcommand is a thin launcher. It needs an interactive
terminal, creates (or finds) the cockpit server on its own socket — `cockpit.sock` next to AIW's private tmux
socket under the state dir, `-f /dev/null` so the user's tmux configuration never applies — with a session
`aiw`, one window, two panes: pane 0 runs `env -u TMUX aiw sidebar` (the whole dashboard app: instance lock,
IPC socket, database, hooks; a hidden subcommand), pane 1 runs `aiw sidebar --placeholder` ("select a
session"). Then it runs `tmux -S cockpit.sock attach-session` with `TMUX` unset (nested tmux works, Ghostty
without an outer tmux works) and exits when the client returns. Server settings: `status off`, `prefix None`,
`prefix2 None`, `mouse on`, `escape-time 0`, `pane-border-status top` with `@aiw_title` per pane
(`AIW` / `project › feature › session`), `pane-active-border-style` from the theme's `primary` colour,
`window-size latest`, `terminal-features ,*:RGB` when the terminal is truecolor. Root table: `C-q` →
`last-pane` (sidebar ⇄ agent), a no-op while a dialog has the sidebar zoomed. A `client-resized` hook pins
the sidebar at `sidebar_width` columns (default 36, floor 28, `config.json`), never while zoomed; the sidebar
also notices when its own viewport disagrees with that width after a resize and asks once (`cockpit_tick`),
so a resize that raced tmux's layout still ends at 36 columns.
`remain-on-exit` is **on** (the brief said off): a crashed sidebar then stays as a dead pane with its message
instead of leaving a one-pane window, and the next `aiw` respawns it; the placeholder never exits on its own.
If the server is already up, `aiw` repairs dead panes and attaches — the dashboard survives a closed terminal
window — and a second `aiw` attaches a second client (the instance lock belongs to the sidebar process, not
the launcher). `q` in the sidebar kills the cockpit server (after `SIGHUP` is ignored, since its own pane dies
with it) and the view sessions it made; the agents' own sessions on AIW's socket are untouched.

**The right pane (`src/app/fork_cockpit.rs`).** The sidebar talks to the cockpit server through a `Link`: a
channel to one worker thread (`aiw-cockpit`) that runs the tmux commands, so nothing on the draw path or the
main thread spawns a process (`workspace_frame_makes_no_tmux_calls` and the frame bench stay as they were). Each
main-loop iteration `cockpit_tick` looks at the committed workspace target — the selection after the 150 ms
debounce of §15, kept — and when it changed sends `Respawn { command, title }`: `respawn-pane -k -t aiw:0.1
'<cmd>'`, where the command is `env -u TMUX tmux -S <AIW socket> new-session -t =<feature session> -s
aiw-view-ck<pid>-<n> ; set-option destroy-unattached on ; set-option status off ; set-window-option -t :=<window>
window-size latest ; select-window -t :=<window>` — a grouped view session on AIW's socket, exactly as
`grid.rs` builds it but by window *name* — followed by `tmux -S cockpit.sock set-option -p @aiw_title 'select a
session'` and `exec aiw sidebar --placeholder`, so the pane retitles itself and falls back to the placeholder
when the agent's window goes. Only the newest `Respawn` in a batch is run. A stopped session, or a project or
feature with nothing running, shows the placeholder. `Enter` (and `i`, a pasted prompt, the needs-you picker's
open) commits the selection at once (`workspace_commit_now`), respawns if needed and sends `FocusAgent`
(`select-pane -t aiw:0.1`); looking at the agent clears the feature's generic "needs input" notes as entering
the view did. `v` respawns the pane with `env -u TMUX aiw grid <project>/<feature>; <retitle>; <agent command>`
so `Alt+q` in the grid brings the selected agent back.

**Dialogs.** When `app.mode` leaves `Normal` the tick sends `Zoom(true)` (`resize-pane -Z -t aiw:0.0`) and
`Zoom(false)` on the way back, re-pinning the width since the terminal may have been resized meanwhile; the
first-run icon check is zoomed the same way from `cli.rs`. The selection is frozen while a dialog is open, so
the pane keeps the agent behind it. Every "return to the view" a dialog had is `App::return_to_dashboard(view)`:
mode `Normal` and the sidebar on that session's row (the feature's row when the window is gone).

**Keys.** Sidebar: `j/k`, `Enter` (focus the agent), `h/l`, `/`, `i`, `a`/`X`, `e`, `c`, `x`, `r`, `d`, `N`/`n`,
`s`, `z`, `L`, `E`, `T`, `A`, `?`, `q`, `v`. Gone: `b`, `o`, `Tab`, the pane's own leader.
`Ctrl+Q` is tmux's. The old view's leader commands that still matter moved to the dashboard's
`Ctrl+Space` menu, for the selected session (`App::selection_view_state`, `src/app/session_tools.rs`): `d` diff viewer, `m` Markdown, `l` past/unsent prompts, `N` TODO capture,
`z` complete the referenced TODO; the bookmark,
picker, wizard and refresh entries stayed; `i`, `?`, `A` left the menu (they are plain keys). The session
switcher (`w`), remote-control URL copy, scroll mode and text selection went with the view: the sidebar *is*
the switcher, and tmux has copy mode.

**Prompts, unsent (`src/app/fork_prompt_paste.rs`).** Every path that opened the composer seeded with text —
TODO spawn (both kinds), prompt-library
delivery, a feature's `startup_prompt`, `Ctrl+Space l` —
calls `App::paste_prompt_unsent(pi, fi, si, text)`: it writes `.claude/latest-prompt.txt`, pastes with
`tmux.paste_text` (bracketed paste, **no Enter**), selects the row, closes whatever dialog asked, and focuses
the agent pane. The task prompt is never sent on its own. A failed paste is an error the callers stash as
before (`stash_lost_prompt`, the `unsent_prompts` table; `Ctrl+Space l` lists them first). The `App` embedded
in the desktop GUI has no cockpit, so the same call keeps the text as an `UnsentPromptHandoff` the GUI collects
with `take_unsent_prompt` — the GUI contract (`PlanHandoff.draft_prompt`, `TodoAgentLaunchResponse.draft_prompt`)
is unchanged.

**Session info block (`src/fork/session_info.rs`, `src/app/fork_session_info.rs`,
`src/ui/fork_session_info.rs`).** The bottom 8 rows of the sidebar when it is ≥ 24 rows tall, for the selected
session (a feature or project row: the agent the pane shows): title, harness · model, `ctx 42% · $0.84 · 12m
old`, `⎇ branch`, the worktree shortened to `…/<project dir>/<feature dir>`, `⏭ n queued · ✓ allowed Bash`.
Sources, all local and read-only: `App::telemetry` (the §16 status line / rollout / chat readers) for model,
context and cost; the follow-up queue; `answer_permission` now records the decision per session; and the
harness transcript the hooks already name — Claude's session `.jsonl` at `~/.claude/projects/<cwd
encoded>/<session id>.jsonl` (the `claude_session_id` the hooks bind; the first non-meta `user` message that is
not a `<command-…>` echo, the last `"type":"summary"` line, the assistant `model`), Codex's rollout from the
indexed thread id (first `input_text` user message or `user_message` event, `turn_context.model`), Gemini's
newest chat `.jsonl` for the project (first `user` record). The folds run on the 5-second sweep in a background
thread (`aiw-session-info`) through `LineFoldCache`, so a long transcript is read once and then by its appended
bytes; results are cached per session id and the draw path only formats them. No transcript → the AIW task
prompt (`latest-prompt.txt`) → the feature name. Tests use fixtures under a temp HOME, never `~/.claude`.

**Config.** The layout, composer and view-auto-refresh keys are gone; unknown keys in an existing
`config.json` are ignored and not written back (`unknown_keys_are_ignored_and_the_sidebar_width_has_a_floor`).
`tmux_control_mode` stays: the desktop GUI's terminal still uses the control-mode client. `sidebar_width` is
new.

**Also removed as bloat (2026-10-11).** The session switcher (redundant with the sidebar), the compose-only
clipboard helpers, the TUI's agent-sidebar runtime, the dead `DASHBOARD_SESSION_FILTER_ENABLED` paths in view
code. The GUI contract, `aiw-gui`, automation, hooks and IPC are unchanged.

### 18.1 Removed: harness-native duplicates (2026-10-11)

AIW is a workstation, not a second agent. Everything that re-implemented a feature Claude Code and Codex ship
themselves was deleted the same day, in nine commits on `lean-harness`, each with the full suite green:

| Removed | What the harness does instead |
|---|---|
| Remote Control (`--remote-control` flag, dialog row, URL detection) | Claude's own `claude remote-control`; already on the no-cloud remove list (§6) |
| Steering prompt ("prompt coach" box after launch) | Type into the agent's pane; prompts paste unsent |
| Learning mode (read-only repo browser + Q&A, `K`) | Ask the agent in its session |
| Model / reasoning analyzer (`m`, Codex settings socket) | `/model` and `/effort` in the harness |
| Plan interview and Quick Plan (`P`, `Q`, `AIW_PLAN.md`, question banks, expert picker) | Claude's plan mode, Codex's plan mode |
| AI Review of a PR (`W`), review memory (`C`), review destination and review questions | `/code-review`, `/review` |
| Final Review (verdict/comment viewer, walkthroughs, co-review, batched review, PR submit) and Review mode (the `review` flag and its CLAUDE.md note block) | `/code-review`, `/review` inside the agent |
| PR Triage and PR Review (`G`, comment reader, fix-prompt injection, reply drafts, investigations, companion features, `aiw reply-draft`) and the agent screenshot-evidence flow | `gh pr view --comments` and a fix in the agent's own session |
| Session summary (`Z`), fresh-context handoff (`F`), prompt overrides (`E`), the headless prompt registry and the headless runner | `/compact`, `--resume`, the harness's own prompts |

What stays: projects → features (branch / worktree) → sessions, the cockpit and `aiw grid`, badges and the
needs-you list, queued follow-ups, dashboard permission answers and the Vibeless per-edit approval popup, hooks
and the status line, context / cost telemetry, usage windows, dormancy, TODOs, bookmarks, the prompt library
(user templates pasted into a session), the plain diff viewer (`d`), the Markdown viewer, the issue browser
(`g`), the PR badge refresh (`R` / **Refresh PRs**), config wizard, harness setup, themes, `doctor`, `fonts`,
`automation` and IPC. No HTTP client or server is linked (`scripts/check-no-network.sh`).

Persistence is untouched: no `DROP TABLE` migrations were added. `learning_*`, `model_research`,
`plan_interviews`, `ai_review_cache`, `pr_review_drafts`, `pr_comment_triage`, `pr_investigations`,
`pr_review_cache`, `screenshot_scopes`, `prompt_overrides` and the `remote_*` tables, and the
`features.review` / `plan_mode` / `selected_plan_path` / `triage_source` / `review_source` columns remain in
the schema, unread. An older `aiw.db` opens as before.

**Measured** (`cargo +1.99.0 test --locked --release fork_bench -- --ignored --nocapture`, 36×60 sidebar with
the info block, 3 projects × 10 sessions, the selection alternating between a `⛔` row and a `●` row, 2,000
frames): **mean 143 µs, p50 142 µs, p95 159 µs, max 233 µs** — 12× under the 2 ms budget (the old 200×60
workspace frame was 540 µs mean; the same bench at 200×60 for the sidebar-only dashboard: 439 µs mean, 486 µs
p95). The frame makes no tmux calls (`sidebar_frame_makes_no_tmux_calls`). The agent itself costs AIW nothing:
tmux draws it.

**Resumable.** Nothing here is lost when the terminal closes: the cockpit server, the sidebar process and the
agents' tmux sessions all outlive the client, `aiw` re-attaches, and `S` resumes a saved Claude or Codex
conversation after a reboot as before.

## 19. Orchestration features (2026-10-11)

Built from docs/research/orchestration-landscape-2026-10.md, Part C. Each subsection is one proposal.

**How the pieces fit.** All ten ride on three things aiw already had: the harnesses' own hooks (now a much wider
event set, §19.1), tmux as the terminal (the agent's pane stays native), and the follow-up queue as the one safe
way to put text in front of an agent. Identity (§19.2) gives every Claude session a stable name, which messaging
(§19.5) addresses and the roster matches. Fan-out (§19.3) and layout presets (§19.4) shape what a new feature
starts; groups and notes (§19.4) organise the sidebar around it. Claude's own worktrees (§19.6) and background
sessions (§19.7) are pulled into aiw's layout and rows instead of living beside it. The watchdog (§19.8) and
schedules (§19.9) act on idle and stuck agents through the same queue and paste paths, never typing into a busy
or waiting agent. Landing (§19.10) closes a feature's loop. §19.11 is how the whole set was tested together and
against the real CLIs.

### 19.1 Richer row events from the newer hooks (proposal 5)

**What.** Claude Code 2.1 reports more than a turn starting and ending. AIW now wires `StopFailure`, `PostCompact`,
`PostModelSwitch`, `TaskCreated`, `TaskCompleted`, `CwdChanged`, `PermissionDenied` and `SessionEnd`, a second
command on `Notification`, and a second command on `Stop` that reads `background_tasks` and `session_crons`. Each
runs `scripts/hook-event.sh <kind> claude` → `aiw hook-event` (hidden), which forwards a whitelist of fields
(`fork::hook_events::FORWARDED_KEYS`; arrays as `<key>_count`) as one `hook-event` IPC line. The app folds it into
`App::hook_events[session id]` (`fork::hook_events::HookEventState`, pure, unit-tested) and the badge reads it.

**Row.** `SessionBadge` gained `tasks`, `background` and `event`. Working: `working · 42% ctx · tasks 1/3 · 1 bg ·
1 loop · compacted`. Finished: `done · 1 bg · compacted`. Idle: the event or `idle`. A `StopFailure` is ⛔ `turn
failed: <error>` and clears the working, tool and subagent markers (the harness sends no `Stop` then); it
outranks the previous turn's "done" because the failing turn is newer, and `thinking-start` retires it. One-off
events expire after ten minutes or at the next turn; a model switch persists. Notifications: `agent_needs_input`
and `elicitation_dialog` / `elicitation_url_dialog` are now questions (attention layer); `idle_prompt` and
`quota_auto_resume_*` are row events; `agent_completed` and logins add nothing.

**Dormancy and exits.** A feature with background tasks or `/loop` schedules at its last `Stop` is dropped from
the dormant scan (`App::feature_has_background_work`). The exited sweep is unchanged: if the agent process left,
the pane is a shell and `■ exited` is the truth (Claude hands live background work to its own supervisor on
exit); the exited label carries `· 1 bg` so the handoff is visible.

**Turn end clears tools.** `thinking-stop` now also clears the session's tool marker. An interrupted turn can
skip `PostToolUse`, and Codex's new hooks made that visible (a Codex row stayed ● after notify ended the turn).

**Codex.** Codex already had project-layer hooks in `<checkout>/.codex/hooks.json` (§16, git-excluded, trusted
once with `/hooks`). That file is the clean per-launch path, so no `CODEX_HOME` overlay and no `~/.codex` write
was needed: `UserPromptSubmit` → `thinking-start.sh` + `clear-notify.sh`, `PreToolUse` → `tool-start.sh`,
`PostToolUse` → `tool-stop.sh`. Codex rows now show ● from the submitted prompt instead of only at the notify
call; notify (`-c notify=[…]` per launch) still ends the turn and stays the fallback for a Codex that does not run
the hooks. Changing the hook set changes its hash, so Codex asks to trust AIW's hooks once more after this update.

**Not done.** `PreModelSwitch` (a veto hook; AIW has nothing to veto) and Codex's `SessionStart` (the launch
context already travels in `developer_instructions`).

**Tests.** `fork::hook_events` (parse, apply, TTL, task counting, notifications), `app::fork_hook_events`
(row labels, failure precedence, tool clearing, dormancy, redraw), `fork::claude_settings` and
`fork::codex_hooks` (wiring), `app::attention` (classifier). E2E: the fake claude gained `/compact`, `/model`,
`/fail`, `/task`, `/taskdone`, `/bg`, `/idle`, `/needs`, `/denied`; the fake codex fires `UserPromptSubmit` /
`PreToolUse` / `PostToolUse`; `run.sh` checks every row event, the settings wiring and Codex's live working
state. `setup-sandbox.sh` now resolves the binary to an absolute path (a relative one silently ran the
installed `aiw`).

### 19.2 Session identity: names, forks, Claude's roster (proposal 3)

**Names.** Every Claude launch carries `--name <name>` (`fork::launch::claude_launch_args` via
`LaunchContext::session_name`). The name is `fork::session_identity::base_name(feature, label)`, slugged and cut to
60 characters, assigned once per feature session (`SessionNames::assign`) and stored in a new FORK_SCHEMA table
`aiw_session_names(session_id, name)`; a collision with another AIW session appends the session id's first six
alphanumerics. It is kept across AIW renames, so `claude --resume <name>`, `/list-agents`, agent view and step 5's
messages see one stable handle. Assignment happens at every launch site: the feature start loop and the saved-
conversation pickers fill the context inline (they hold the feature mutably, so the names live on a separate
`App::session_identity` field), and the `&self` launch helpers read a name assigned just before
(`App::assign_session_name`). Names are flushed to SQLite on the status sweep and forgotten with their session or
feature. `fork::launch::only_settings_flag` accepts AIW's own `--name` pair. Codex 0.162 has no launch-time name
flag (`codex resume <name>` only resolves names set inside Codex), so Codex sessions stay unnamed. Resume stays
id-first: an ambiguous `claude --resume <name>` opens a picker, so the name is used for matching the roster, not
passed to `--resume`.

**Forks.** `f` on a Claude or Codex row with a saved conversation (`session_ops::persisted_resume_id`) opens
`AppMode::ForkSession`: (1) a new window in the same feature — `create_agent_session_with_fork` adds a
`<label> fork` session whose launch resumes the source with `--fork-session` (Claude) or runs
`codex <opts> fork <thread>` through `launch_program` (Codex); (2) a new worktree — `F`'s `ForkingFeature` flow with
the harness preset to the source's, `include_context` off and `fork_conversation` set; `create_forked_feature`
records `pending_feature_forks[(project, branch)]`, and the feature's first start
(`fork_identity::claim_feature_fork`) hands it to the first matching session, consumed once by the start loop.
A changed harness drops the fork with a toast. Claude's `--resume <id>` finds the source across the repo's
worktrees and other projects, and `--fork-session` starts in the launch directory, so the new checkout is right.

**Roster.** `fork::claude_roster` runs `claude agents --json` (verified: it does not start Claude's supervisor,
answers in ~0.1 s; `~/.claude/jobs/*/state.json` is documented as unstable, so it is not read) every 10 s from the
5 s sweep on a background thread with a 4 s timeout; the resolved binary is cached between reads.
`App::apply_claude_roster` matches entries to AIW sessions by conversation id, then by name. Matched entries with
`waitingFor` / `state: blocked` become `roster_waiting`, a needs-you badge that outranks a finished-turn attention
record but not a hook-reported question (`fork::badges::session_badge`). A hook message for the session that shows
the agent moving again (`clear`, tool or thinking start/stop, `stop`) drops its `roster_waiting` entry at once
(`App::roster_saw_hook_activity`), so an answered prompt does not stay ⛔ until the next read. The read drains
stdout on a thread while the command runs (a roster over the pipe buffer would otherwise block `claude` until the
timeout) and retries a spawn that fails with ETXTBSY, as when `claude` is being updated. Unmatched entries inside a project's
repo or a feature's checkout (most specific root wins, canonical paths compared) become `ExternalSession`s, drawn
as dimmed `↗ name · state` lines (at most two per row, then `+N more (E)`; `workspace_item_height` counts them) and
listed by `E` (`AppMode::ExternalSessions`), where `Enter` copies `claude attach <id>` for background sessions.
Making them selectable rows would have touched every `Selection`/`VisibleItem` match (over 200 sites); a list
dialog gives the same information. Config: `claude_agents_roster` (default on).

**Tests.** `fork::session_identity` (names, collisions, fork args, labels), `fork::claude_roster` (parse,
state words, attach commands, path matching, fetch with fake and slow binaries), `db::session_names`,
`app::fork_identity` (persistence, roster split, fork dialog, feature-fork claim), `handlers::fork_identity`,
`app::tests::fork_identity` (same-feature Claude fork, Codex fork, unique launch names, new-worktree fork through
the start loop, restart after a fork, roster needs-you precedence). E2E (`scripts/dev/e2e/identity.sh`): the fake
claude registers itself in a fake roster, answers `agents --json` from it plus `$HOME/.e2e-roster.json`, honours
`--name` / `--fork-session` and `codex fork`, and has `/rosterwait` / `/rosteridle`; the run checks names,
resume keeping the name, both fork targets, a Codex fork, external lines appearing and clearing, the `E` list,
and a roster-only wait turning an AIW row ⛔ and back.

### 19.3 One prompt, several agents (proposal 1, builds §17)

**Dialog.** On the New Feature dialog's harness list `Space` ticks harnesses (`CreateFeatureState::ticked`, tick
order, first = primary; `agent` follows it via `sync_primary`). One tick or none is an ordinary feature. With two
or more, the Mode step's last `Enter` (`advance_past_mode`) applies `fork::fan_out::fan_out_mode` — a Vibeless set
with Codex becomes Vibe and the reason is shown — then goes to the new `CreateFeatureStep::FanOut` (only when a new
branch gets a new worktree; the repo dir and an existing worktree can only be shared) and then to
`CreateFeatureStep::Prompt`. A single agent goes Mode → SessionName → Prompt, so the prompt step is reachable for
every feature. §17 named a dead `TaskPrompt` step and a `prompt_analysis` field; both were removed with steering,
so `Prompt` is new: a multi-line field (`Alt+Enter` / `Ctrl+J` newline, pasted text keeps its newlines), `Ctrl+L`
opens the prompt library inline (`App::prompt_templates_for_repo`: user, global and project templates), `Ctrl+T`
ticks "send when ready", `Ctrl+U` clears, `Enter` creates. Feature presets gain `agents` and `fan_out`.

**Launch.** `App::create_feature_from_dialog` (`app/fork_fan_out.rs`) sets a `FanOutRun` and calls the ordinary
`create_feature` for the primary. `finish_feature_launch` now wraps an inner function and calls
`fan_out_feature_launched` on success. Shared: once the feature's tmux session runs, each extra agent is added with
`create_agent_session_labeled_identified` (labels `fork::fan_out::shared_labels`: `Claude 1`, `Codex 1`,
`Claude 2`…), so `max_agent_autostart_sessions` does not hold them back. Worktree-each: every sibling branch
(`sibling_branches`: `<branch>-claude`, `<branch>-codex`, `-claude-2` for a repeat) is validated before the first
worktree exists; the next sibling's dialog state is built and `create_feature` runs again when the previous
feature's launch finishes, so worktree hooks, their prompts and their async completion work per sibling exactly as
for one feature. A sibling refused late (a name taken meanwhile) leaves its own dialog open; `Enter` continues the
run, `Esc` stops it (`fan_out_cancelled` lists what was not created). Siblings share a group id stored in a new
FORK_SCHEMA table `aiw_fanout_members(feature_id, group_id, base_branch, agent, position)`. Step 2's names stay
unique: siblings have different feature names, and a shared feature's labels differ.

**Prompts.** §17's compose drafts are gone, so every task prompt — fan-out or not — that has a cockpit to watch, or
comes from a dialog run, becomes a `PendingPrompt` (`queue_startup_prompt` replaces the immediate paste in
`finish_feature_launch`; without a cockpit and outside a dialog run the GUI handoff stays). It is stored in a new
FORK_SCHEMA table `aiw_pending_prompts(session_id, text, send, created_at)`, so a restart keeps it. Ready signal:
a new `session-start` hook-event kind, wired as Claude's `SessionStart` in `--settings` (`CLAUDE_MANAGED_EVENTS`
gained `SessionStart` for the legacy writer) and as Codex's `SessionStart` in `.codex/hooks.json`; it marks the
prompt ready after `READY_SETTLE` (1.5 s, so the TUI has enabled bracketed paste). Fallback, for an untrusted Codex
hook file or an older harness: after `fan_out_ready_fallback_secs` (default 15) with no hook, the prompt goes once
the pane has run something other than a plain shell for 3 s (`session_reconcile::agent_panes`, checked at most
once a second). Delivery (`pending_prompts_tick`, every loop turn) removes the prompt from memory and the database
before pasting, so it is pasted at most once; with send on, one `Enter` follows `ENTER_DELAY` (0.5 s) later. Only
the primary of a dialog launch takes the selection and the agent pane's focus, and only when no dialog is open;
automation never does. The row shows `⏸ prompt` while one waits.

**Sidebar and tiles.** The first sibling in list order draws a header line `⇶ <base> · claude / codex` with the
group's rolled-up ⛔ count (`App::fan_out_header`; `workspace_item_height` counts the line). `v` on any sibling runs
`aiw grid <first> --with <second> …` (`cockpit::grid_group_pane_command`; `grid --with` resolves each name to a
feature and tiles its first agent, `grid::Scope::Group`).

**Automation.** `aiw automation create-feature` takes `agents`, `fan_out` (`shared` | `worktree_each`, default
`worktree_each` when the feature gets a worktree), `prompt` or `prompt_file` (read by the CLI, relative to its
working directory) and `send`; `agent` stays as the one-element form. The response gains `features`: branch, agent,
workdir, tmux session, id and agent sessions per created feature. The desktop GUI's create-feature call is
unchanged (it ignores the new fields).

**Tests.** `fork::fan_out` (branches, labels, mode fallback, header, layout spelling, prompt normalising, pending
readiness), `db::fan_out`, `fork::grid` (group selection), `app::tests::fork_fan_out` (18: dialog transitions,
worktree-each and shared launches with a lenient tmux mock, unique names, a late refusal, hook-ready and fallback
delivery exactly once, send-on-ready's single Enter, restart survival, deletion, the library picker, presets, `v`
on a sibling, automation in both layouts and a refusal). E2E (`scripts/dev/e2e/fan_out.sh`, 27 checks): the fake
Codex now fires `SessionStart`; a worktree-each fan-out from the dialog with the prompt pasted unsent into both
agents exactly once and neither running it, the group header and `v` group tiles; a shared fan-out with send when
ready, each agent running the prompt exactly once; and `aiw automation create-feature` with `prompt_file` and
`send`, with no prompt left waiting at the end.

### 19.4 Groups, notes and layout presets (proposal 10)

**Model.** Pure rules in `src/fork/organize.rs`, App side in `src/app/fork_organize.rs` (`OrganizeRuntime` on App
field `organize`: groups, memberships, notes, a pending layout), storage in `src/db/organize.rs`. Three new
FORK_SCHEMA tables, no numbered migration: `aiw_feature_groups(id, project_id, name, color, position, collapsed)`,
`aiw_feature_group_members(feature_id PK, group_id)` and `aiw_notes(kind, target_id, text, updated_at)` keyed by
`(kind, target_id)` with kind `feature` or `session`. Deleting a session, feature or project removes its rows
(`forget_organize_session` / `_feature` / `_project`, called beside the §19.2/§19.3 cleanups).

**Groups without a new row kind.** A selectable group header would need a new `Selection` variant, which touches
hundreds of match sites. Instead `order_features` puts ungrouped features first (the old repo-dir-then-creation
order) and then each group by position, and `feature_slots` gives every feature a slot: `Ungrouped`, `GroupHead`
(the header line is drawn above it, like the fan-out header, and counted by `workspace_item_height`),
`GroupMember`, `CollapsedGroup` (the group's first feature, whose row is drawn as `▸ ■ name · N features` with the
group's ⛔ count) or `Hidden`. `visible_items` skips hidden features and the sessions under a collapsed group's
row, so selection, scrolling, mouse rows and `Ctrl+j/k` work unchanged. Collapsing moves a selection that would
vanish to the group's row. `Enter` / `l` on that row open the group (the cockpit's Enter-focuses-the-pane rule
steps aside for it), `h` on a folded member folds its group, and a search jump opens the group it lands in.
`fan_out_heads_group` now puts the `⇶` header above the first sibling drawn as a feature row, so a fan-out pair
inside a user group (or split across groups) still has one header. Colours are six named ANSI colours; a new group
takes the first one the project's groups do not use.

**Keys.** `G` the groups dialog (`Enter` move here / `n` new / `r` rename / `c` colour / `J` `K` order / `d` then
`y` delete / `Esc`), `C` fold or unfold the selected feature's group, `#` the note editor (`Enter` save, `Alt+Enter`
or `Ctrl+J` newline, `Ctrl+U` clear, `Esc` cancel). With a feature selected, "+ New group…" creates the group and
moves the feature in at once.

**Notes.** Normalised on save (trailing spaces and blank edge lines dropped; empty deletes). Rows with a note show
`✎`; the info block grows one row for the first line of the session's note, else the feature's
(`SessionInfo.note`, `App::session_info_has_note`); `/` matches note text and group names after names, showing the
matching line.

**Layout presets.** `ExtensionConfig.layout_presets` (merged by name, project wins) holds `LayoutPreset { name,
description, sessions: [LayoutSessionSpec { kind, agent, name, label, command }] }` as plain strings, so a typo
never breaks loading `aiw.json`; `resolve_layout` checks it against the project's custom sessions and enabled
harnesses and returns an error naming the preset and the entry. The New Feature dialog gains a Layout row after
Chrome (Claude) or Mode (`CreateFeatureState.layout_presets`, `layout_index`, `layout_focus`, `max_mode_focus`); a
feature preset's `layout` selects one (a missing name is a toast). `create_feature_from_dialog` resolves the choice
before anything is created and queues a `PendingLayout { project, preset, remaining }` (one per worktree-each
sibling); `finish_feature_launch` calls `layout_feature_launched` after the fan-out continuation, and
`apply_layout_to_feature` opens the sessions in order (agents through `create_agent_session_labeled_identified`,
terminals with `add_terminal_session_for_picker` then `run_shell_command` for the command, Neovim, custom sessions
with their `pre_check`), keeps the selection where it was, and reports any session that failed while the rest
open. `aiw automation create-feature` accepts `layout` (`create_feature_from_request_organized` wraps the fan-out
path: a bad layout fails the request first; per-feature problems go in the response message). The desktop GUI does
not show groups, notes or layouts yet.

**Tests.** `fork::organize` (order, slots, names, colours, notes, layout resolution and errors, lenient parsing),
`db::organize`, `extension` (merge by name, preset `layout`), `handlers::fork_organize` (the editor's keys),
`ui::fork_session_info` (the note row), `app::tests::fork_organize` (10: order and folding with navigation and
restart survival; the dialog end to end; a fan-out pair inside a group with both headers drawn; notes through `#`
with the info block, search, a jump that opens a folded group, deletion; a layout opened on launch; a bad layout
refused before creation; a feature preset's layout; automation in every sibling and a refusal; project deletion).
E2E (`scripts/dev/e2e/organize.sh`): groups in alpha with the fan-out pair inside one, a duplicate name refused,
`C` fold saved and a search jump reopening it; notes on a feature and a session, the `✎` marks, the info-block line
and `/` by note text; a project whose `aiw.json` has a layout preset — a broken preset refused before anything
starts, then a feature opening claude, a terminal that ran its command and a custom session.

### 19.5 Messages to agent sessions (proposal 9)

**Ask.** Tell an agent something without typing into its pane, and let agents address each other. `@` on the
dashboard opens a small dialog (target, text, how it will travel); `aiw automation message --to <session> --text
<text> [--from <name>]` does the same from a script and prints `"status": "delivered" | "queued" | "failed"`.

**Claude's inbox.** Claude Code 2.1.224+ binds a per-session inbox (a Unix socket, user-only) and exports its path
to every hook as `CLAUDE_CODE_MESSAGING_SOCKET`, with a per-session token in `CLAUDE_CODE_MESSAGING_TOKEN`. The
`SessionStart` hook already runs `aiw hook-event --kind session-start` (§19.3); for Claude it now forwards both
variables (`hook_cli::inbox_fields`, a well-formed absolute path and a 32-hex token only) over AIW's own user-only
IPC socket. `App::messaging_session_started` keeps the address per feature session; `SessionEnd`, deleting the
session, or a post that finds the socket gone forgets it.

**The wire format** was read from the installed Claude Code 2.1.296 bundle (the native binary embeds its JS; the
inbox handler and its own debug hint, `Inject messages (auth line …): { echo '{"type":"auth","token":…}'; echo
'{"type":"user","message":{"role":"user","content":"hello"}}'; } | socat - UNIX-CONNECT:…`). One JSON object per
line; an optional `{"type":"auth","token":…}` first (required only on Windows); then `{"type":"user","message":
{"role":"user","content":<string>},"from":<sender>,"uuid":<id>,"session_id":<conversation>}`. Claude queues a
`user` line as the next prompt (`priority` defaults to `next`, so a running turn is not interrupted), shows `from`
as the sender, and drops a line whose `session_id` is not the receiving conversation — AIW always sends the
conversation id it learned from the same hook, so a stale socket path (or a pid reused by another Claude) never
gets the message. Claude closes a connection that sends no complete line within 30 s, so AIW connects only with
the message ready. `fork::messaging::post` runs connect + write on a helper thread with a 2 s timeout, after
checking the path is a socket owned by the current user.

**Fallback.** Codex, a Claude whose hook never reported an inbox, `messaging_inbox: false`, and an inbox that turns
out to be gone all go through the follow-up queue (§16): delivered when the agent next finishes a turn, never typed
into a busy pane. A queued message from a named sender is prefixed `Message from <name>:` since the queue pastes it
as a plain prompt. The route is shown in the dialog and returned by automation (`via: inbox | followup-queue`).

**Addressing.** `--to` takes a session id, a Claude session's `--name` (§19.2 — the same name `/list-agents` and
Claude's own `SendMessage` use, so agents can address siblings by it), the derived `<feature>-<label>` name (which
Codex sessions also answer to), `<feature>/<label>`, or a label unique across AIW. Ambiguity is an error listing
the matches.

**Inbound policy and trust.** Messages AIW posts are not from a child of the receiving Claude, so Claude applies
its inbound rule. AIW sets `crossSessionInbound` in each launch's `--settings` from `claude_cross_session_inbound`
(default `accept`). `accept` means messages from your other local sessions are delivered without an approval
dialog; the socket is restricted to your user, and permission prompts still apply to whatever a message asks for.
It is deliberately *not* applied to SuperVibe sessions: a bypass-permissions session accepting peer messages would
let one agent run commands in another without any prompt, which is the case Claude's default holds for approval.
`hold` and `refuse` apply to every mode; `default` sets nothing.

**The token** stays in memory only. `aiw_inbox_addresses` stores the socket path and conversation id (no token
column): `aiw.db` is not a secrets file, and on Linux Claude accepts a line without the auth line, so a reloaded
address still delivers after a restart. The token is never logged.

**Tests.** `fork::messaging` (line shape, auth line only for a valid token, the hook-event address, a real
socket round trip, gone / not-a-socket / refused told apart, a listener that never reads times out without
blocking, routing, delivery reports, queue prefix, target resolution and ambiguity, the inbound value per mode),
`db::messaging` (round trip, no token in the file), `hook_cli` (only well-formed inbox fields), `fork::launch` (the
settings value per policy and mode), `handlers::fork_messaging` (keys), `app::tests::fork_messaging` (11: inbox
delivery with auth and conversation id and nothing queued; Codex and inbox-less Claude queued with the sender named;
a stale inbox forgotten and queued; inbox delivery off; session end and deletion forget; bad input; restart keeps
the address without the token; the dialog end to end; automation by name, by pair, unknown; the IPC dispatcher).
E2E (`scripts/dev/e2e/messaging.sh`): the fake claude binds a real inbox socket and exports it to its hooks; AIW
stores the socket; `@` delivers into it once, authenticated and naming the conversation, with nothing queued;
automation by name from `ci` delivers; an unknown target fails; a Codex target is shown as via the queue, the
message is not typed into its idle pane, and its next turn end delivers it exactly once.

### 19.6 Claude's own worktrees in AIW's layout (proposal 6)

**Ask.** Claude Code makes worktrees by itself — `claude --worktree`, the `EnterWorktree` tool, a subagent with
`isolation: worktree`, a workflow agent, background-session isolation — in `<repo>/.claude/worktrees/<name>`, which
AIW never saw: no included files, no port block, nothing in the sidebar, a doctor orphan. Claude lets a
`WorktreeCreate` hook replace its `git worktree add` entirely, so AIW provides one.

**The contract** (read from the hooks reference, "WorktreeCreate" / "WorktreeRemove", and checked against the
installed Claude 2.1.296). `WorktreeCreate` gets `name` (a slug, given or generated, e.g. `bold-oak-a3f2`; Claude
names a subagent's `agent-<hex>`) plus the common fields (`session_id`, `transcript_path`, `cwd`, `hook_event_name`;
`agent_id` / `agent_type` inside a subagent) on stdin. A command hook prints the worktree's absolute path as the last
non-empty stdout line (ANSI stripped); failure or no path fails the create; the path must be normalized and must not
pass through a symlink below the repository root. With a hook, Claude does not process `.worktreeinclude` and does
not sweep the worktree. `WorktreeRemove` fires when Claude removes a hook-created worktree (exit from a worktree
session, `ExitWorktree`, deleting a background session; not when a subagent finishes — those stay) with
`worktree_path`; exit 0 counts as removed, a non-zero exit with the directory still there refuses. There is **no
force field**; Claude never deletes a hook-created worktree's branch.

**Wiring.** Both hooks run `<hooks dir>/worktree-hook.sh create|remove` (`aiw worktree-hook create|remove`, hidden),
added to every Claude session's per-launch `--settings` file (`fork::launch::claude_settings_json`) whether AIW's other
hooks go there or into the repo file — never into the repo's `.claude/settings.local.json`, so a `claude` you start
yourself keeps Claude's default. The script is installed by the launch path itself (`claude_worktrees::ensure_script`),
so `app/setup.rs` is untouched. Unlike AIW's other hook scripts it passes stdout and the exit status through and does
not check `AIW_ACTIVE`. Timeouts: 120 s create, 60 s remove. Off with `claude_worktree_hooks: false`.

**Create** (`fork::claude_worktrees::create`), in the hook process, never waiting on the dashboard:
- the requesting checkout is `git rev-parse --show-toplevel` of the input `cwd`; the repository's main checkout is the
  first entry of `git worktree list --porcelain`;
- names follow Claude: directory `claude-<slug>` under AIW's worktree folder (`worktree_paths::new_worktree_path`),
  branch `worktree-<slug>`; `-2`, `-3`, … up to 50 when either is taken; the slug keeps `[a-z0-9._-]`, collapses the
  rest, never yields `..`;
- the base is the requesting checkout's `HEAD` — a subagent works on the feature, not on `main` (Claude's own default
  is `origin/HEAD`; `worktree.baseRef: head` is the equivalent);
- `WorktreeManager::create_from` does the `git worktree add -b`, the `settings.local.json` merge and the included
  files (`.worktreeinclude` / `worktree_include`) exactly as a new feature does;
- a port block is reserved under `claude-worktree:<id>` and written to `<worktree>/.aiw/ports.env` (`.aiw/` is in
  AIW's `info/exclude` patterns, so the worktree stays clean); AIW's terminals there source it;
- the record goes into `aiw_claude_worktrees` through `AiwDb::open_for_hook` (the existing store only: no migrations,
  a 5 s busy timeout, only this table created if missing); a missing store still yields a worktree, just unrecorded;
- a `claude-worktree` message on AIW's socket is fire-and-forget; a dashboard that is not running reads the table at
  its next start;
- a name Claude already has in this repo (a resumed `--worktree` session) gets the same worktree back.
All diagnostics go to stderr; stdout carries only the path.

**Remove** (`claude_worktrees::remove`, shared by the hook and the dashboard): refused, with the `git status` entries,
when the worktree has uncommitted or untracked files (ignored ones such as the copied `.env` and `ports.env` do not
count); then `git worktree remove` (no `--force`), empty-folder pruning, the branch deleted with `-D` only when it
has no commit beyond its recorded base (otherwise kept, and said so), the port block released, the record dropped.
A path AIW has no record of is removed only if it is a linked git worktree (so Claude's own pre-hook worktrees
still clean up); anything else is refused. A path already gone just clears the bookkeeping.

**Dashboard** (`app::fork_claude_worktrees`). The table is read at start and on every hook message; a recorded
worktree whose directory is gone is forgotten (ports released). Ownership is worked out once per load: the AIW
session whose Claude asked (`AIW_FEATURE_SESSION_ID` in the hook's environment), else the deepest feature workdir
containing the requesting checkout, else the project by repository. A worktree is drawn as a dimmed `⎇ <name> ·
<origin>` line under its feature (`subagent <type>` or `worktree`), up to three then `⎇ +N more (W)`; one no feature
claims is drawn under the project. Not a new row kind — like the outside sessions of §19.2. `W` lists the selected
feature's (or, on a project row, the project's) Claude worktrees: `t` opens a terminal session `⎇ <name>` in the
feature, `cd`s into the worktree and sources `ports.env`; `y` copies the path; `d` then `y` removes it under the
rules above. `aiw doctor` no longer reports recorded Claude worktrees as orphans. Chosen over registering them as
features: Claude owns the process living in them, and a feature implies AIW-managed tmux sessions.

**Tests.** `fork::claude_worktrees` (slugs, Claude's names and collision suffixes, the create and remove inputs,
origin labels, the settings entries and script, `ports.env`, summaries; against real git in temp repos: a create
lands in AIW's folder from the feature's HEAD with the `.env` and a port block, clean, recorded with its session,
reused by name, with no dashboard listening; collisions; no store; outside a repo; dirty refused then clean removed
with its branch and ports; a branch with commits kept; a vanished worktree forgotten; a plain folder and the main
checkout refused), `db::claude_worktrees` (round trip, `FORK_SCHEMA` and the hook DDL agree, the hook never creates
the store, adds the table to an older one), `fork::launch` (both hooks ride `--settings` in both hook modes, off when
the flag is off, the script installed), `ui::fork_workspace` (lines under the feature, capped with `+N more`),
`app::tests::fork_claude_worktrees` (6: listed under the session's feature with a toast and one more sidebar row; the
checkout finds the feature and a deleted feature falls back to the project; a worktree made while AIW was closed shows
at start and a vanished one is forgotten; `W` refuses a dirty one with the reason and removes a clean one after `d` `y`;
nothing to list; `t` opens the terminal with `cd` and `ports.env`). E2E (`scripts/dev/e2e/worktree_hooks.sh`): the fake
claude runs the hook from its `--settings` exactly as Claude does; the settings carry both hooks and nothing lands in
the repo file; a subagent worktree lands in AIW's folder as `claude-agent-e2e1` on `worktree-agent-e2e1` from the
feature's HEAD with the `.env`, a port block, clean, recorded with its session and origin, drawn under `lay-1`, listed
by `W`; `/wt` (EnterWorktree) gets `claude-e2e-enter`; WorktreeRemove refuses it dirty (Claude is told why), removes it
clean with its branch and record, and the row goes; `W` → `t` opens a terminal in the worktree with `AIW_PORT`; `W` →
`d` `y` removes; `aiw doctor` does not list a Claude worktree as an orphan.

### 19.7 Background Claude sessions (proposal 4)

**Ask.** A Claude session that keeps working when the terminal, the cockpit or AIW itself closes, attached only when
you want to look. Claude Code has this natively: `claude --bg` hands the session to Claude's own supervisor daemon,
`claude agents --json` lists it with a state, `claude attach <id>` opens it in a terminal, and `logs`, `stop`,
`respawn`, `rm` manage it. AIW starts such sessions, shows them as rows, and attaches on demand; it does not run its
own daemon.

**What a background session is in the store.** An ordinary `SessionKind::Claude` session whose window name starts
with `aiw-bg-` (`FeatureSession::is_background`). Chosen over a new `SessionKind` (265 match sites): the window name
is persisted for free, `runs_with_feature()` excludes the session so a feature start never creates its window or
launches an interactive Claude into it, and the exited sweep (`fork::session_reconcile`) skips it. The window exists
only while you are attached. `aiw_background_sessions(session_id, bg_id, conversation_id)` keeps the short id
`claude --bg` printed and, once learned from the roster or a hook, the conversation id (FORK_SCHEMA, no numbered
migration).

**Start.** Two routes, both opt-in behind `claude_background_sessions` (default true; the default harness stays
interactive): the `s` picker's "Claude (background)" (right after Claude; then the name, default `Claude bg N`, then
a first-prompt dialog — empty means it waits for you), and `b` on the New Feature dialog's harness list, which turns
the highlighted Claude into "Claude (background)" for a single-agent feature (not a fan-out). The dialog route records
`(project, feature)`; the feature's first start claims its first Claude session without a conversation
(`claim_feature_background`) by renaming its window. A feature whose only tmux-backed sessions are background ones gets
**no tmux session** at start (it would hold an idle shell); `c` on it restarts what Claude dropped. The task prompt of
such a feature goes onto the `claude --bg` command line instead of the paste path (`background_take_startup_prompt`,
and a fan-out pending prompt is taken over the same way).

**Launch** (`App::background_tick`, every loop turn): a pending session runs `claude --bg <AIW's usual Claude flags>
[prompt]` on a thread with a 30 s timeout — the same `--settings` file (hooks, status line, worktree hooks,
`crossSessionInbound`), `--name` (§19.2), mode flags and context paragraph as an interactive launch. Two additions to
that settings file (`fork::background::amend_settings_file`):
- an `env` block with `AIW_ACTIVE`, `AIW_SESSION`, `AIW_TMUX_SESSION`, `AIW_TMUX_WINDOW`, `AIW_FEATURE_SESSION_ID`,
  `AIW_BIN`, `AIW_TMUX_BIN`, `AIW_TMUX_SOCKET`, the `XDG_*` dirs and the feature's port block. Claude's supervisor,
  not the shell that ran `claude --bg`, starts the process, so AIW's hook scripts (which identify the session from
  these variables) must not rely on environment inheritance; `--settings` is documented as kept when backgrounding;
- `"worktree": {"bgIsolation": "none"}`: Claude otherwise moves a background session into a fresh worktree before
  its first edit unless it already runs in a linked worktree, and a feature in the repo dir is not one. The docs name
  this setting for project settings; whether Claude honours it from `--settings` is for the real-harness pass to
  confirm.
The short id is read from the output (`started a copy as <id>` wins over `backgrounded <id>` wins over
`claude attach <id>`, the formats in the installed 2.1.296 bundle). The process runner drains both pipes on threads
and stops waiting a moment after the process exits, so a supervisor started by the first `--bg` that inherited a pipe
cannot hang the launch.

**State.** `claude agents --json` (the §19.2 roster) is read every 3 s while a background session exists (every 10 s
otherwise), even with `claude_agents_roster` off. An entry is matched by short id, then conversation id, then name;
its `state`/`status`/`waitingFor` map to `working`, `waiting: <why>`, `done`, `failed`, `stopped`; a session Claude no
longer lists is `not running`. The session's own hooks overlay that: a question raised by a hook, or a turn a hook
says is running, shows at once. The badge carries `bg ·` before its label (`bg · working 2m`, `bg · permission
prompt`, `bg · stopped · Enter resumes`, `bg · not running · c resumes`); the feature rolls its background sessions up
even while it has no tmux session. The info block says `Claude Code (background)` and leads its last row with
`bg <id> · <state> · <last line of claude logs>` (logs read off the UI thread every 5 s for the selected session).

**Attach** (`Enter` on the row). AIW creates the feature's tmux session if it has none (the port block reserved first)
or a window `aiw-bg-claude…` in it, runs `claude attach <id>; <tmux> kill-window -t =<session>:<window>` there and
focuses it in the cockpit's agent pane like any session window. Detaching in Claude ends `attach`, the window closes
itself, the cockpit falls back to its placeholder, and a 2 s window check returns the row to "in the background". An
attach of a stopped session resumes it (Claude's own behaviour). `Enter` on an attached row just focuses the pane.

**Manage.** `x` → `claude stop <id>` (the conversation is kept). `c` → `claude respawn <id>` (running or stopped,
conversation intact); when Claude no longer lists the session, `claude --resume <conversation> --bg` with no other
flags (with flags Claude starts a copy under a new id); when it never started, a fresh `--bg`. `d` opens a
confirmation; `y` runs `claude rm <id>`. A refusal (a dirty worktree, unpushed commits) is shown in Claude's words
with the exact flag Claude printed (`--force-remove-worktree <wt>` / `--discard-unpushed <commit>@<wt>`), and a second
`y` reruns `rm` with it; only then is the row removed. Removing the row any other way (deleting the feature) runs
`claude stop` so nothing keeps running unseen; the conversation and Claude's list entry stay.

**Not done.** Background Codex (Codex has no supervisor mode). Attaching to background sessions started outside AIW
(they are the dimmed `↗` lines of §19.2; `E` copies their `claude attach`). A restart of AIW reattaches state from
the roster by the stored short id; a session the roster no longer lists shows `not running` with `c` to resume.

**Tests.** `fork::background` (window names, the short id from `--bg` output including the copy note, launch / resume
/ verb / rm arguments, rm refusals and their flags vs the usage line, roster states, log tails without escapes, the
attach command closing its window, the settings `env` and `bgIsolation` amendment, the runner reading both streams and
not waiting for a daemon holding the pipe), `db::background` (round trip, update, delete), `app::fork_background`
(the dialog claim, states from pending / roster / errors), `ui::fork_session_info` (the background line leads the last
row), `app::tests::fork_background` (10 against a fake `claude` script: the picker names, prompts and launches with
`--settings` / `--name` / the mode / the prompt, the settings `env` and `bgIsolation`, no tmux and a stored id; the
New Feature `b` (off for a fan-out, back for one Claude) launches without a tmux session with the task prompt; roster
states and hooks drive the badge and the stopped feature's roll-up; attach opens a self-closing window and detach
returns the row; `x` / `c` / a refused then forced `rm`; a dropped session resumes its conversation; the selected
session's logs reach the info block; a feature start skips background sessions; a background-only feature opens no
tmux session; removing the row stops it in Claude). E2E (`scripts/dev/e2e/background.sh`; the fake `claude --bg`
starts a detached worker from a scrubbed environment plus the settings `env`, like Claude's supervisor): the picker
route with the prompt, `--settings` and `--name`, the stored id, no tmux window; the worker's hooks reach AIW only
through the settings env and the row reads `bg · done`; the info block shows the id, state and latest output; `Enter`
attaches in the agent pane, typing reaches the session, `/detach` closes the window; `x` stops, `c` respawns; `d` →
`claude rm` refused over a dirty checkout with Claude's words, `y` again removes with the printed flag; the New
Feature `b` route starts the feature's Claude with `--bg` and its task prompt and no tmux session.

### 19.8 Watchdog recovery (proposal 8)

**Ask.** With several agents running, the ones that quietly stop making progress are the expensive ones: an agent
that exited, a turn whose API call hangs, a follow-up queued for a session that is idle (so the `Stop` hook that
would deliver it never fires), a session sitting at its prompt with a nearly full context. Say so on the row and in
the needs-you list, and offer the fix; never act on its own unless asked to.

**Detectors** (`fork::watchdog`, pure; `app::fork_watchdog` feeds them every 2 s from what the app already knows):
- *exited*: the §16 exited sweep's marker (`exited_sessions` / the `exited` status text);
- *background failed*: §19.7's roster state `failed`;
- *stuck*: `badges::session_working` AND no hook event of any kind for `watchdog_stuck_minutes` AND the pane's
  fingerprint unchanged for the same window. Two signals because each alone misleads: a long build prints but sends no
  hooks; a long tool call sends `PreToolUse` and then nothing while its output moves. Running subagents (§16 P5)
  suppress it — the parent waits on them quietly. The last hook is stamped where every hook already arrives (one call
  beside §19.2's `roster_saw_hook_activity` in the IPC handler, one in the `hook-event` handler); any hook clears a
  stuck flag at once;
- *queued, idle*: follow-ups queued (the §19.5 fallback lands there too) for a running session that has not been
  working for `watchdog_queued_idle_secs`;
- *context critical, idle*: the existing critical band while not working.
A session already waiting on the user — a pending permission (P6), a question or wait attention record that applies
to it, an unexplained pending input, a §19.2 roster wait — is never flagged stuck, queued or critical: a permission
prompt looks exactly like a stuck turn (working, quiet, still), and pasting into it would answer the question. Exited
and failed still count.

**The pane fingerprint.** Working sessions' panes are captured (`capture_pane_ansi`) on a helper thread at a quarter
of the stuck window (2–30 s), never on the draw path; sessions not working are not captured and their fingerprint is
dropped, so each turn starts fresh. The fingerprint hashes the visible text with ANSI stripped, except that on the
harness's own status line (a line with `esc to interrupt`, or one starting with a spinner glyph) digits and spinner
glyphs are dropped: Claude and Codex keep animating the spinner and counting seconds and tokens while an API call
hangs, so a raw hash would never stay still. Every other line keeps its digits, so `Compiling 12/400` → `13/400` is
progress. Tests feed fingerprints directly (`watchdog_record_pane`); the capture thread is enabled only in the real
dashboard (`WatchdogRuntime::production`).

**Surface.** A flag carries `kind`, `since` and a detail; its reason ends with `· ! recover`. Exited keeps the ■
exited badge with `! recover` added; stuck and queued-idle become ⛔ needs-you with the reason (ahead of the attention
record, behind a failed turn); context-critical becomes ⛔ only when nothing else (finished, question) speaks for the
row. The needs-you list gains `AttentionRow::Watchdog { session_id, project_name, feature_name }` rows after the
attention and pending rows (most urgent kind first, then oldest): `Enter` opens the actions, `x` dismisses. A dismissed
flag is snoozed per `(session, kind)` for the stuck window (at least a minute).

**Actions** (`!` on a session row, or on a feature row for its flagged session; `fork::watchdog::actions_for` picks
the ones that fit; `1`–`9` run one directly):
- *Restart, resuming*: an exited agent goes through `restart_stopped_session_window_resuming` with the session's
  persisted conversation id for any harness (the default `None` resumes only Claude), behind the resource gate
  (`PendingStart::WatchdogRestart` replays it after confirmation with the id kept). A stuck agent is ended first: the
  pane's shell pid comes from `pane_commands`, its child trees get SIGTERM then SIGKILL after 3 s on a helper thread
  (`resources::procs::terminate_tree`), the shell stays, and a pending restart relaunches as soon as
  `exited_agent_pane` sees the shell (row: `restarting…`), giving up after 30 s. A background session runs
  `claude respawn` (§19.7).
- *Interrupt*: `Escape` into the pane.
- *Send /compact*, *Paste the latest prompt again*, *Deliver the first queued item now*: only into an agent that is
  running, not working and not waiting on the user. `/compact` is typed (`send_literal`) and its Enter scheduled
  through §19.3's delayed-Enter map (`schedule_prompt_enter`). Replay delivers a §19.3 pending prompt if one waits,
  else pastes the latest prompt (harness transcripts, else `.claude/latest-prompt.txt`) unsent. Deliver peeks the
  queue, pastes, and only then takes the item, so a failed paste leaves it queued; the rest still go at turn ends.
- *Dismiss*.

**A dead agent stops reading as working.** An agent that dies mid-turn — ended by the watchdog, crashed, exited —
sends no `Stop`, so its per-session working markers (`ipc_thinking_feature_sessions`, `ipc_tool_feature_sessions`, the
conversation-keyed sets, subagent counts) used to survive the relaunch: the new agent read as working, and with the
watchdog, as stuck again within the window. `clear_session_working_markers` now runs when the exited sweep marks a
session exited (`mark_session_exited`), when the watchdog ends a stuck agent, and before any watchdog relaunch. The
thinking hook also marks the feature's tmux session, which sync turns into the feature-level marker that every agent
of the feature without a marker of its own falls back to (`session_working`); it is dropped too unless a sibling still
has its own marker — otherwise a dead Claude left the idle Codex beside it reading as working, then stuck. Both were
caught by the first e2e runs.

**Automatic restart.** `watchdog_auto_restart` (default false): an exited flag is relaunched resuming, at most once per
session per hour, only when `check_start_preconditions` is `Ok`, and logged. Nothing else is ever automatic.

**Config.** `watchdog` (true), `watchdog_stuck_minutes` (10.0; fractions allowed so the e2e can use 6 s),
`watchdog_queued_idle_secs` (30), `watchdog_auto_restart` (false). No schema change: flags, snoozes and fingerprints are
in memory (a restart of AIW re-derives them).

**Housekeeping in this step.** Test runs leaked one `aiw-test-home-<pid>` (and `aiw-test-data-`, `aiw-test-tmux-`)
directory each, because the process-global roots are never dropped; 2,204 of them filled `/tmp` during §19.7. An
`atexit` handler removes the current run's roots and every run first sweeps roots whose process is gone (another
user's live process counts as alive). The e2e check "nothing was typed through the sidebar" read the whole sidebar,
including the session info block, whose title is the agent's own first prompt once its transcript is read — a race
with the transcript reader, not typed input; it now skips the info block (`cap_no_info`) and asserts no dialog opened.

**Tests.** `fork::watchdog` (12: two-signal stuck, subagent suppression, waiting-on-user suppression, the stuck
detail, priority of exited/failed, queued grace, context only at the prompt, the fingerprint ignoring spinner/timer/
token ticks but not other digits, thresholds with fractions and nonsense, hourly auto-restart, actions per flag,
reasons), `app::tests::fork_watchdog` (20 against `MockTmuxOps`: stuck flagged / badge / needs-you row / actions and
cleared by a hook; moving output and subagents; a question blocks both stuck and pasting; interrupt; exited restart
with `--resume conv-1`; Codex restart with its thread id; a stuck restart kills the pane's children, clears its
working markers and relaunches once the pane is a shell, and gives up at the deadline; an agent that died mid-turn
loses its working markers when the sweep marks it exited; the feature-level marker goes unless a sibling still works; queued-idle flag, badge and deliver-once with the rest kept; a
failed paste keeps the item; never into a working agent; context-critical `/compact`; replay of a pending prompt;
dismiss snoozing; the needs-you row's Enter and `x`; `!` unflagged; auto-restart off by default then hourly; turning
the watchdog off), `project` (the test-dir sweep). E2E (`scripts/dev/e2e/watchdog.sh`; aiw restarted with a 6 s stuck
window and a 3 s grace; the fake gained `/hang` — a turn that only ticks its status line — `/ctx N`, and a log of every
input line): a hung claude turns stuck after the window and `i` lists it; `!` → Restart ends it and relaunches with
`--resume <conversation>`; `/exit` → listed, `!` → Restart resumes; `/ctx 92` → flagged, `!` → `/compact` typed once and
the flag clears when the context drops; a follow-up queued for an idle codex is flagged and not typed, `!` → Deliver
runs it exactly once and empties the queue.

### 19.9 Scheduled and recurring prompts and commands (proposal 7)

**Ask.** Agents and terminals that should do something on a clock — a nightly test run, a status summary every two
hours, "check CI" at 09:00 on weekdays, a one-off reminder this afternoon — without typing into a busy agent, without
answering a question for you, and without piling up runs when nobody is looking.

**Model** (`fork::schedules`, pure; `db::schedules`; `app::fork_schedules`). A schedule belongs to a feature; a
*prompt* schedule goes to an agent session — pinned (`session_id`), by label (`target`), or the feature's first agent
at fire time — and a *shell* schedule runs its command in a named terminal of the feature (`target`, default
`schedule`, created as a Terminal session when missing, without moving the selection). Fields: spec text (canonical),
text, paused, `start_if_stopped`, `catch_up` (`once` | `none`), `end_at`, `max_runs`, `runs`, `last_fire`,
`created_at`, `config_name` (for `aiw.json` ones), `queued_followup` (the follow-up id its last fire queued). Tables
`aiw_schedules` and `aiw_schedule_runs` (history capped at 100 per schedule) via `FORK_SCHEMA`; no numbered or
destructive migration.

**The spec.** `every <n><unit>…` (s/m/h/d, words and spaces accepted; at least `schedule_min_interval_secs`, default
60), `daily HH:MM`, `cron M H DOM MON DOW` (numbers, `*`, `a-b`, lists, steps `*/n` / `a-b/n` / `a/n`; 0 and 7 are
Sunday; Vixie OR when both day fields are restricted; names, `L`, `W`, `#`, `?` and macros are refused with a message
naming the supported subset), and `at YYYY-MM-DD HH:MM` / RFC 3339 / bare `HH:MM` (pinned to its next occurrence when
saved). The next fire is computed from the wall clock every time, generic over `chrono::TimeZone` so tests use a
fixed-rule test zone: `every` is `last_fire` (or `created_at`) plus the interval; wall-clock rules walk local days and
resolve each candidate minute — a skipped local time runs at the first minute after the gap, a repeated one at the
earlier instant (so it fires once). No stored "next" value, so DST and clock jumps need no bookkeeping.

**The timer** rides on the app loop (`schedules_tick` beside `watchdog_tick` in `cli.rs`), looks every 2 s, and re-reads
`aiw.json` schedules every 30 s. A fire is due when the next fire time is not in the future. Due before AIW started =
missed: `catch_up: none` records `missed while aiw was not running`; `once` (default) fires once — `last_fire` becomes
the fire's wall time, so many missed slots never become many runs. Schedules whose feature (or pinned session) is gone
are dropped by the timer; session and feature deletion drop them at once (`forget_organize_session` / `_feature`).

**Delivery** (prompt), deliberately the follow-up path:
- idle at its prompt (running, not `session_working`, not waiting on the user) with nothing queued and no §19.3
  pending prompt → pasted, Enter scheduled through §19.3's delayed-Enter map: `delivered`;
- working, or something already queued → `queue_followup`, sent at its turn end by the Stop hook: `queued` (the
  follow-up id is remembered);
- the previous fire's follow-up still in the queue → `skipped: previous run still queued` (never stacks);
- blocked on the user — a pending permission (P6), a question attention record that applies, a §19.2 roster wait,
  or a held Vibeless edit (a pending input that is not a session wait) → `skipped: waiting on you` is recorded once
  and the fire is *deferred*: retried every 30 s without new history lines;
  it delivers (`… (after waiting on you)`) as soon as it can, and gives up (`gave up`) when the next slot comes due or,
  for a one-shot, after an hour;
- stopped / exited → `skipped: not running`, or with `start_if_stopped` and the resource gate open
  (`check_start_preconditions`; never a dialog from a timer): the feature is started for that session or the session
  relaunched resuming its conversation, and the prompt becomes a §19.3 pending prompt with send, delivered when the
  agent reports ready: `started` (gate closed: `skipped: resource limits`);
- a background Claude session (§19.7) → §19.5 `send_session_message`: inbox (`delivered (inbox)`) or queued.
This is deliberately narrower than §19.8's `waiting_on_user`, which also counts the generic "Agent finished and is
waiting for input" pending input that `sync.rs` leaves after every turn end and a harness's undifferentiated
`Waiting` attention (Codex, Cursor): both mean the agent is at its prompt, which is when a scheduled prompt should go
in. Counting them made every fire after the first one skip (found by the e2e). Shell: the feature must be running (else skipped or started as above); the terminal window is recreated if it is
gone; the command is typed (`run_shell_command`) only when the pane is a plain shell with no children: `ran`;
otherwise `skipped: busy (<command>)`. Failures record `failed: <why>` and raise a toast. Skips do not count towards
`max_runs`; a one-shot is finished after its fire.

**Surface.** `t` on the dashboard opens the list for the selected session (its pinned schedules and its feature's), the
feature, or the project (all features, read-only for `n`): rows show ⏲ / ⏸ / ✓, the rule in words, the target, the
text, the next run, the last outcome, and `waiting on you` while deferred; `n` new, `e`/`Enter` edit, `Space`/`p`
pause/resume (a resumed schedule counts from now — no catch-up burst), `r` run now, `d` then `y` delete, `h` history.
The editor (`Tab`/arrows between fields, `←→`/`Space` on choices, `Alt+Enter` for a line break) previews the rule
(`every 30 min · first 14:30`) or the error as you type. Rows with an active schedule show `⏲`; the info block's last
line gains `⏲ next 14:30 · every 30 min` (or `⏲ 2 schedules · next …`).

**aiw.json.** `schedules: [{name, match, when, prompt | command, session | terminal, start_if_stopped, catch_up,
end, max_runs}]` in a project's file or the global extension block (merged by name, project wins). Each entry is
materialized per feature whose name or branch matches `match` (`*`/`?`), id `cfg:<feature>:<name>`; resyncs keep runs,
`last_fire` and a dashboard pause, pick up changed fields (a changed rule counts from now), and drop entries that went
away. One-shots are refused in the file. Invalid entries are logged once. Edit and delete are refused from the
dashboard and automation ("defined in aiw.json").

**Automation.** `aiw automation schedule add|list|pause|resume|delete|run|history` (IPC action `schedule` with an
`op`); `--session` takes the §19.5 targets, `--feature` an id, name, branch or `project/feature`; ids accept a unique
prefix. Config: `schedules` (true; off fires nothing), `schedule_min_interval_secs` (60).

**Tests.** `fork::schedules` (21: every/daily/cron/at parsing and round trips, the cron subset and its refusals,
normalizing a bare time and refusing the past, DST across both changes, a skipped time, a repeated time, month ends,
Feb 29, weekdays, the OR rule, steps, missed vs due, pause/limits/end, one-shot finishing, outcomes, aiw.json
validation and matching, globbing, end times, relative labels), `db::schedules` (round trip and update, delete with
history, history cap), `app::tests::fork_schedules` (20 against `MockTmuxOps`: idle delivered once per fire and the
next slot; working queued and never stacked; queued behind existing follow-ups; waiting-on-you deferral, quiet retries
and delivery after; giving up at the next slot; not running vs `start_if_stopped` resuming `conv-1` with a pending
prompt; catch-up none vs once (once, not per missed slot); a shell schedule opening its terminal, keeping the
selection and reusing it; a busy terminal; a stopped feature; pause and resume-from-now; max runs and a past one-shot;
save validation; deletion and orphan cleanup; aiw.json materialize / resync keeping the pause / removal; the dialog
add and edit; the project view; automation add/list/pause/run/history/delete and errors; a failed paste),
`handlers::fork_schedules` (editor typing, fields, toggles, save, Esc; list pause, history, delete confirm). E2E
(`scripts/dev/e2e/schedules.sh`, aiw restarted with a 5 s shortest interval): an `every 6s` prompt reaches the idle
fake claude once per fire and again at the next interval, pause stops it, the `t` list shows it and Space/`h` work,
an open question (`/ask`, the Notification hook) makes the fire skip and the retry delivers after it is answered in
the pane, a terminal opened just now is not mistaken for busy (tmux reports itself until the shell starts), run now delivers at once, a shell
schedule opens `sched-term` and its output appears, a terminal running `sleep` is skipped as busy, and automation
list / history / delete.

### 19.10 Landing a feature (proposal 2)

**What.** `P` on a feature (or one of its sessions) runs the end of a feature's life as one guarded step:
preflight → update onto the base → check → finish → cleanup. `src/fork/land.rs` is pure plus the worker (git and `gh`
only through `std::process`, nothing linked); `src/app/fork_land.rs` owns the dialog, the threads, the row state and
the cleanup; `src/handlers/fork_land.rs` and `src/ui/dialogs/fork_land.rs` are thin.

**Preflight** (`compute_preflight`, on a helper thread; the dialog shows "Checking…" until it answers). Base:
`land.base` → `<remote>/HEAD` → the first existing local `main` / `master` / `trunk` / `develop` → the main
checkout's branch. The worktree's real branch (`symbolic-ref HEAD`) wins over the stored one, with a warning. Ahead /
behind against the upstream ref (`<remote>/<base>` for a `pr` finish when the remote has it, else the local base) as of
the last fetch. Blockers: repo-dir feature, detached HEAD, no base, base == branch, base missing locally, `pr` without
the remote, a rebase or merge in progress (`a` aborts), uncommitted or untracked files (`t` opens a terminal),
nothing to land, a dirty checkout of the base for `local_merge` (tracked files only; untracked ones are fine), another
land in the same repository (in this process). Warnings: behind count, the remote's base ahead of the local one for a
local merge, cleanup that will remove the worktree, working agents (`Enter` then needs `y`), agents waiting on you,
and the configured hooks. `plan_commands` lists exactly what will run.

**Worker** (`land::run`, its own thread; `Event::Stage` / `Event::Finished` on a channel, polled by `land_tick` in the
main loop). Takes `RepoLock` (`flock` on `<state>/land/<sha256(repo)>.lock`, non-blocking: a second land in the
repository is refused, not queued — one in another aiw instance is caught too). Then `pre_land` (in the worktree,
non-zero stops), `git fetch --no-tags <remote> <base>` when the remote exists (fatal only for `pr`; a local merge goes on
onto the local base), the update (`rebase <sha>` / `merge --no-edit <sha>` against the upstream's sha read once —
`expected_base`; skipped when the branch already contains it), the check (`sh -c` in the worktree with the feature's
port block and `AIW_LAND_*`; a non-zero exit or tracked files modified afterwards stop), the finish, and `post_land`
(in the repository, either way, with `AIW_LAND_RESULT` / `_REASON` / `_PR_URL`; its failure is logged, the result
stands). Git runs with `GIT_TERMINAL_PROMPT=0`, `LC_ALL=C` and, unless the user set one, `GIT_SSH_COMMAND="ssh -o
BatchMode=yes"`; fetch, push and `gh` have a 180 s limit. The worker never panics; every failure is an `Outcome` with a
`FailureKind`, and a conflicted rebase or merge is left exactly as git left it (`in_progress` says which).

**Finish.** `local_merge`: require `expected_base` to be an ancestor of HEAD; where the base is checked out
(`git worktree list --porcelain`), refuse a dirty checkout, require its HEAD to still be `expected_base`, then `merge
--ff-only` there so the files follow; where it is checked out nowhere, `update-ref refs/heads/<base> <head>
<expected_base>` so a moved base fails atomically. `pr`: `git push -u <remote> <branch>` (never forced; a rejected push
says why), then `gh pr create --base --head` with `--fill` or the `pr_title` / `pr_body` templates (`{feature}`,
`{branch}`, `{base}`, `{summary}` = first line of `.claude/latest-prompt.txt`), `--draft` optional; "already exists"
plus a URL is `pr_existed`, never a second PR, never a merge. `none`: update and check only.

**Strategy defaults** (`resolve_strategy`). `finish`: `pr` when the remote is on GitHub and `gh auth status` succeeds
(probed only when `finish` is unset), else `local_merge`; `delete_branch` / `remove_worktree`: true for `local_merge`,
false otherwise; `mark_done`: true; `update`: rebase; `remote`: origin. `land` lives in `ExtensionConfig`: a project's
`aiw.json` over the global `extension` block in `config.json`, field by field (`LandConfig::merged_over`). Lifecycle
hooks gained `pre_land` / `post_land`, merged like the others and gated by the existing one-time approval
(`HookName::PreLand` / `PostLand`, `HookResume::Land` replays the confirm); automation refuses an unapproved repository
hook (`approved_repo_hook_or_refuse`) rather than skipping a gate.

**Seeing it.** The worker writes every step and command output to `<state>/land/<feature-id>.log`. When the feature
runs, a `land` terminal session (created once, selection kept) runs `sh <state>/land/<id>.tail.sh`: `tail -n +1 -F` on
the log plus a watcher that stops it once the end marker is written, so the output stays and the shell comes back by
itself (a script, not a pipeline typed into the user's shell, which may be fish; `tail --pid` is not portable). An
earlier version sent `C-c` when the land ended; a land that finished in 0.1 s (the full e2e run) interrupted the tail
before the shell had started it. The feature row gets a line under it (counted in `workspace_item_height`):
`⇡ landing · <stage>` while it runs, `⛔ land failed: <reason> (P)` after a failure until the next success or `x`;
`feature_badge` rolls both up (Working / NeedsYou), so the header pills count a failed land. The dialog shows the
preflight, the running stage and log tail, or the last failure (with `a aborts the rebase`) or success.

**Cleanup** (app side, after success, only once `mode` is Normal so the user sees the result): `remove_worktree` →
`AppMode::DeletingFeature` + `delete_feature()`, the normal delete flow (TODO disposition, sessions, worktree, ports);
`delete_branch` → `git branch -d <branch>` once the feature is gone (up to 5 min, else kept and logged); a kept
feature with `mark_done` gets `ready`; `delete_branch` with a kept worktree is reported as kept. A `pr` success refreshes
PR badges (`sync_active_prs_background`).

**Automation.** `aiw automation land --feature … [--project] [--dry-run] [--yes] [--no-wait] [--status] [--abort]`
(IPC action `land`, ops `dry_run` / `start` / `status` / `abort`); the CLI polls `status` until the land ends.
`LastLand` keeps the feature's name and project so `status` answers after the cleanup removed the feature.

**Tests.** `fork::land` (25, real git in temp repos: the tail script returning at the end marker, config merge and JSON, strategy defaults, gh output parsing,
worktree porcelain, templates and quoting, preflight base / ahead / behind / plan, the blockers, a dirty base checkout,
local merge into a checked-out base and through update-ref, a base moved during the land both ways, a failing check,
a check that dirties the tree, rebase and merge conflicts left in progress and aborted, the repo lock, hooks with their
env and a failing `pre_land`, finish none, `pr` with a local bare remote and a fake `gh` including an existing PR, base
resolution order, log and lock paths). `app::tests::fork_land` (12: the dialog and preflight, project rows, a local
merge end to end with the land terminal, the base, the removal through the delete flow, the branch deletion and
automation status afterwards; a failing check on the row and its dismissal; the working-agents confirm; aborting a
conflicted rebase; a dirty worktree; one land per repository; automation dry run / start / status and the `--yes`
rule; an unapproved repository hook gated in the dialog and refused in automation; the landing line). E2E
(`scripts/dev/e2e/land.sh`, real git in sandbox repos, no network): a dashboard land with a moved base and a streamed
check, then removal of the feature, worktree and branch; a failing check and its ⛔ row; a rebase conflict aborted with
`a`; a dirty worktree refused; a base moved by the check itself; a `trunk` repository; a `pr` finish to a local bare
remote with a fake `gh`, then the existing-PR path.

Found while building it: after `y` confirmed a land with agents working, the dialog stayed in its confirm state, so
the first `Esc` only cleared it — the confirm now resets when the land starts. And the `C-c` above. And `status` by
name failed once the cleanup had removed the feature — `LastLand` now keeps the name.

### 19.11 Testing it together, and against the real harnesses (2026-10-12)

**Multi-project e2e.** `scripts/dev/e2e/multi-project.sh` runs every feature above in one aiw across six
repositories: a Rust crate (land check `cargo check --offline`), a Node package (`npm test`), a Python package
(a check that fails on purpose), a two-package monorepo with a layout preset, a repository whose default branch is
`trunk`, and one with a local bare remote and a fake `gh` for `finish: pr`. It checks the features against each
other, not one at a time: landing one fan-out sibling while its twin keeps running, deleting a group that holds a
fan-out pair, a schedule aimed at a forked session (dropped when that feature lands), a message to a background
session, worktree hooks in two repositories at once, and the watchdog leaving a session that waits on you alone.
108 checks, all passing.

Two bugs it found: a failed land's needs-you row borrowed the feature's strongest badge, so a watchdog flag on the
same feature labelled both rows; and a deleted (or landed) feature left its harness's "stop" / "waiting for
input" requests in `i` for a feature that no longer exists. Both fixed, each with a regression test.

**Real-harness smoke run.** Claude Code 2.1.296 (Haiku 5.5) and Codex 0.162.1, aiw on sandboxed `XDG_*` with the
real `HOME` for harness auth and a sandboxed `CODEX_HOME`. Spend: about 640k cache-read, 79k cache-write and 4k
output tokens on Haiku (≈ $0.02 at list price) and ≈ 50k input tokens (41k cached) on Codex. Verified against the
real binaries: launch with `--name` and Claude's roster showing it; working / done / permission badges from the
hooks; a permission prompt allowed from the dashboard (`a`); `--resume … --fork-session` (a new conversation that
kept the context); a message into a real Claude inbox (Claude answered it; the §19.5 wire format holds); Claude's
`EnterWorktree` / `ExitWorktree` going through aiw's `WorktreeCreate` / `WorktreeRemove` hooks into aiw's layout;
`claude --bg` output parsing, the roster state, a held Vibeless edit approved from the dashboard, attach,
`claude stop` and `claude rm`; `bgIsolation: none` (the edit landed in the feature checkout — which as a linked
worktree is also skipped by Claude's own rule); Codex's live working badge; a `local_merge` land of a commit a
real agent made.

Bugs it found, all fixed with regression tests:

- **Inherited session identity.** Started from inside a Claude Code session, aiw handed that session's
  `CLAUDE_CODE_SESSION_ID`, inbox socket and token, `CLAUDE_CODE_CHILD_SESSION` and job directory to every agent it
  launched: the agents showed the parent's name and counted as its children. `fork::launch::INHERITED_SESSION_VARS`
  are now unset (`env -u`) in every launch environment.
- **Vibeless did not ask.** With no flag, current Claude Code starts in auto mode, whose classifier ran a shell
  command with no prompt. Vibeless now passes `--permission-mode default`.
- **Codex died in every new worktree.** aiw passed `--add-dir` with no sandbox flag; in a checkout Codex has not
  been told to trust, the sandbox is read-only and Codex 0.162 refuses the extra writable root and exits. Vibe now
  passes `--sandbox workspace-write --ask-for-approval on-request`.
- **Codex ignored aiw's hooks in worktrees.** Codex 0.162 reads project hooks only from the repository root, never
  a linked worktree's `.codex/` (`/hooks` listed 0 installed). aiw now also merges them into the main checkout's
  `.codex/hooks.json` (git-excluded, user entries kept); one trust per repository.
- **Input requests on the wrong row.** A background session's held edit was drawn on its sibling Claude rows and
  not on its own, because pending inputs were matched per feature. `PendingInput` now carries the raising
  `aiw_feature_session_id`; an owned request lands on that session's row (feature-wide signals still light every
  agent row).

**Real vs fake differences** (the fakes were not changed to hide these): a real `claude attach` is left with `←`
(Claude opens its own agent view) and then `Esc`, not a `/detach` command; a stopped background session drops out
of `claude agents --json` (it is still listed with `--all`, as `done`), so the row reads "not running · c resumes"
rather than "stopped"; Codex warns that `-c` overrides force its embedded mode (harmless); Claude's prompt bar
shows the session's `--name`. Not exercised for real: `StopFailure` (no cheap way to provoke an API error) and
Claude subagents with `isolation: worktree` (`EnterWorktree` covered the same hook contract).

**Lessons for the next real run.** Create `$XDG_CONFIG_HOME/ai_workstation` before starting aiw: with only
`XDG_CONFIG_HOME` set, aiw falls back to an existing `~/.config/ai_workstation` (the first attempt here opened the
real config for about a minute and left 11 empty tables in the real `aiw.db`, a `hook-event.sh` and a bumped hook
stamp; the last two were restored). aiw's Claude hook scripts always live under `$HOME/.config/ai_workstation/hooks`,
so back that folder up and restore it afterwards. Accepting Claude's bypass-permissions warning writes
`skipDangerousModePermissionPrompt` into `~/.claude/settings.json`; remove it after a SuperVibe test.

