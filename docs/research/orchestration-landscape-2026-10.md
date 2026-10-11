# Orchestration landscape and what aiw should add next

Researched 2026-10-11, after the harness-native duplicates were removed
(see DESIGN.md §18.1 and docs/PRODUCT.md §14). Scope: what comparable
agent-orchestration tools do that aiw does not, and which Claude Code and
Codex features aiw could exploit, with ten ranked proposals at the end.
Everything marked *unverified* comes from third-party write-ups rather than
the vendor's own documentation.

## Executive summary

- The field has converged on the same core aiw already has: one worktree
  and one tmux pane per agent, status badges from hooks, a dashboard, a
  needs-attention inbox. aiw is not behind on the basics.
- The capabilities aiw lacks fall into four groups: fan-out and broadcast
  (send one prompt to several agents), a merge and land step at the end of
  a feature, durable agent-to-agent messaging and scheduling, and watchdog
  style self-healing (restart, auto-compact, replay).
- Claude Code has grown a background-session supervisor (`claude agents`,
  `--bg`, `attach`, `logs`, `respawn`) with a stable JSON interface, named
  sessions, session forking, cross-session messaging over a per-session
  Unix socket, worktree hooks that let a manager place worktrees, and a
  much richer hook vocabulary (notification types, compaction, model
  switch, teammate idle, task created and completed).
- Codex has a JSON-RPC app server that exposes approvals, token usage,
  rate limits and thread fork and resume, plus a `hooks.json` that replaces
  the legacy `notify` setting aiw still uses.
- Top three proposals by impact: (1) fan-out one prompt to N agents with a
  shared group in the sidebar, which DESIGN.md §17 already designed;
  (2) a land step per feature (rebase, test, merge or PR, clean up) driven
  from the dashboard; (3) adopt Claude's named sessions, `--fork-session`
  and the `claude agents --json` roster so aiw and Claude agree on what a
  session is.

## Part A. Comparable tools

Only capabilities that aiw lacks are listed under each tool. Planning, code
review, PR review and chat-with-the-code features are omitted on purpose.

### Superset (superset.sh)

Desktop IDE that runs many agents in parallel, each in its own worktree,
with an iPhone app, CLI and MCP server. Elastic License 2.0.
Sources: https://github.com/superset-sh/superset, https://docs.superset.sh/overview

- Scheduled automations: agent sessions started on a schedule, with a real
  workspace as the output (https://docs.superset.sh/automations).
- Setup and teardown scripts per workspace, run when a worktree is created
  or removed (https://docs.superset.sh/setup-teardown-scripts). aiw has
  lifecycle hooks in aiw.json; the gap is the teardown half and per-preset
  scripts.
- Terminal presets: saved layouts of agent plus shell panes per workspace
  (https://docs.superset.sh/terminal-scripts).
- Session forking from inside the app (https://docs.superset.sh/agent-sessions).
- An MCP server so the agent itself can create workspaces and tasks
  (https://docs.superset.sh/mcp-server). Tool list not documented on the
  overview page.
- Merge through ordinary branches and PRs, resolved at merge time
  (https://docs.superset.sh/pull-requests).

### Claude Squad (smtg-ai/claude-squad)

Terminal app, one tmux session and worktree per agent, AGPL-3.0.
Source: https://github.com/smtg-ai/claude-squad

- Background mode with an experimental auto-accept flag (`-y`).
- A "review, commit and push" step before opening a PR from the TUI.

### Conductor (conductor.build)

Mac app, one worktree per Claude Code or Codex agent. Free, Apple Silicon.
Sources: https://docs.conductor.build, https://www.conductor.build/llms-full.txt

- Dispatcher that assigns tasks to agents.
- Pull Linear issues in as agent tasks (*unverified*, third-party review).
- Diff viewer and merge from the dashboard.

### Vibe Kanban (bloop, now community-maintained)

Kanban board where moving a card to In Progress creates a worktree and
starts an agent. Apache-2.0. Company shut down April 2026.
Sources: https://virtuslab.com/blog/ai/vibe-kanban/, https://aiidelist.com/ide/vibe-kanban

- Board-driven dispatch: a card is the unit, the worktree and agent follow.
- MCP server so agents can move cards themselves.
- One dev server and port pool per task attempt, with a preview.
- PR creation from the board.

### amux (mixpeek/amux)

Single Rust binary, web dashboard plus phone, tmux backend. MIT.
Source: https://github.com/mixpeek/amux

- Shared board with compare-and-swap claiming; "done" requires evidence and
  "verified" requires a peer check.
- Cron-style recurring prompts and one-shot jobs with an audited run history.
- Self-pacing loops for unattended runs.
- Origin-stamped messaging between workers; every prompt and reply is a
  ledger row.
- Model or provider switch on a running worker without losing context.
- Self-healing watchdog: auto-compact, restart a crashed session, replay
  the last message.

### NTM, Named Tmux Manager (Dicklesworthstone/ntm)

Go binary over tmux for Claude Code, Codex and Gemini.
Source: https://vibecoding.app/blog/ntm-review (*third-party*), https://github.com/Dicklesworthstone/ntm

- Broadcast: `ntm send <session> --cc "<prompt>"` sends one prompt to every
  agent of a type or to all agents.
- File reservations: agents claim files before editing and others see what
  is locked. Enforcement strictness not documented.
- Per-agent token velocity badges and a "context full" detector.

### dmux (standardagents/dmux)

Agent multiplexer pairing panes with worktrees over tmux.
Source: https://github.com/standardagents/dmux

- Merge from the pane menu: auto-commit, merge and clean up in one step;
  or push and open a PR.
- AI-generated branch names, commit messages and pane names.
- Hooks on worktree create, pre-merge and post-merge.
- Pane hide and show, per-project isolation of the view, built-in file and
  diff browser.
- Resumes the agent's conversation when a pane is recreated.

### herdr (herdrdev/herdr)

Rust runtime that owns agent terminals and exposes a socket API.
Sources: https://dev.to/pvgomes/herdr-is-tmux-for-coding-agents-3ai0, https://innfactory.ai/en/ai-harness/herdr/

- `agent wait --until done` lets a script block on an agent's state.
- `pane read --source recent --lines N` reads output over the API.
- Screen-manifest fallback for agents with no hook integration.

### Cyclops (cyclops-team/cyclops)

Durable mailbox for tmux agents with a workspace UI.
Source: https://github.com/cyclops-team/cyclops

- Fsynced append-only journal; every message goes accepted, notified,
  claimed, completed, and completion needs an explicit reply.
- Composer guard: a message is held while a human draft, modal, permission
  prompt or copy-mode is on screen. This is the same problem aiw's
  follow-up queue solves by delivering on Stop hooks.
- Sender identity from the socket, so a message cannot forge its origin.
- Layout presets (`--preset duo`).

### agent-session-manager (izll/agent-session-manager)

tmux TUI with groups, resume and fork, diff review.
Source: https://github.com/izll/agent-session-manager

- Session groups with colours, favourites, notes per session and per tab,
  reordering, search over names and notes.
- `p` sends a prompt to a running session without attaching.
- Global search across agent histories (`Ctrl+F`).
- Parallel instance of a session in the same directory.

### octomux (ShreyPaharia/octomux)

Local dashboard with a Kanban fleet view and a permission inbox.
Source: https://github.com/ShreyPaharia/octomux

- Unified permission inbox across all agents, with a pending count in the
  tab title. aiw already answers permissions from the dashboard; the gap
  is a single ordered inbox view and the count.
- Hooks as executable scripts in `~/.octomux/hooks/<event>.d/`.

### YYLO (yylo-dev/yylo)

CLI that gives each task a branch and worktree and lands them one at a time.
Source: https://github.com/yylo-dev/yylo

- `task finish` queues a clean, committed result; `merge land` integrates
  one task at a time in a private candidate with expected-old-ref
  protection, so a moved target forces recomposition.
- Conflicts stay private to their task and never block unrelated tasks.
- `yy watch await RUN_ID` and checkpoint and evidence commands.

### ccmanager (kbwo/ccmanager)

TUI without tmux, eight harnesses. MIT.
Source: https://cdn.jsdelivr.net/npm/ccmanager@4.1.18/README.md

- Copy Claude session data between worktrees to carry conversation context.
- Create, merge and delete worktrees from inside the app.
- Experimental auto-approval.

### agent-deck (asheshgoplani/agent-deck)

Go TUI over tmux.
Source: https://github.com/asheshgoplani/agent-deck

- Attach and detach MCP servers per project; share one MCP process across
  sessions through a socket pool.
- Fork conversations for parallel experiments.
- Opt-in private tmux server, which aiw already has.

### Others worth a glance

From https://github.com/andyrewlee/awesome-agent-orchestrators:
pappardelle turns a description into issue, worktree, PR and session;
Rove and openkanban are terminal Kanban boards over worktrees; termany
tags panes working, done or needs-attention; tuios holds a shared inbox;
foremerge flags collisions when agents declare intent before writing;
wit locks individual functions via Tree-sitter; gastown runs 20 to 30
agents with a merge queue. Crystal (stravu) is deprecated in favour of
Nimbalyst.

## Part B. Harness features aiw does not yet use

What aiw uses today, from the source: hooks `SessionStart`, `SessionEnd`,
`Stop`, `Notification` (matcher `permission_prompt` only),
`PermissionRequest`, `PreToolUse`, `PostToolUse`, `SubagentStart`,
`SubagentStop`, `UserPromptSubmit`, one `PreCompact` reference; the status
line for context and cost; rate limits in `usage.rs`; Codex `-C`,
`--add-dir`, `--sandbox`, `--ask-for-approval` and the legacy `notify`
setting. Not used: session naming, forking, the background supervisor,
cross-session messaging, worktree hooks, agent teams, Codex app server,
Codex `hooks.json`.

### Claude Code

| Feature | Doc | How aiw would use it |
|---|---|---|
| `--name` / `-n`, `/rename`, `--resume <name>` | https://code.claude.com/docs/en/cli-reference | Launch every session with its aiw label as the Claude name, so `claude --resume`, `/list-agents` and agent view all show aiw's names. Resume by name instead of scanning transcripts. |
| `claude agents --json [--all] [--cwd]` | https://code.claude.com/docs/en/agent-view | Stable roster with `state` (`working`, `blocked`, `done`, `failed`, `stopped`), `status`, `waitingFor`, `sessionId`, `name`, `cwd`, `pid`. Merge it into the sidebar so sessions started outside aiw appear, and use `waitingFor` as a second needs-you signal. |
| `claude --bg`, `attach`, `logs`, `stop`, `respawn`, `rm` | same | Offer "run in background" as a session kind that needs no pane until you attach; `respawn --all` after a Claude upgrade; `claude --bg --exec` for shell jobs as rows. |
| `--fork-session`, `/branch`, `/fork` | https://code.claude.com/docs/en/sessions | A dashboard key to fork the selected session into a new aiw session, same or new worktree, for "try another approach". |
| Cross-session messaging, `CLAUDE_CODE_MESSAGING_SOCKET`, `ListAgents`, `SendMessage`, `notify_when_idle` | https://code.claude.com/docs/en/cross-session-messaging | aiw can post a line into a session's inbox socket from a hook or the dashboard without typing into the pane; agents can message each other by aiw name; `crossSessionInbound` via `--settings` per launch. |
| `WorktreeCreate` and `WorktreeRemove` hooks | https://code.claude.com/docs/en/worktrees | Let Claude's own `--worktree`, subagent isolation and `/batch` create worktrees where aiw keeps them (outside the repo, with ports and includes), so the two tools stop disagreeing about worktree locations. |
| `--worktree` and `.worktreeinclude`, `worktree.baseRef` | same | Align aiw's worktree include with Claude's file; expose base-ref choice in the New Feature dialog. |
| Notification matchers `idle_prompt`, `agent_needs_input`, `agent_completed`, `quota_auto_resume_fired` and friends | https://code.claude.com/docs/en/hooks | Badge "idle, waiting for you" distinctly from "working"; surface quota pauses on the row. |
| `Stop` input `background_tasks` and `session_crons` | same | Show that a session still has background work or a `/loop` schedule even though its turn ended, so it is not marked dormant. |
| `PreCompact` / `PostCompact`, `PreModelSwitch` / `PostModelSwitch`, `CwdChanged`, `StopFailure`, `PermissionDenied`, `TaskCreated` / `TaskCompleted`, `TeammateIdle` | same | Row events: "compacted", "switched to sonnet", "cd'd into worktree", "turn failed on API error", task counts from the session's own task list, teammate idle. |
| `SessionStart` output `sessionTitle`, `initialUserMessage`, `watchPaths` | same | Set the Claude title to the feature name; deliver the first prompt from aiw as the initial message instead of a paste; have Claude watch aiw's own files. |
| Agent teams with `--teammate-mode tmux`, team config at `~/.claude/teams/<team>/config.json` with tmux pane IDs | https://code.claude.com/docs/en/agent-teams | Show teammates as child rows under the lead session and read their pane IDs for the grid. Experimental; disabled by default. |
| `--settings` inline JSON (already used), `--setting-sources`, `--autocompact`, `--effort`, `--fallback-model` | https://code.claude.com/docs/en/cli-reference | Per-feature presets for effort and auto-compact window; fallback chain in aiw.json. |
| Checkpoints and `/rewind` | https://code.claude.com/docs/en/checkpointing | No external API; only a note that "Restore code" exists so aiw does not need its own undo. |
| Status line `rate_limits.five_hour`, `seven_day`, `spend_limit` | https://code.claude.com/docs/en/statusline | aiw already shows usage; confirm it reads these fields rather than a separate source. |
| `claude auth status --json`, `claude doctor`, `claude purge` | https://code.claude.com/docs/en/cli-reference | `aiw doctor` can call these and show auth method and config dir per harness. |
| Mods (`plugins/mods`) | https://code.claude.com/docs/en/plugins/mods/overview | Out of scope for aiw's no-network stance except as an idea: a tiny mod could post the live tool-call count to aiw's socket. Settings hooks already cover this. |

### Codex

| Feature | Doc | How aiw would use it |
|---|---|---|
| `hooks.json` with `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `Stop` | https://codex.danielvaughan.com/2026/04/15/codex-cli-hooks-complete-guide-events-policy-patterns/ (*third-party*; official page at learn.chatgpt.com) | Replace the legacy `notify` payload (`AfterAgent`, `AfterToolUse`) with the same hook shape Claude uses, so one aiw hook binary serves both harnesses. |
| App server JSON-RPC: `thread/list`, `thread/resume`, `thread/fork`, `turn/start`, `turn/steer`, `turn/interrupt`, `item/commandExecution/requestApproval`, `item/fileChange/requestApproval`, `thread/tokenUsage/updated`, `account/rateLimits/read`, `codex --remote unix://PATH` | https://learn.chatgpt.com/docs/app-server | Answer Codex approvals from the dashboard with the same UI as Claude's; read context and rate limits without parsing rollouts; steer a running turn from the follow-up queue; attach the TUI to an aiw-owned app server. Experimental. |
| `codex resume [--last] [--all]`, `codex fork`, `codex archive`, named sessions, `tui.resume_cwd` | https://learn.chatgpt.com/docs/developer-commands?surface=cli | Resume by name, fork from the dashboard, archive on feature delete. |
| `[agents]` config: `max_concurrent_threads_per_session`, `default_subagent_model`; `/agent` view | https://learn.chatgpt.com/docs/agent-configuration/subagents | Cap Codex subagents per feature from aiw's resource limits; count them on the row as aiw does for Claude. |
| Worktrees default in CLI (reported 0.156, 2026-09-22) | https://codex.danielvaughan.com/2026/03/27/codex-cli-in-2026-whats-new/ (*unverified*) | Same alignment as Claude's worktree hooks once the official doc is confirmed. |

## Part C. Ten proposals, ranked by impact for three to six agents

1. **Fan-out one prompt to N agents.** Pick harnesses and count, choose
   shared worktree or one each, send the same prompt, tag them as a group in
   the sidebar with a group badge that rolls up needs-you. From NTM
   broadcast, Superset dispatch and DESIGN.md §17. Hooks into the New
   Feature dialog, worktree creation, the paste path. Size M. Risk: two
   agents in one worktree collide; default to one each.

2. **Land step per feature.** A dashboard key that runs rebase on base,
   the project's check command, then merge or push and open a PR, then
   removes the worktree and marks the feature done. One task at a time with
   expected-old-ref protection as YYLO does. From dmux, YYLO, Claude Squad.
   Hooks into aiw.json lifecycle hooks (new pre-merge and post-merge),
   the `R` PR refresh, feature delete. Size L. Risk: merge conflicts need
   a human; stop and show the pane rather than resolve.

3. **Adopt Claude's session identity.** Launch with `--name <aiw label>`,
   resume with `--resume <name>`, merge `claude agents --json` into the
   sidebar, add a fork key using `--fork-session`. Size S to M. Risk:
   version floors (names in agent view need v2.1.290).

4. **Background session kind.** A row that runs `claude --bg` under
   Claude's supervisor instead of a pane, with attach on Enter and logs in
   the info block. Sessions survive aiw and tmux restarts for free. Size M.
   Risk: two supervisors (aiw's tmux and Claude's daemon) with different
   restart rules; keep it opt-in.

5. **Richer row events from the new hooks.** Wire `Notification`
   matchers beyond `permission_prompt`, `PostCompact`, `PostModelSwitch`,
   `StopFailure`, `TaskCreated` and `TaskCompleted`, `Stop.background_tasks`
   and `session_crons`. Show "idle", "compacted", "model switched", "turn
   failed", and task counts on the row; do not mark a session with live
   background work or a `/loop` as dormant. Size S. Risk: none beyond
   noise; keep the sidebar reason line short.

6. **Worktree hooks so Claude uses aiw's worktree layout.** Ship a
   `WorktreeCreate` and `WorktreeRemove` hook through `--settings` that
   creates worktrees in aiw's location with ports and includes applied,
   so `--worktree`, subagent isolation and `/batch` land where aiw expects
   and show up as sessions. Size M. Risk: hook-created worktrees skip
   Claude's `.worktreeinclude`; aiw's own include handles it.

7. **Scheduled and recurring prompts.** A per-feature schedule (cron or
   interval) that pastes a prompt or runs a shell job, with run history.
   From amux schedules and Superset automations. Hooks into the follow-up
   queue and the resource gate. Size M. Risk: unattended runs need a
   budget; pair with Vibe mode and the agent cap.

8. **Watchdog recovery.** Detect a crashed or exited agent pane, a stuck
   turn (no hook events past a threshold) or a context-full session, and
   offer restart with resume, `/compact`, or replay the last unsent prompt.
   From amux self-healing and NTM stuck detection. Hooks into the existing
   exited sweep and launch grace. Size M. Risk: false positives on long
   tool calls; require two signals.

9. **Agent-to-agent messaging by aiw name.** Give every Claude session
   its aiw name and let the dashboard post a message into a session's inbox
   socket (`CLAUDE_CODE_MESSAGING_SOCKET`) without typing into the pane; let
   agents address each other by feature name. From Cyclops, amux and
   Claude's own cross-session messaging. Size S for Claude, M if Codex
   gets an equivalent via `turn/steer`. Risk: a bypass-permissions session
   holds messages from a prompting one; document the pairing.

10. **Groups, notes and presets.** Session groups with colours, a note per
    session, and saved layout presets (agent plus terminal plus server) per
    project. From agent-session-manager, Cyclops presets and Superset
    terminal presets. Hooks into aiw.json feature presets and custom
    sessions. Size S. Risk: none; pure organisation.

Deliberately not proposed: file reservations and function-level locks
(wit, foremerge, NTM) because worktree-per-agent already prevents the
collision; Kanban boards because TODO lists plus the sidebar cover the
state machine; MCP servers for agents to drive aiw because the automation
IPC already exists and adding an MCP server would pull a network-shaped
dependency into the binary; anything cloud or phone.

## Sources

- https://github.com/superset-sh/superset and https://docs.superset.sh/overview
- https://github.com/smtg-ai/claude-squad
- https://docs.conductor.build
- https://virtuslab.com/blog/ai/vibe-kanban/
- https://github.com/mixpeek/amux
- https://vibecoding.app/blog/ntm-review and https://github.com/Dicklesworthstone/ntm
- https://github.com/standardagents/dmux
- https://dev.to/pvgomes/herdr-is-tmux-for-coding-agents-3ai0
- https://github.com/cyclops-team/cyclops
- https://github.com/izll/agent-session-manager
- https://github.com/ShreyPaharia/octomux
- https://github.com/yylo-dev/yylo
- https://cdn.jsdelivr.net/npm/ccmanager@4.1.18/README.md
- https://github.com/asheshgoplani/agent-deck
- https://github.com/andyrewlee/awesome-agent-orchestrators
- https://code.claude.com/docs/en/cli-reference
- https://code.claude.com/docs/en/hooks
- https://code.claude.com/docs/en/agent-view
- https://code.claude.com/docs/en/agents
- https://code.claude.com/docs/en/agent-teams
- https://code.claude.com/docs/en/sessions
- https://code.claude.com/docs/en/worktrees
- https://code.claude.com/docs/en/cross-session-messaging
- https://code.claude.com/docs/en/statusline
- https://code.claude.com/docs/en/checkpointing
- https://code.claude.com/docs/en/plugins/mods/overview
- https://learn.chatgpt.com/docs/app-server
- https://learn.chatgpt.com/docs/developer-commands?surface=cli
- https://learn.chatgpt.com/docs/agent-configuration/subagents
- https://codex.danielvaughan.com/2026/04/15/codex-cli-hooks-complete-guide-events-policy-patterns/
- https://codex.danielvaughan.com/2026/03/27/codex-cli-in-2026-whats-new/
