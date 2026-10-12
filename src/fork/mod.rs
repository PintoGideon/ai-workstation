//! AI Workstation (AIW) additions to the code base inherited from upstream
//! (credited in the README).
//!
//! Everything substantial the fork adds lives here so cherry-picking
//! upstream commits stays cheap; upstream files only carry small `// FORK:`
//! hooks into these modules. See `DESIGN.md` for the roadmap.

// G1 — split view inside the TUI and `aiw grid` (DESIGN.md §4).
pub mod grid;
pub mod shell; // POSIX shell quoting shared by every command builder

// Workspace layout and the badge model (DESIGN.md §15); native attach (§15.4).
pub(crate) mod badges;

// The cockpit: tmux draws the dashboard (DESIGN.md §18).
pub mod cockpit;
pub mod session_info; // the session info block under the sidebar (DESIGN.md §18)

// G9 — sessions whose tmux window closed on its own.
pub mod session_reconcile;

// §19.2 — harness names and conversation forks.
pub(crate) mod session_identity;

// G6 — theming and fonts (DESIGN.md §8).
pub mod agent_context; // the launch context in the agent's system prompt (DESIGN.md §16)
pub(crate) mod appearance;
pub(crate) mod color;
pub(crate) mod contrast;
pub(crate) mod fonts;
pub(crate) mod icon_check;
pub(crate) mod termbg;
pub(crate) mod terminal;
pub(crate) mod theme_picker;
pub(crate) mod themes;

// G2 — Gemini CLI and Cursor agent (DESIGN.md §5).
pub(crate) mod background; // background Claude sessions under Claude's supervisor (`claude --bg`, DESIGN.md §19.7)
pub(crate) mod claude_roster; // `claude agents --json`: external sessions and waitingFor (DESIGN.md §19.2)
pub(crate) mod claude_settings; // Claude hooks + status line via --settings (DESIGN.md §16.6)
pub(crate) mod claude_worktrees; // Claude's own worktrees in AIW's layout via WorktreeCreate/Remove hooks (DESIGN.md §19.6)
pub mod codex_hooks; // Codex `.codex/hooks.json` (DESIGN.md §16)
pub mod cursor;
pub mod extra_agents;
pub mod fan_out; // one prompt, several agents (DESIGN.md §17, §19.3)
pub mod gemini;
pub(crate) mod messaging;
pub mod organize;
pub mod schedules; // scheduled and recurring prompts and commands (DESIGN.md §19.9)
pub(crate) mod watchdog; // flags exited, stuck, idle-with-queued and context-critical agents (DESIGN.md §19.8) // feature groups, notes and layout presets (DESIGN.md §19.4) // messages to agent sessions: Claude's inbox socket or the follow-up queue (DESIGN.md §19.5)

// G4/G5 — work-safety fixes (DESIGN.md §7).
pub(crate) mod bad_notifications; // quarantine unparsable notification files
pub mod branch_name; // git check-ref-format rules, applied in the dialog
pub(crate) mod config;
pub mod followups; // P7: queued follow-ups, delivered by the Stop hook (DESIGN.md §16)
pub mod git_exclude;
pub(crate) mod hook_approval;
pub(crate) mod hook_cli; // the `aiw` subcommands hook scripts call (DESIGN.md §16)
pub(crate) mod hook_events; // richer row events from harness hooks (DESIGN.md §19.1)
pub mod instance_lock; // one TUI per config dir (flock on aiw.lock)
pub(crate) mod ipc_rebind; // re-bind the IPC socket if its file vanishes
pub(crate) mod ipc_reply; // requests that wait for the app's answer (DESIGN.md §16)
pub mod land; // landing a feature: update, check, merge or PR (DESIGN.md §19.10)
pub mod launch; // what AIW adds to an agent's command line (DESIGN.md §16)
pub(crate) mod line_fold; // incremental JSONL reads for the status sweep (DESIGN.md §12.3)
pub mod migrate; // first-run copy of an AMF setup (DESIGN.md §11)
pub(crate) mod notices; // toasts raised off the main thread (hooks, worktree prep)
pub(crate) mod paths;
pub mod permissions; // P6: permission approvals from the dashboard (DESIGN.md §16)
pub(crate) mod ports;
pub(crate) mod quit_confirm; // `q` asks first when agents run and `confirm_quit` is on
pub mod telemetry; // P4: per-session context and cost, read locally (DESIGN.md §16)
pub(crate) mod worktree_paths;
pub(crate) mod worktree_prep;
pub(crate) mod worktree_repair; // recreate a checkout that vanished on disk

#[cfg(test)]
mod tests;
