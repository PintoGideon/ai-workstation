//! AI Workstation (AIW) additions to the code base inherited from upstream
//! (credited in the README).
//!
//! Everything substantial the fork adds lives here so cherry-picking
//! upstream commits stays cheap; upstream files only carry small `// FORK:`
//! hooks into these modules. See `DESIGN.md` for the roadmap.

// G1 — split view inside the TUI and `aiw grid` (DESIGN.md §4).
pub mod grid;
pub mod split;

// Workspace layout and the badge model (DESIGN.md §15); native attach (§15.4).
pub(crate) mod attach;
pub(crate) mod badges;

// G9 — sessions whose tmux window closed on its own.
pub mod session_reconcile;

// G6 — theming and fonts (DESIGN.md §8).
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
pub mod codex_hooks; // Codex `.codex/hooks.json` (DESIGN.md §16)
pub mod cursor;
pub mod extra_agents;
pub mod gemini;

// G4/G5 — work-safety fixes (DESIGN.md §7).
pub(crate) mod bad_notifications; // quarantine unparsable notification files
pub mod branch_name; // git check-ref-format rules, applied in the dialog
pub(crate) mod config;
pub mod git_exclude;
pub(crate) mod hook_approval;
pub(crate) mod hook_cli; // the `aiw` subcommands hook scripts call (DESIGN.md §16)
pub mod instance_lock; // one TUI per config dir (flock on aiw.lock)
pub(crate) mod ipc_rebind; // re-bind the IPC socket if its file vanishes
pub(crate) mod ipc_reply; // requests that wait for the app's answer (DESIGN.md §16)
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
