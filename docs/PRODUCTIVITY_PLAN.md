# Productivity plan

Goal: make running 10–30 agents across several projects feel calm. You should know what each agent did without
opening it, answer and steer many agents in one pass, review in small steps, and ship without leaving AIW.

Sources (October 2026):
- an inventory of what AIW already has;
- the integration points of each agent CLI (Claude Code, Codex, Gemini, Cursor, Pi);
- a survey of comparable tools: t3code, Conductor, Superset, Vibe Kanban, agent-deck, Claude Squad, Sculptor, Jean,
  the Codex app, Cursor 3 and Claude Code's agent view.
  Code may be borrowed only from MIT or Apache tools (t3code, agent-deck, Sculptor, Nimbalyst, Vibe Kanban, Jean). The
  others give ideas only (Claude Squad AGPL, cmux GPL, Superset ELv2, closed apps).

## Rules every item keeps

- **No network from AIW.** Agents talk to their own providers. `gh` runs only on an explicit action or refresh.
  Linear is never polled.
- **The task prompt is never sent automatically.** Queued follow-ups and broadcasts (P5, P7) send only text you
  typed and confirmed.
- **No global agent config.** Everything goes into per-worktree files (`.claude/settings.local.json`, `.codex/`,
  `.gemini/settings.json`, `.cursor/hooks.json`) or launch flags (`--settings`, `-c`).
- **Frame budget:** ≤ 2 ms per dashboard frame and no tmux calls while drawing. The test in `src/ui/fork_bench.rs`
  guards this.
- **Fork conventions:** new code goes in `src/fork/` or `src/app/fork_*.rs`, and `// FORK:` marks every upstream
  touch point.
- **Keys:** every key named here is a proposal and is checked against the existing bindings (`?`) when it is built.

## What AIW already covers (don't rebuild)

- **Starting work:** features from presets, batch create `B`, fork `F`, handoff (leader `F`).
- **Lists and inputs:** TODOs `I`, needs-you queue `i`, dormant list `z`, prompt library `L`.
- **Review and PRs:** diff viewer (leader `d`), supervised edits (Claude), PR badges `R`.
- **Running things:** resource gate, `v` tiles via `aiw grid` in the cockpit's agent pane (DESIGN.md §18), ports, worktree prep, lifecycle hooks, custom
  run sessions.

## Gaps this plan closes

- AIW tells you nothing outside its own window.
- Rows say "working" but not *what* the agent did.
- To answer a permission prompt you must open the agent.
- You can't queue the next instruction, or send one to many agents.
- There is no per-turn undo for Codex, Cursor or Pi.
- There is no command palette, no search across conversations, and no switcher across features.
- There is no PR create or merge.
- The Claude 5-hour and weekly meters went blank when the OAuth usage call was removed.

---

## Phase 1: know without looking (small items, do first)

### P1 Desktop and terminal notifications
- **What:** a local notification and an optional bell when an agent needs you or finishes. Off, needs-you only, or
  all. Several events within 3 s merge into one notification. Nothing fires for the session you're looking at.
- **How:**
  - Add `src/fork/notify.rs` with a `Notifier` trait (mockable in tests). Implementations:
    - macOS: `osascript -e 'display notification … with title "AIW"'`
    - Linux: `notify-send -a AIW`, when it is on `PATH`
    - fallback: BEL, or OSC 9 / OSC 777 written to the outer terminal. Ghostty, iTerm2, WezTerm and Kitty show these
      as native notifications.
  - Call it where attention is recorded: `record_attention` in `src/app/notifications.rs`, reached from
    `handle_ipc_message_value`.
  - Config: `AppConfig.notifications: { mode, bell, sound }` with `#[serde(default)]`.
- **Effort:** S. **Tests:** coalescing, the "focused session is quiet" rule, and the mode switch.

### P2 One-line "what it did" summaries, at no cost
- **What:** every session row and needs-you row shows the agent's last message in one line, for example
  `✓ codex  Added retry to webhook sender; 3 tests pass`. No extra model calls.
- **How:**
  - Claude and Codex Stop hooks send `last_assistant_message`; Gemini's AfterAgent sends `prompt_response`.
  - Extend `scripts/notify.sh` and `src/hook_payload.rs` to forward the first 200 characters.
  - Store them in a new `FeatureSession.last_summary` field (`#[serde(default)]`, so old data loads) and draw them
    dimmed in `src/ui/list.rs` and the attention picker.
  - Cursor: read the transcript tail from `transcript_path`.
  - Pi: no hooks yet (see P15).
- **Effort:** S.

### P3 Needs-you rows say why
- **What:** each waiting row shows the reason (`permission`, `question` or `finished`) and its subject, such as
  `Bash: git push origin feat-x`.
- **How:**
  - Claude: register `PermissionRequest` and read `notification_type`.
  - Codex: `PermissionRequest`.
  - Gemini: `Notification` (`ToolPermission`).
  - The fields go into `AttentionRecord` (`src/app/attention.rs`) and are rendered in the picker.
- **Effort:** S.

### P4 Usage meters restored, locally — **done 2026-10-10** (DESIGN.md §16): Claude's status line via `--settings` (context %, cost, 5h/7d limits), Codex rollouts, Gemini chat files; shown on rows, badges, tiles and the GUI contract. Verified: `--settings` hooks merge with the worktree's (hooks.md), so the status line and the hooks both ride on the flag.
- **What:** context %, cost per session, and the Claude 5-hour and weekly limits in the header and sidebar, as before
  the OAuth call was removed, without any network.
- **How:**
  - Launch Claude with `--settings <json>` that adds a `statusLine` command. That command is a script AIW writes to
    its hooks folder. Claude passes it JSON (`context_window.used_percentage`, `cost.total_cost_usd`,
    `rate_limits.five_hour` / `seven_day`); the script sends it to AIW's socket.
  - If the user already has a status line (read-only lookup in their settings), the script runs it and prints its
    output, so their line doesn't change.
  - Codex: cumulative `token_count` events from the rollout file, read with the incremental reader in
    `src/fork/line_fold.rs`.
  - Feed `src/usage.rs` / `src/token_tracking.rs`.
  - Warn at 80% of a limit; at 100%, the resource gate asks before starting another Claude agent.
- **Effort:** M.
- **Verify first:** whether `--settings` replaces the worktree's `settings.local.json` hooks or merges with them.
  If it replaces them, put `statusLine` in `settings.local.json`, which AIW already writes.

### P5 Command palette and a global switcher
- **What:**
  - `Ctrl+P` (or `:`) opens a fuzzy palette over every AIW action, built from the same tables as the help overlay,
    so the two can't drift apart.
  - Typing `@` jumps to any session in any project, which replaces the switcher that only covers one feature.
- **How:**
  - Make one action registry out of the key tables in `src/ui/dialogs/help.rs`, each entry with its handler.
  - Add `AppMode::Palette` with these touch points: `src/app/state.rs`, `src/handlers/mod.rs`,
    `src/ui/dashboard.rs`, `src/ui/status.rs`, `src/handlers/mouse.rs`.
  - Matching: a small built-in subsequence scorer, with no new dependency.
- **Effort:** M.

### P2b Obsidian log of what every agent did — not built (dropped on 2026-10-10 to keep the app stable first; the hooks it would use, `PreCompact` / `SessionEnd`, are not wired)
- **What:** every summary from P2 is also appended to a Markdown note in your Obsidian vault, which shows it live.
  No plugin, no MCP, no server: a vault is a folder of Markdown files.
- **How:**
  - Config: `obsidian_vault: "~/Notes"` (off when unset) and `obsidian_layout: daily | feature | both`.
  - `src/fork/obsidian.rs` formats one line per event and appends it:
    - daily: `AIW/2026-10-10.md`, a `## project / feature` heading per feature, then
      `- 09:41 **claude** ✓ summary (+61 −3)`;
    - feature: `AIW/<project>/<feature>.md` with YAML properties (`tags: [aiw]`, project, branch, agents) and the
      same lines, so notes can `[[link]]` to tickets.
  - Writes are append-only and atomic (write a temp file, then rename); AIW never rewrites your notes.
  - Called from the same place as P2 (`record_attention` in `src/app/notifications.rs`).
  - When an agent should *read* the vault for context, that is a separate, optional per-worktree MCP server in
    `.mcp.json` / `.codex/config.toml`, never global.
- **Effort:** S. **Tests:** formatting, append-only behaviour, missing vault path, `~` expansion.

## Phase 2: steer many agents in one pass

### P6 Approve or deny from the dashboard — **done 2026-10-10** (DESIGN.md §16): keys are `a` allow / `X` deny (`y` is "mark ready"); Claude + Codex `PermissionRequest`, Cursor `beforeShellExecution` / `beforeMCPExecution`; the hook waits on one IPC connection. Gemini stays a `⛔` answered in the pane (its hooks cannot grant).
- **What:** in the needs-you list, `y` approves, `n` denies, and `Enter` opens the agent as today.
- **How:**
  - A `PermissionRequest` hook (Claude, Codex) or a `beforeShellExecution` hook (Cursor) sends the request and waits
    on AIW's reply socket for up to 10 minutes.
  - It then returns `decision.behavior` (Claude, Codex) or `permission` (Cursor).
  - If the wait times out or AIW is closed, the hook prints nothing, so the agent's own prompt appears: nothing can
    get stuck.
  - Reuse the reply-socket and claim-lock code from supervised edits (`src/app/supervised_edits.rs`, `src/ipc.rs`).
  - Gemini's hook can only observe, so `y` / `n` send the matching keys to the pane instead.
- **Effort:** M.

### P7 Queued follow-ups — **done 2026-10-10** (DESIGN.md §16): `e` on the row, `aiw automation queue`; `aiw_followups` table; delivered by the Stop hooks (Claude/Codex `Stop`, Gemini `AfterAgent`, Cursor `stop` with `loop_limit` 20). Gemini is delivered through its hook, not `send-keys`; Pi has none.
- **What:** write the next instruction for a busy agent now (leader `q`). It's delivered the moment that agent
  finishes. The row shows `⏵2` for two queued messages; you can edit or drop them before they're sent.
- **How:**
  - New `followups` table (session, position, text, created).
  - Delivery without racing the agent's input box:
    - Claude and Codex: the Stop hook returns `{"decision":"block","reason":<text>}`, which becomes the next turn.
      Check `stop_hook_active` to avoid loops.
    - Cursor: the stop hook's `followup_message`.
    - Gemini and Pi: `tmux send-keys -l`, then Enter, once the session is idle.
- **Effort:** M.

### P8 Broadcast to many agents
- **What:** mark sessions with `Space` in the sidebar, press `b`, type once (for example "rebase on main
  and rerun tests") and confirm. It's queued for each marked agent through P7, so busy agents get it when they
  finish.
- **Effort:** S once P7 exists.

### P9 Batch create with a shared task, pasted but not sent
- **What:** `B` takes one task description. Each new agent opens with it pasted into the input but **not sent**,
  as the spec requires. Pressing Enter in each one sends it.
- **How:** add a `task` field to `CreateBatchFeaturesState`. After launch, a `tmux send-keys -l` with no Enter,
  once the agent shows its prompt.
- **Effort:** S.

## Phase 3: review in small steps

### P10 Checkpoint every turn, for every agent
- **What:**
  - Every prompt you send creates a checkpoint.
  - The diff viewer gets a scope switch: last turn, whole branch, or uncommitted.
  - Leader `u` restores the worktree to before the last turn (after a confirmation).
  - This works for all five agents. It also covers Bash edits, which Claude's own `/rewind` misses.
- **How:**
  - On the prompt-submitted hook (Claude and Codex `UserPromptSubmit`, Gemini `BeforeAgent`, Cursor
    `beforeSubmitPrompt`):
    ```
    GIT_INDEX_FILE=$(mktemp) git add -A
    git write-tree
    git commit-tree
    git update-ref refs/aiw/cp/<session>/<n>
    ```
    (t3code's `CheckpointStore.ts` uses the same method; it's MIT, so it's a safe reference.)
  - Diff two refs for the last-turn scope.
  - Restore with `git read-tree -u --reset <ref>` and remove files the checkpoint didn't have.
  - The refs never touch the branch, the index or the stash. They're deleted with the feature, and only the last 50
    per session are kept.
- **Effort:** M.

### P11 Comment on lines, send them as one prompt, and revert a hunk
- **What:**
  - In the diff viewer, `c` adds a comment on a line.
  - `Ctrl+Enter` sends every comment to the agent as one prompt (`file:line — comment`, …), through P7 if the
    agent is busy.
  - `R` reverts the selected hunk.
- **How:**
  - Hunk revert: `git apply -R` on that hunk.
- **Effort:** M.

### P12 Run checks before an agent says "done" (per project, opt-in)
- **What:** when an agent stops, run a per-project check command. If it fails, the failures go back to
  the agent automatically, at most 2 times. Then it's marked finished with ✓ or ✗ on its row.
- **How:**
  - Claude and Codex: a Stop hook that blocks with the failure text.
  - Gemini: AfterAgent deny.
  - Cursor: `followup_message`.
  - Keep a counter per session to stop loops.
- **Effort:** S–M.

## Phase 4: ship from AIW

### P13 Create and merge a PR, and see whether it's ready
- **What:** for one feature:
  - `p` creates a PR with `gh pr create`, titled in your commit style (`feat: …`, one line).
  - `M` merges it with `gh pr merge` after a confirmation.
  - A readiness line shows CI, review decision, mergeability and open TODOs.
  - After a merge, AIW offers to archive the feature, removing its worktree and keeping its branch.
- **How:** run `gh pr view --json statusCheckRollup,reviewDecision,mergeStateStatus` only when you press `R`, open
  the feature, or right after a push. It's never polled in the background.
- **Effort:** M.

### P14 Compare attempts (G8 core)
- **What:** for features that share a task, show one column per attempt: status, +/−, files, checks (P12), time, cost
  (P4) and summary (P2). Pick one, which opens P13; discard the others.
- **Effort:** M, after P2, P4 and P12. It is also G8's first step.

## Phase 5: find and recover

### P15 Status for Pi
- **What:** Pi has no shell hooks, so AIW writes a small extension, `.pi/extensions/aiw.ts`, into the worktree.
  - Its `turn_end`, `agent_settled` and `tool_call` events call `aiw notify`.
  - Pi gets status, summaries (P2), queued follow-ups through `pi.sendUserMessage` (P7) and checkpoints (P10).
- **Effort:** S–M.

### P16 Search every conversation
- **What:** `Ctrl+F` searches the transcripts of all agents in all projects ("who touched the auth middleware?").
  `Enter` jumps to the session or resumes it.
- **How:**
  - SQLite FTS5 (rusqlite `bundled` already includes it) in `aiw.db`.
  - Indexing on the 5 s tick, in the background, by byte offset, using `src/fork/line_fold.rs`.
  - Transcripts: Claude `~/.claude/projects/…/*.jsonl`, Codex `~/.codex/sessions/…`, Gemini
    `~/.gemini/tmp/…/chats`, Pi `~/.pi/agent/sessions/…`. They are only read, never written.
  - agent-deck's `internal/recall` (MIT) is a reference.
- **Effort:** M–L.

### P17 Come back from a crash
- **What:** if an agent's process dies, its row shows `✗ crashed · S resume`, and an optional auto-resume restarts it
  once.
- **How:**
  - Start Claude with `--session-id <uuid>` (Pi with `--session-id`) so the id is known before launch. For Codex and
    Gemini, take the id from the SessionStart hook.
  - A tmux `pane-died` hook on AIW's server marks the session, which `remain-on-exit` already keeps.
  - Resume reuses `S`.
- **Effort:** S–M.

### P18 Hand a task to a different agent
- **What:** the existing handoff (leader `F`) gets an agent picker. For example, Claude reaches its limit and the
  task moves to Codex with the diff, the original request, recent turns and the changed files.
- **How:** extend `src/app/handoff.rs`. A same-agent fork uses `claude --resume <id> --fork-session` or `codex fork`.
- **Effort:** S.

## Also done 2026-10-11 (the cockpit, DESIGN.md §18)

- The dashboard is drawn by tmux: sidebar left, the selected agent's real pane right; typing is native,
  `Ctrl+Q` flips focus, dialogs zoom. The in-app view, the composer, the tree layout, the in-app split
  tiles and the attach hand-over are gone (−20k lines). Prompts AIW prepares are pasted into the agent's pane
  unsent (P9's "pasted but not sent" is now how every seed lands). A session info block under the sidebar reads
  the harness's own transcript, status line and rollout. The dashboard's `Ctrl+Space` menu is the home of the
  view's surviving commands.

## Also done 2026-10-10 (hook-driven, DESIGN.md §16)

- **Subagent count** on the working badge (`working 6m · 2 subagents`) from the Claude, Codex and Cursor subagent hooks.
- **Launch context in the system prompt** (project, feature, branch, worktree, AIW session id, "mention the branch name in commit messages and PR titles") for Claude (`--append-system-prompt`), Codex (`developer_instructions`), Gemini and Cursor (session-start hooks). The "shared context" item below is thereby partly covered.
- **Claude hooks and status line via `--settings`** (`claude_hooks_via_settings_flag`, default on): AIW no longer writes hooks into the repo's `.claude/settings.local.json`.

## Later or maybe
- **G7** many-projects dashboard: tags, colours, `0`–`9` scope.
- **Shared context:** one `AGENTS.md` per repo, which every CLI reads. Gemini needs
  `context.fileName: ["AGENTS.md","GEMINI.md"]` in the worktree settings. Per-agent facts go through SessionStart
  `additionalContext`.
- **Discovering dev-server ports:** map `ss -ltnp` / `lsof` to process ids, then up the process tree to the tmux pane.
- **Shared MCP server pool:** 30 agents × N MCP servers use a lot of memory (agent-deck `mcppool`).
- **Scheduled headless runs** feeding the needs-you list.
- **Driving agents over a protocol:** Codex `app-server`, Gemini/Cursor ACP, Pi RPC.

## Things to know about the agents
- **Gemini CLI:** since 18 June 2026, free and individual Google accounts are moved to the closed-source
  Antigravity CLI. Gemini CLI remains for paid API keys and Code Assist Standard/Enterprise. Check which one your
  work account has before investing more in Gemini.
- **Cursor CLI hooks:** reported not to fire on Linux (build 2026.08.11), while the same build works on macOS. On
  Linux, AIW should fall back to watching the screen.
- **Gemini hooks can't answer permission prompts.** P6 sends keys instead.
- **Codex** trusts project hooks by their hash. The first run in a worktree may ask you to trust AIW's hooks once
  (`/hooks`).

## Suggested order

| Sprint | Items | Why first |
|---|---|---|
| 1 | P1, P2, P2b, P3, P4, P9 | Small. You see what every agent did and needs without opening it, and the usage meters come back |
| 2 | P6, P7, P8, P5 | Answer and steer many agents in one pass |
| 3 | P10, P11, P12, P17 | Review per turn, safe undo, fewer broken "done"s |
| 4 | P13, P14, P15, P16, P18 | Ship from AIW, compare attempts, full Pi support, search |

Each item lands as its own `feat:` commit, with tests, the frame-budget check and `scripts/check-no-network.sh`
passing.
