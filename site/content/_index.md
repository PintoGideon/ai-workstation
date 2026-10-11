+++
title = "AI Workstation"
template = "index.html"

[extra]
headline = "Run multiple AI coding agents in parallel, without losing track of them."
subhead = "AIW is a terminal cockpit for managing Claude Code, Codex, Gemini CLI, Cursor agent and Pi sessions. It organizes work by project and feature, creates isolated git worktrees, and shows which agents are asking a question and which have finished work waiting for you."
install_cmd = "cargo +1.99.0 install --path . --locked\naiw"
install_note = "The crate installs the aiw binary. Keep ~/.cargo/bin in your PATH. This source installation requires Rust, a C compiler, and tmux."
hero_image = "images/dashboard.png"
hero_image_alt = "AIW dashboard showing a project with four concurrent Claude agent sessions, each on its own git worktree"

agents = ["Claude Code", "Codex", "Gemini CLI", "Cursor agent", "Pi"]

[[extra.features]]
title = "One dashboard, many agents"
body = "Run several coding-agent sessions side by side and see at a glance which ones need attention."

[[extra.features]]
title = "Isolated by design"
body = "Concurrent features stay isolated with git branches and worktrees, so parallel agents never step on each other's changes."

[[extra.features]]
title = "The agent's real terminal"
body = "The sidebar sits in a tmux pane beside the agent's own pane. Typing is native; nothing is relayed or redrawn."

[[extra.features]]
title = "Answer from the dashboard"
body = "Permission prompts and Vibeless edit approvals are answered from the sidebar, and follow-ups queue up for the agent's next turn."

[[extra.features]]
title = "Know what it costs"
body = "Context use, cost and rate-limit windows on every row, read from the harness's own files. No network calls."

[[extra.features]]
title = "Reusable building blocks"
body = "Reusable prompts, scoped TODO lists, themes, lifecycle hooks, and workspace presets carry across features and projects."
+++

## About the project

🤖 AIW is a terminal-based control center for running AI coding agents against your own repos — built for everyday developers, not teams burning enterprise budgets on the priciest frontier models. It's designed around subscription plans people already have through work or pay for out of pocket, so you get real leverage from AI-assisted coding without needing a premium API tier. AIW stays out of the agent's way: planning, code review and compaction happen in the agent's own session, and AIW manages the sessions, the worktrees, the permissions and the attention. It makes no network calls of its own.
