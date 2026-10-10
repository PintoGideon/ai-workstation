//! AI Workstation (AIW) additions to the code base inherited from upstream
//! (credited in the README).
//!
//! Everything substantial the fork adds lives here so cherry-picking
//! upstream commits stays cheap; upstream files only carry small `// FORK:`
//! hooks into these modules. See `DESIGN.md` for the roadmap.

// G1 — split view inside the TUI and `aiw grid` (DESIGN.md §4).
pub mod grid;
pub mod split;

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
pub mod cursor;
pub mod extra_agents;
pub mod gemini;

// G4/G5 — work-safety fixes (DESIGN.md §7).
pub(crate) mod config;
pub mod git_exclude;
pub(crate) mod hook_approval;
pub(crate) mod ipc_rebind; // re-bind the IPC socket if its file vanishes
pub(crate) mod line_fold; // incremental JSONL reads for the status sweep (DESIGN.md §12.3)
pub mod migrate; // first-run copy of an AMF setup (DESIGN.md §11)
pub(crate) mod paths;
pub(crate) mod ports;
pub(crate) mod worktree_paths;
pub(crate) mod worktree_prep;
pub(crate) mod worktree_repair; // recreate a checkout that vanished on disk

#[cfg(test)]
mod tests;
