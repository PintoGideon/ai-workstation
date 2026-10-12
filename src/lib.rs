//! The library behind the `aiw` binary (`src/main.rs` -> `cli::run`).
//!
//! Only `cli`, `automation`, `project` and `fork` are `pub`, for the binary
//! and for request/response types. Everything else stays a plain `mod`,
//! which keeps it fully visible within this crate without pulling its whole
//! public surface into clippy's API-hygiene lints (`should_implement_trait`
//! and friends). Widen deliberately, module by module, as a consumer needs
//! it.

pub mod automation;
pub mod cli;
pub mod fork; // FORK: AI Workstation additions (DESIGN.md)
pub mod project;

mod app;
mod claude;
mod codex;
mod codex_config;
mod context_collectors;
mod context_display;
mod context_tracking;
mod custom_session_icons;
mod db;
mod debug;
mod diff;
mod editor;
mod extension;
mod fswatch;
mod github;
mod handlers;
mod highlight;
mod hook_payload;
mod ipc;
mod markdown;
mod perf;
mod pi;
mod prompt_library;
mod resources;
mod theme;
mod tmux;
mod tmux_observer;
mod token_tracking;
mod traits;
mod transcript;
mod ui;
mod usage;
mod worddiff;
mod worktree;
