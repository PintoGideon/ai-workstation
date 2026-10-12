//! App integration tests, grouped by behavior.

mod attention;
mod automation;
mod editor_lifecycle;
mod feature_sessions;
mod fork_agents; // FORK: Gemini/Cursor
mod fork_fan_out; // FORK: one prompt, several agents (§19.3)
mod fork_identity; // FORK: names, forks, Claude roster (§19.2)
mod fork_organize; // FORK: groups, notes, layout presets (§19.4)
mod fork_session_reconcile; // FORK
mod hooks_setup;
mod pane_input;
mod prompts_configuration;
mod resources;
mod startup_navigation;
mod status_sidebar;
mod store_sync;
mod support;
mod todos;
