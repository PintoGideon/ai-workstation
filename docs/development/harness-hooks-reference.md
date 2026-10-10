# Harness hook contracts (researched 2026-10-10)

## Claude Code (code.claude.com/docs/en/hooks.md, statusline.md, settings.md, cli-reference.md)

Settings precedence: managed > `--settings` (file path or inline JSON, one session, does not write files) > `.claude/settings.local.json` > `.claude/settings.json` > `~/.claude/settings.json`. `CLAUDE_CONFIG_DIR` relocates `~/.claude`.

Common stdin fields on every hook: `session_id`, `prompt_id`, `transcript_path`, `cwd`, `scratchpad_dir`, `permission_mode` (default|plan|acceptEdits|auto|dontAsk|bypassPermissions), `effort{level}`, `hook_event_name`, `agent_id`/`agent_type` (only inside a subagent). Matcher: exact name/list (`Bash`, `Edit|Write`) or regex; Notification matcher = `notification_type`; SessionStart = `source`; SessionEnd = `reason`; PreCompact = `trigger`; SubagentStart/Stop = agent type. Exit 0 = success (stdout JSON parsed if a single object), exit 2 = blocking error with stderr as reason, other = non-blocking. Universal output: `continue`, `stopReason`, `systemMessage`, `hookSpecificOutput{hookEventName,…}`. Default command timeout 600 s (`timeout` per hook, seconds); UserPromptSubmit 30 s; SessionEnd 1.5 s budget (per-hook `timeout` up to 60 s). `async: true` = background, cannot decide. Env for hook commands: `CLAUDE_PROJECT_DIR`, `CLAUDE_ENV_FILE` (SessionStart), `CLAUDE_CODE_REMOTE`.

### PermissionRequest
- Fires only when a prompt would otherwise appear (so not for acceptEdits-auto-approved edits; rarely in bypassPermissions). Not an agent-type hook.
- stdin: common + `tool_name`, `tool_input` (+`mcp_server` for MCP), optional `permission_suggestions[]`.
- stdout allow: `{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow","updatedInput":{…optional…},"updatedPermissions":[…optional…]}}}`
- stdout deny: `{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"deny","message":"…","interrupt":false}}}`
- Exit 0 with no decision → normal prompt shown. Exit 2 does NOT block here; use the decision object.
- Notification `permission_prompt` does not fire alongside (it only fires after ~6 s unanswered).

### statusLine
- `{"statusLine":{"type":"command","command":"/abs/path","padding":0,"refreshInterval":5}}`; can be supplied via `--settings` (user-settings key).
- stdin JSON: `cwd`, `session_id`, `session_name?`, `transcript_path`, `version`, `model{id,display_name}`, `workspace{current_dir,project_dir,added_dirs[],git_worktree?,repo?{host,owner,name}}`, `output_style{name}`, `cost{total_cost_usd,total_duration_ms,total_api_duration_ms,total_lines_added,total_lines_removed}`, `context_window{total_input_tokens,total_output_tokens,context_window_size,used_percentage,remaining_percentage,current_usage{input_tokens,output_tokens,cache_creation_input_tokens,cache_read_input_tokens}|null}`, `exceeds_200k_tokens`, `rate_limits?{five_hour{used_percentage,resets_at},seven_day{…},spend_limit{…}}`, `effort?{level}`, `thinking{enabled}`, `fast_mode`, `agent?{name}`, `pr?{number,url,review_state,kind}`, `worktree?{name,path,branch,original_cwd,original_branch}`, `vim?{mode}`.
- Runs at session start, after each assistant message, after /compact, on mode change, on `refreshInterval`; debounced 300 ms; `COLUMNS`/`LINES` env give size. Output lines are shown in Claude's footer (we can print one short line AND write the JSON to a file for AIW).

### Stop
- stdin: common + `stop_hook_active`, `last_assistant_message`, `background_tasks`, `session_crons`.
- Continue with a new instruction: `{"decision":"block","reason":"<next instruction>"}` (exit 2 + stderr equivalent). Check `stop_hook_active` to avoid loops; cap of 8 consecutive continuations (`CLAUDE_CODE_STOP_HOOK_BLOCK_CAP`).

### UserPromptSubmit
- `{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"…"}}` adds context; `decision:"block"` blocks.

### PreCompact / PostCompact / SessionEnd
- PreCompact stdin: common + `trigger` (manual|auto), `custom_instructions` (string|null). Output: `{"decision":"block"}` or exit 2 blocks compaction; nothing injects instructions. PostCompact stdin: `trigger`, `compact_summary`.
- SessionEnd stdin: `reason` ∈ clear|resume|logout|prompt_input_exit|other.

### SubagentStart / SubagentStop
- SubagentStart stdin: `agent_id`, `agent_type`; output `hookSpecificOutput.additionalContext`.
- SubagentStop stdin: `stop_hook_active`, `agent_id`, `agent_type`, `agent_transcript_path`, `last_assistant_message`; `{"decision":"block","reason":…}` continues the subagent.

### CLI flags
- `--settings <file|json>`, `--append-system-prompt "<text>"`, `--append-system-prompt-file <path>` (combinable), `--session-id <uuid>`, `--name <display name>` (resume with `claude -r <name>`), `--permission-mode default|acceptEdits|plan|auto|dontAsk|bypassPermissions`, `--system-prompt-snapshot on|off`. Do not use `--bare` or `--safe-mode` (they disable hooks/status line). Auto mode is the default starting mode for interactive sessions since v2.1.283.

## Codex CLI (learn.chatgpt.com/docs; openai/codex rust-v0.162.1, 2026-10-09)

Hooks GA since 2026-05. Config: `~/.codex/hooks.json` or `[hooks]` in `~/.codex/config.toml`; project `<repo>/.codex/hooks.json` / `.codex/config.toml` (only when the project `.codex/` layer is trusted); all layers MERGE. Every non-managed hook must be trusted via `/hooks` in the TUI (trust bound to the hook definition hash) — or pass `--dangerously-bypass-hook-trust` on launch. Hooks on by default (`[features] hooks = false` disables).

Events: `SessionStart` (matcher source startup|resume|clear|compact), `SessionEnd` (reason other), `UserPromptSubmit`, `PreToolUse`, `PermissionRequest`, `PostToolUse` (matcher tool name: `Bash`, `apply_patch`/`Edit`/`Write`, `spawn_agent`/`Agent`, `mcp__server__tool`), `PreCompact`/`PostCompact` (trigger manual|auto), `SubagentStart`/`SubagentStop` (agent_type), `Stop`, `Interrupt`. No Notification event; `notify = ["prog", …]` (user config only) receives ONE JSON ARGV (not stdin): `{type:"agent-turn-complete","thread-id","turn-id",cwd,"input-messages"[],"last-assistant-message"}`.

hooks.json shape: `{"hooks":{"Stop":[{"hooks":[{"type":"command","command":"…","timeout":30,"statusMessage":"…","async":false}]}]}}`; `timeout` seconds default 600 (SessionEnd/Interrupt 1 s default, 3 s max). Commands run with the session cwd.

stdin common: `session_id`, `transcript_path` (nullable, rollout JSONL), `cwd`, `hook_event_name`, `model`; turn events add `turn_id`, `permission_mode`. Event fields: PreToolUse `tool_name`,`tool_use_id`,`tool_input{command…}`; PermissionRequest `tool_name`,`tool_input{…,description}`; PostToolUse +`tool_response`; SubagentStart `agent_id`,`agent_type`; SubagentStop `agent_id`,`agent_type`,`agent_transcript_path`,`stop_hook_active`,`last_assistant_message`; Stop `stop_hook_active`,`last_assistant_message`; PreCompact `trigger`.

stdout: PermissionRequest `{"hookSpecificOutput":{"hookEventName":"PermissionRequest","decision":{"behavior":"allow"}}}` / `{"behavior":"deny","message":"…"}` (no updatedInput/interrupt); PreToolUse deny `{"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":"…"}}`; Stop/SubagentStop `{"decision":"block","reason":"<next prompt>"}` — reason becomes a new user prompt (loop guard `stop_hook_active`; `continue:false` overrides); PreCompact `{"continue":false}` aborts. Stop/SubagentStop REQUIRE JSON on exit 0. Exit 2 = block with stderr reason (PreToolUse, PostToolUse, UserPromptSubmit, SubagentStop, Stop).

Approval: `approval_policy = "on-request" | "never" | {granular=…}` (`untrusted` retired, `on-failure` deprecated); `sandbox_mode = read-only|workspace-write|danger-full-access`; flags `-a/--ask-for-approval`, `--sandbox`, `--dangerously-bypass-approvals-and-sandbox`. App-server (`codex app-server`, JSON-RPC) can also answer `item/commandExecution/requestApproval` (experimental). `codex mcp-server` removed.

Context usage: rollout files `$CODEX_HOME/sessions/YYYY/MM/DD/rollout-<ts>-<thread_id>.jsonl` (may be `.jsonl.zst` when cold); lines `{timestamp, ordinal?, type, payload}`; `event_msg` payload `{"type":"token_count","info":{"total_token_usage":{input_tokens,cached_input_tokens,cache_write_input_tokens,output_tokens,reasoning_output_tokens,total_tokens},"last_token_usage":{…},"model_context_window":<int|null>},"rate_limits":{…}}`; `turn_context` payload has `model`, `approval_policy`, `sandbox_policy`; `session_meta` has `cwd`, `cli_version`, `agent_role`, `parent_thread_id`. TUI `[tui] status_line = ["model","context-remaining","git-branch"]`.

Launch injection: `-c key=value` (TOML value, highest precedence), `--profile <name>` (`~/.codex/<name>.config.toml`), `developer_instructions = "…"` (extra developer instructions — AIW already uses this), `model_instructions_file` (replaces built-ins), AGENTS.md chain, `CODEX_HOME` (dir must exist). Project config ignores `notify`.

## Gemini CLI (geminicli.com/docs, v0.63.0, 2026-10-06; paid/Code Assist users keep `gemini`)

Hooks in `settings.json` (`.gemini/settings.json` project > `~/.gemini/settings.json` user): `{"hooks":{"AfterAgent":[{"matcher":"*","hooks":[{"name":"aiw-after-agent","type":"command","command":"…","timeout":60000}]}]},"hooksConfig":{"enabled":true,"notifications":true}}`. `timeout` MILLISECONDS default 60000. Project hooks fingerprinted (name+command); changed hook → user warned before it runs. Events: `SessionStart` (source startup|resume|clear), `SessionEnd` (reason exit|clear|logout|prompt_input_exit|other), `BeforeAgent` (prompt), `AfterAgent` (prompt, prompt_response, stop_hook_active), `BeforeModel`, `BeforeToolSelection`, `AfterModel` (llm_response per chunk with usageMetadata.totalTokenCount), `BeforeTool` (tool_name, tool_input), `AfterTool` (+tool_response{llmContent,returnDisplay,error?}), `PreCompress` (trigger auto|manual; advisory, async), `Notification` (notification_type "ToolPermission", message, details; observe-only). Common stdin: `session_id`, `transcript_path`, `cwd`, `hook_event_name`, `timestamp`. Env: `GEMINI_PROJECT_DIR`, `GEMINI_SESSION_ID`, `GEMINI_CWD`, `CLAUDE_PROJECT_DIR`.

stdout: only the final JSON on stdout; `decision: "allow"|"deny"` (+`reason`), `continue`, `stopReason`, `systemMessage`, `hookSpecificOutput{hookEventName, additionalContext | tool_input | clearContext | …}`. AfterAgent `{"decision":"block","reason":"<text>"}` → response rejected and reason sent as a new prompt (loop guard `stop_hook_active`). BeforeTool deny `{"decision":"deny","reason":"…"}`. Exit 2 = block with stderr. SessionStart ignores decision; Notification cannot grant permission.

Permission: NO hook can approve a prompt. Allow-lists: `tools.allowed` (e.g. `["run_shell_command(git)"]`, restart required), `tools.confirmationRequired`, policy engine TOML (`[[rule]] toolName="run_shell_command" decision="allow"|"deny"|"ask_user" priority=…`, `--policy <file>`), `--approval-mode default|auto_edit|yolo|plan` (`--yolo` deprecated alias).

Context usage: session files `~/.gemini/tmp/<project_hash>/chats/session-<YYYY-MM-DDTHH-MM>-<id8>.jsonl`: first record `{sessionId, projectHash, startTime, lastUpdated, kind}`, then `{id, timestamp, type:"user"|"gemini"|…, content, model?, thoughts?, toolCalls?, tokens:{input,output,cached,thoughts?,tool?,total}}`; patches `{"$patch":…}`, `{"$set":…}`. Telemetry local: `"telemetry":{"enabled":true,"target":"local","outfile":".gemini/telemetry.log","logPrompts":false}` → events `gemini_cli.api_response{model,input_token_count,output_token_count,cached_content_token_count,total_token_count,…}`, `gemini_cli.chat_compression{tokens_before,tokens_after}`, `gemini_cli.tool_call`, `gemini_cli.agent.start/finish`. Footer: `ui.footer.hideContextPercentage` (default true!), `ui.footer.hideModelInfo`.

Launch injection: `GEMINI_SYSTEM_MD=/path` REPLACES the system prompt (not a suffix); for a suffix use `context.fileName` (GEMINI.md or array), `--include-directories`, or SessionStart/BeforeAgent `additionalContext`. `GEMINI_CLI_HOME` relocates `~/.gemini`. `-m/--model`, `-e/--extensions`, `--resume`, `--approval-mode`. No subagent hooks (telemetry `gemini_cli.agent.*` only).

## Cursor agent CLI (`agent`; cursor.com/docs/hooks, cli/*; changelog v2026.09.28)

Hooks: `<project>/.cursor/hooks.json` (trusted workspace), `~/.cursor/hooks.json`; all matching hooks run, responses merge (deny > ask > allow; `followup_message` last wins). Hot-reloaded. Payloads on STDIN since 2026-05-20. Shape: `{"version":1,"hooks":{"stop":[{"command":".cursor/hooks/x.sh","loop_limit":10}],"beforeShellExecution":[{"command":"…","timeout":30,"matcher":"…","failClosed":true}]}}`; `timeout` seconds; `loop_limit` default 5 for stop/subagentStop.

Common stdin: `conversation_id`, `generation_id`, `model`, `model_id?`, `hook_event_name`, `cursor_version`, `workspace_roots[]`, `user_email|null`, `transcript_path|null`. Env: `CURSOR_PROJECT_DIR`, `CURSOR_TRANSCRIPT_PATH`, `CLAUDE_PROJECT_DIR`.

Events → output: `sessionStart` (session_id, composer_mode) → `{"env":{…},"additional_context":"…"}`; `sessionEnd` (reason completed|aborted|error|window_close|user_close, duration_ms); `preToolUse` (tool_name, tool_input, tool_use_id, cwd) → `{"permission":"allow"|"deny","user_message","agent_message","updated_input"}`; `postToolUse` (tool_output string, duration) → `{"additional_context"}`; `postToolUseFailure`; `subagentStart` (subagent_id, subagent_type, task, subagent_model) → `{"permission":"allow"|"deny"}`; `subagentStop` (subagent_type, status completed|error|aborted, task, summary, duration_ms, modified_files[], agent_transcript_path) → `{"followup_message":"…"}` (only when completed); `beforeShellExecution` (command, cwd, sandbox) → `{"continue":true,"permission":"allow"|"deny"|"ask","user_message","agent_message"}`; `beforeMCPExecution` (tool_name, tool_input string, mcp_server_name) → same; `afterShellExecution` (command, output, duration); `afterFileEdit` (file_path, edits[]); `beforeReadFile` (file_path, content) → `{"permission"}`; `beforeSubmitPrompt` (prompt, attachments) → `{"continue","user_message"}`; `afterAgentResponse` (text); `afterAgentThought` (text, duration_ms); `stop` (status completed|aborted|error, loop_count) → `{"followup_message":"…"}` (auto-submitted as the next user message); `preCompact` (trigger, context_usage_percent, context_tokens, context_window_size, message_count, messages_to_compact, is_first_compaction) → `{"user_message"}`. Exit 0 = JSON, exit 2 = deny, other = fail-open unless `failClosed`; invalid JSON on permission hooks blocks.

Permission: hooks decide (`beforeShellExecution`, `beforeMCPExecution`, `beforeReadFile`, `subagentStart`, `preToolUse`); `ask` = fall back to the interactive prompt. Static: `~/.cursor/cli-config.json` / `.cursor/cli.json` `{"version":1,"permissions":{"allow":["Shell(git)","Read(src/**)"],"deny":["Shell(rm)"]},"approvalMode":"allowlist"|"auto-review"|"unrestricted"}`; flags `-f/--force`, `--approve-mcps`, `--trust`, `--sandbox`, `--mode plan|ask`.

Context usage: `preCompact` payload only (tokens/percent at compaction); `statusLine` key mentioned in the Apr 2026 changelog but undocumented (UNVERIFIED); headless stream-json has token totals (field names UNVERIFIED). Launch injection: no CLI system-prompt flag; `.cursor/rules/*.mdc`, `AGENTS.md`/`CLAUDE.md`, `sessionStart` hook `additional_context`, `--plugin-dir`, `CURSOR_CONFIG_DIR`, `--resume [chatId]`, `agent create-chat`. Renames 2026: `cursor-agent`→`agent`, `/compress`→`/summarize`.

## Cross-CLI cheat sheet

| Need | Claude Code | Codex | Gemini | Cursor |
|---|---|---|---|---|
| Turn complete | Stop | Stop (or notify argv) | AfterAgent | stop |
| Next instruction | Stop `{"decision":"block","reason"}` | same | AfterAgent same | stop `{"followup_message"}` |
| Answer permission | PermissionRequest decision.behavior | PermissionRequest decision.behavior (needs hook trust) | not via hooks; BeforeTool deny / allow-lists | before* `permission` |
| Pre-compaction | PreCompact (can block) | PreCompact (can abort) | PreCompress (advisory) | preCompact (advisory, has token counts) |
| Subagents | SubagentStart/Stop | SubagentStart/Stop | none | subagentStart/Stop |
| Context usage | statusLine JSON | rollout token_count | session jsonl tokens / telemetry | preCompact only |
| Timeout units | s (600) | s (600) | ms (60000) | s |
| Inject at launch | --settings, --append-system-prompt | -c, developer_instructions, --profile | GEMINI_SYSTEM_MD (replace), GEMINI.md | rules, sessionStart additional_context |
