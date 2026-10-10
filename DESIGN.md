# AI Workstation — design

**AI Workstation (AIW)** is our own fork of the upstream project credited in [README.md](README.md)
(MIT © Ryan Eldridge, see [LICENSE](LICENSE)). It keeps upstream's terminal dashboard and tmux-based engine and adds
what our own design work asked for (`~/ai_harness_ui/SPEC.md`, `DESIGN.md`, `TUI_DESIGN.md`). Maintenance and
cherry-picking from upstream: [FORK.md](FORK.md). "Upstream" below means that original project.

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
SuperVibe) · needs-you list `i` · dormant `z` · resource guards · plan interview · final review · PR triage and review ·
Learning mode · prompt library · prompt overrides · TODO lists · usage/cost panels · themes · config wizard ·
`doctor` / `automation` commands.

## 3. Gaps and what we add

| # | Gap upstream | What AIW adds | Priority | Status |
|---|---|---|---|---|
| G1 | One agent on screen at a time | **Split view**: 2–4 live agents tiled inside the TUI, plus `aiw grid` for an external terminal | P1 | ✅ in-TUI split (`v`) + `aiw grid`; pinning across projects ⏳ |
| G2 | No Gemini, no Cursor | **Gemini CLI** and **Cursor agent** as first-class agents | P1 | ✅ (untested live — CLIs not installed) |
| G3 | App makes its own network calls | **No-cloud build**: remove self-upgrade, phone remote control, GitHub screenshot fetching, background PR polling | P1 | ✅ (`NO_CLOUD.md`, `scripts/check-no-network.sh`) |
| G4 | Edits `~/.codex/config.toml` (and, before OpenCode was removed, `~/.config/opencode`) | per-session config only | P1 | ✅ |
| G5 | Worktrees inside the repo; no `.env` copy; port clashes; `~` not expanded; confusing branch error | work-safety fixes | P1 | ✅ |
| G6 | Basic theme list; fonts left entirely to the terminal | **Theming & fonts polish** | P1 | ✅ |
| G7 | Flat list for many projects | project tags/colours, summary counts, `0`–`9` scope | P2 | ⏳ |
| G8 | No tickets | board, start N agents on a ticket, compare, pick → PR → Linear (on demand only) | P3 | ⏳ |
| G9 | A session whose window closed on its own (`exit`, agent quit) stayed listed as running; opening it errored | closed terminals disappear; closed agents show as stopped (■) and reopen with their conversation; an agent that exits while its shell survives shows ■ "exited" (no "finished" attention) and `c`/`S` relaunch it in the same window; the single view returns to the dashboard when its window is gone | P1 | ✅ |
| G10 | Typing in an agent opened a compose box | keys go **straight to the agent**; compose box optional (`compose_box` setting, or `Ctrl+Space e` per session) | P1 | ✅ |
| G11 | Dated look | **UI revamp** within a strict performance budget (§12) | P2 | ✅ dashboard, footer, help; project colour tags ⏳ (with G7) |
| G12 | OpenCode support we don't use | **OpenCode removed** (code, plugins, themes, GUI, docs); old data still loads — see §14 | P1 | ✅ |

---

## 4. G1 — Split view (no more switching)

### 4.1 Inside the TUI

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
| Background PR badge sync | `gh` every 5 minutes for active PRs | **off**; refresh only when you open PR triage or press refresh |
| Optional syntax-parser install | `git clone` of tree-sitter grammars | keep as an explicit, user-started action, clearly labelled |
| PR triage / review / posting | `gh` | keep — only when you start it |
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
on macOS), repo files `aiw.json` / `.aiw/` / `AIW_PLAN*.md`, injected skills `aiw-*`, theme `classic`.
We no longer merge upstream; useful commits are cherry-picked by hand ([FORK.md](FORK.md)).

**Data migrated by copy.** On the first start with no `~/.config/ai_workstation`, an existing upstream setup
(`~/.config/amf`) is *copied* in (`src/fork/migrate.rs`): settings, ports, themes, parsers, review memory, and the
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
  `PromptOverrides::{all, remove}`, `BrowseScope::label`, `LearningViewState::{anchor_path, selected_qa}`,
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

With 10–30 agents the tree is a list of names, and reaching an agent means entering and leaving a view. The
workspace layout keeps the sidebar and shows the selected session live beside it, so moving between agents is
moving the cursor. `b` toggles it (saved as `dashboard_layout` in `config.json`; the tree stays the default for
this round). README "Workspace layout" has the key table.

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
╰ Tab type here  o full view  i next needing you  v tiles  n feature  c start  x stop  b tree ───╯
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
  (`compose_intercept_active`; with the compose box on for the session, typing opens it over the full view).
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

