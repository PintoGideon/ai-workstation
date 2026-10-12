# Automation

AIW now exposes machine-friendly automation entrypoints:

```bash
aiw automation create-project --file docs/automation/create-project.example.json
aiw automation create-feature --file docs/automation/create-feature.example.json
aiw automation create-batch-features --file docs/automation/create-batch-features.example.json
```

The command sends a request to the running AIW dashboard over the same local IPC system used by hook notifications. AIW applies the request inside the dashboard process and prints a JSON response.

## Requirements

- A normal `aiw` dashboard instance must already be running.
- For `create-project`, `path` must exist.
- For `create-feature`, `project_name` must already exist in AIW.
- For `create-batch-features`, `workspace_path` must be inside a git repository.
- For `create-project` and `create-batch-features`, `project_name` must not already exist in AIW.

## Create Project

Use [`create-project.template.json`](create-project.template.json) as the contract reference.

Fields:

- `path`: repo or directory to register as the AIW project
- `project_name`: the AIW-visible project name
- `dry_run`: validate and preview without changing AIW state

Example:

```bash
cat docs/automation/create-project.example.json | aiw automation create-project --dry-run
aiw automation create-project --file docs/automation/create-project.example.json
```

Typical success response:

```json
{
  "type": "automation-result",
  "action": "create_project",
  "ok": true,
  "dry_run": false,
  "input_path": "/home/you/code/my-repo",
  "project_name": "my-repo",
  "project_path": "/home/you/code/my-repo",
  "is_git": true,
  "message": "Created project 'my-repo'"
}
```

## Create Feature

Use [`create-feature.template.json`](create-feature.template.json) as the contract reference.

Fields:

- `project_name`: an existing AIW project
- `branch`: feature / branch name to create
- `agent`: `claude`, `codex`, `pi`, `gemini` or `cursor`
- `mode`: `vibeless`, `vibe`, or `supervibe`
- `use_worktree`: whether to create a git worktree or reuse the project repo
- `hook_choice`: optional answer for prompted `on_worktree_created` hooks
- `dry_run`: validate and preview without changing AIW state
- `agents`: several harnesses for one prompt, first = primary (a fan-out); empty means `agent` alone
- `fan_out`: `"worktree_each"` (sibling features `<branch>-<agent>`, grouped in the sidebar) or `"shared"` (one feature, a window per agent); defaults to `worktree_each` when the feature gets a worktree, else `shared`
- `prompt` / `prompt_file`: the task prompt each agent gets once its pane is ready (the CLI reads `prompt_file`, relative to its working directory)
- `send`: press Enter after pasting the prompt; off, it is pasted unsent
- `layout`: a `layout_presets` entry (by name) whose sessions open in each created feature once it runs; an unknown name or a bad entry fails the request before anything is created

If a repo has a prompted `on_worktree_created` hook, `--dry-run` returns a `worktree_hook_prompt` object with the hook title and valid options. An agent can use that response to pick a `hook_choice` before making the real call.

Example:

```bash
cat docs/automation/create-feature.example.json | aiw automation create-feature --dry-run
aiw automation create-feature --file docs/automation/create-feature.example.json
```

Typical success response:

```json
{
  "type": "automation-result",
  "action": "create_feature",
  "ok": true,
  "dry_run": false,
  "project_name": "my-repo",
  "branch": "automation-feature",
  "workdir": "/home/you/code/my-repo/.worktrees/automation-feature",
  "is_worktree": true,
  "tmux_session": "aiw-automation-feature",
  "started": true,
  "worktree_hook_ran": false,
  "worktree_hook_prompt": {
    "title": "Choose stack",
    "options": ["node", "rust"]
  },
  "message": "Created and started feature 'automation-feature'"
}
```

A fan-out answers with one entry per created feature in `features`:

```bash
echo '{"project_name": "my-repo", "branch": "auth-fix", "agents": ["claude", "codex"],
       "fan_out": "worktree_each", "use_worktree": true, "mode": "vibe",
       "prompt": "Fix the login redirect", "send": true}' | aiw automation create-feature
```

```json
{
  "ok": true,
  "branch": "auth-fix",
  "message": "Created 2 sibling features for 'auth-fix': auth-fix-claude, auth-fix-codex",
  "features": [
    {"branch": "auth-fix-claude", "agent": "claude", "tmux_session": "aiw-my-repo-auth-fix-claude",
     "workdir": "…/my-repo-auth-fix-claude", "feature_id": "…", "sessions": [["Claude 1", "…"]]},
    {"branch": "auth-fix-codex", "agent": "codex", "tmux_session": "aiw-my-repo-auth-fix-codex",
     "workdir": "…/my-repo-auth-fix-codex", "feature_id": "…", "sessions": [["Codex 1", "…"]]}
  ]
}
```

## Create Batch Features

Use [`create-batch-features.template.json`](create-batch-features.template.json) as the contract reference.

Key fields:

- `workspace_path`: any path inside the target repo
- `project_name`: the AIW project to create
- `feature_count`: how many parallel features to create
- `feature_prefix`: generated features will be `prefix1`, `prefix2`, ...
- `agent`: `claude`, `codex`, `pi`, `gemini` or `cursor`
- `mode`: `vibeless`, `vibe`, or `supervibe`
- `dry_run`: validate and preview without changing AIW state

## Example

Dry run from stdin:

```bash
cat docs/automation/create-batch-features.example.json | aiw automation create-batch-features --dry-run
```

Apply from a file:

```bash
aiw automation create-batch-features --file docs/automation/create-batch-features.example.json
```

Typical success response:

```json
{
  "type": "automation-result",
  "action": "create_batch_features",
  "ok": true,
  "dry_run": false,
  "workspace_path": "/home/you/code/my-repo",
  "project_name": "plan-42",
  "project_repo": "/home/you/code/my-repo",
  "features": [
    {
      "name": "plan-1",
      "branch": "plan-1",
      "workdir": "/home/you/code/my-repo/.worktrees/plan-1",
      "tmux_session": "aiw-plan-1",
      "started": true
    }
  ],
  "message": "Created project 'plan-42' with 4 features"
}
```

Typical error response:

```json
{
  "type": "automation-result",
  "action": "create_batch_features",
  "ok": false,
  "error": "Project 'plan-42' already exists"
}
```

## Message a session

```bash
aiw automation message --to auth-fix-claude-1 --text "the API schema changed; rebase on main"
aiw automation message --to "auth-fix/Codex 1" --from ci --text "CI is red on main"
echo "long message" | aiw automation message --to <aiw-session-id>
```

Sends a message to an agent session without typing into its pane. A Claude
session that reported its inbox (Claude Code 2.1.224 or later) receives it
straight away as its next prompt, attributed to `--from` (default `aiw`). Any
other target is queued exactly like `aiw automation queue` below and delivered
at the agent's next turn end. `--to` takes a session id, a Claude session's
name (`<feature>-<label>`), `<feature>/<label>`, or a label unique across AIW.

```json
{
  "type": "automation-result",
  "action": "message_session",
  "ok": true,
  "status": "delivered",
  "via": "inbox",
  "session_id": "…",
  "target": "auth-fix-claude-1",
  "from": "aiw",
  "detail": "Sent to auth-fix-claude-1's inbox"
}
```

A queued message has `"status": "queued"`, `"via": "followup-queue"`, the
queue position in `"queued"` and the reason in `"reason"`. A bad target or
empty text gives `"ok": false`, `"status": "failed"` and an `"error"`.

## Queue a follow-up

```bash
aiw automation queue --session <aiw-session-id> "rebase on main and rerun the tests"
echo "second instruction" | aiw automation queue --session <aiw-session-id>
```

Queues an instruction for an agent session. It is **not** typed into the
agent: the agent's own turn-end hook (Claude/Codex `Stop`, Gemini
`AfterAgent`, Cursor `stop`) asks AIW when the current turn ends and receives
the next queued text as its continuation, one per turn. The reply is
`{"type":"automation-result","action":"queue_followup","ok":true,"session_id":…,"queued":N}`;
an unknown session id or empty text is an error. The queue is stored in
`aiw.db` and shown on the session row as `⏭ N queued`; the dashboard dialog
(`e` on the row) lists, reorders and drops items. See the README section
"Harness hooks: what AIW does with them".

## Schedules

```bash
aiw automation schedule add --session <session> --when "every 30m" --prompt "Run the tests and fix what fails."
aiw automation schedule add --feature api-auth --project api --when "daily 02:00" \
  --command "make test" --target ci
aiw automation schedule list [--session <session> | --feature <feature> [--project <name>] | --project <name>]
aiw automation schedule pause|resume|run|delete|history --id <id or unique prefix>
```

A prompt schedule goes to an agent session: `--session` takes the same targets as `message` (an AIW session id, a
Claude session's name, `<feature>/<label>`, or a unique label); with `--feature` it goes to `--target <label>`, or the
feature's first agent. A command schedule (`--command`) runs in the feature's terminal named by `--target` (default
`schedule`), opened when missing. `--feature` takes an id, a name, a branch or `project/feature`.

`--when` is `every 30m` / `every 1h30m`, `daily 09:00`, `cron 0 9 * * 1-5` (numbers, `*`, ranges, lists and steps; no
names) or `at 2026-10-12 15:00`, in local time. Options: `--start-if-stopped`, `--catch-up once|none` (runs missed
while AIW was not running), `--end "2026-10-31 18:00"`, `--max-runs N`, `--paused`.

`add` answers with the schedule:

```json
{"type":"automation-result","action":"schedule","op":"add","ok":true,
 "schedule":{"id":"5d1c…","project":"api","feature":"api-auth","target":"→ Claude 1","kind":"prompt",
  "spec":"every 30m","describe":"every 30 min","text":"Run the tests and fix what fails.","paused":false,
  "finished":false,"next_fire":"2026-10-12T15:30:00+00:00","runs":0,"max_runs":null,"end_at":null,
  "catch_up":"once","start_if_stopped":false,"source":"user","config_name":null,"last_run":null}}
```

`list` answers `{"schedules": [ … ]}`; `run` answers `{"id", "outcome"}` where the outcome is `delivered`, `queued`,
`ran`, `started`, `delivered (inbox)`, `skipped: waiting on you`, `skipped: not running`, `skipped: previous run still
queued`, `skipped: busy (<command>)`, `skipped: resource limits` or `failed: <why>`; `history` answers `{"runs": [{"at",
"outcome", "detail"}]}`, newest first, where `detail` says what fired it (`timer`, `catch-up`, `run now`,
`automation`, `after waiting on you`). Schedules defined in `aiw.json` can be paused and run but not deleted.

### land

Land a feature (DESIGN.md §19.10): update it onto its base, run `land.check_command`, then fast-forward the base
locally or push and open a PR, as the `aiw.json` `land` block says.

```bash
aiw automation land --feature api-auth [--project api] [--dry-run] [--yes] [--no-wait]
aiw automation land --feature api-auth --status
aiw automation land --feature api-auth --abort
```

`--feature` takes a name, a branch, a nickname or an id. Without `--no-wait` the CLI waits (up to `--wait-secs`, an
hour by default) and prints the final status with the start reply under `"start"`. The IPC action is `land` with an
`op` of `dry_run`, `start`, `status` or `abort`.

- `dry_run` answers `{"landable", "working", "waiting", "preflight"}`; the preflight holds `branch`, `base`,
  `base_source`, `upstream`, `ahead`, `behind`, `modified`, `untracked`, the resolved `strategy`, `blockers`,
  `warnings`, `plan` (the commands in order) and `in_progress`. Nothing changes.
- `start` refuses (`"ok": false`) when the preflight has blockers, when agents in the feature are working and `yes`
  is not set, or when a repository `pre_land` / `post_land` hook has not been approved from the dashboard yet.
  Otherwise it answers `{"status": "started", "plan", "preflight"}`.
- `status` answers `{"state": "idle" | "running" | "succeeded" | "failed", …}`: while running, `stage` and
  `log_tail`; afterwards `result` (`ok`, `failure` with `kind` and `reason` and `in_progress`, `pr_url`,
  `pr_existed`, `merged`, `log`), `finished`, `failure_shown` and `log_tail`. It still answers after a land
  removed its feature.
- `abort` runs `git rebase --abort` or `git merge --abort` in the worktree.

Failure kinds: `locked` (another land in the repository), `pre_hook`, `fetch`, `conflict`, `check`,
`dirty_after_check`, `base_moved`, `base_dirty`, `not_fast_forward`, `push`, `gh`, `other`.
