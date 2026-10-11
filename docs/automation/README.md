# Automation

AIW now exposes machine-friendly automation entrypoints:

```bash
aiw automation create-project --file docs/automation/create-project.example.json
aiw automation create-feature --file docs/automation/create-feature.example.json
aiw automation create-batch-features --file docs/automation/create-batch-features.example.json
aiw automation seed-ai-review --file docs/automation/seed-ai-review.template.json
```

The command sends a request to the running AIW dashboard over the same local IPC system used by hook notifications. AIW applies the request inside the dashboard process and prints a JSON response.

## Requirements

- A normal `aiw` dashboard instance must already be running.
- For `create-project`, `path` must exist.
- For `create-feature`, `project_name` must already exist in AIW.
- For `create-batch-features`, `workspace_path` must be inside a git repository.
- For `create-project` and `create-batch-features`, `project_name` must not already exist in AIW.

## Seed AI Review (screenshot fixture only)

`seed-ai-review` is a deterministic visual-test seam for a completed AI Review
pane. It writes supplied findings to the cache and does not launch an agent,
spend tokens, or contact GitHub. It is not an agent-review API and must not be
used to fabricate a real review result.

Use [`seed-ai-review.template.json`](seed-ai-review.template.json) as the
field reference. With `open: true`, `workdir` and `repository` (`owner/name`)
are required and AIW opens the fixture pane immediately. Screenshot CI uses
this mode so a scenario can prove review presentation and the post disclosure
without any authenticated Claude/Codex harness.

For a local visual check, the repository scenario supplies those fields from
the environment and opens the fixture in the isolated screenshot harness:

```bash
AIW_SCREENSHOT_PR_NUMBER=123 \
GITHUB_REPOSITORY=owner/repository \
scripts/dev/screenshot/aiw-capture.sh \
  --scenario scripts/dev/screenshot/scenarios/ai-review-completed-fixture.txt \
  --out-dir /tmp/aiw-ai-review-fixture
```

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
- `review`: separate toggle for diff-review / final-review flows
- `use_worktree`: whether to create a git worktree or reuse the project repo
- `hook_choice`: optional answer for prompted `on_worktree_created` hooks
- `dry_run`: validate and preview without changing AIW state

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

## Create Batch Features

Use [`create-batch-features.template.json`](create-batch-features.template.json) as the contract reference.

Key fields:

- `workspace_path`: any path inside the target repo
- `project_name`: the AIW project to create
- `feature_count`: how many parallel features to create
- `feature_prefix`: generated features will be `prefix1`, `prefix2`, ...
- `agent`: `claude`, `codex`, `pi`, `gemini` or `cursor`
- `mode`: `vibeless`, `vibe`, or `supervibe`
- `review`: separate toggle for diff-review / final-review flows
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
