//! FORK: the sidebar's half of the cockpit (DESIGN.md §18).
//!
//! `aiw sidebar` is the dashboard app drawn in the left tmux pane; the
//! right pane is the selected agent's real tmux window, driven by the
//! cockpit server. This file turns app state into cockpit requests: the
//! agent pane follows the committed workspace selection (same 150 ms
//! debounce), `Enter` focuses the agent pane, a dialog zooms the sidebar,
//! `v` puts `aiw grid` in the agent pane. Every request goes through
//! [`crate::fork::cockpit::Link`] to a worker thread — never from the draw
//! path, never on the main thread.

use anyhow::Result;

use super::fork_workspace::LiveTarget;
use super::{App, AppMode, Selection};
use crate::fork::cockpit::{self, Link, Request};
use crate::project::SessionKind;

/// Runtime state of the sidebar's cockpit link.
#[derive(Default)]
pub struct CockpitRuntime {
    pub link: Option<Link>,
    /// The agent target the pane was last told to show (`Some(None)` is the
    /// placeholder); `None` before anything was sent.
    last_target: Option<Option<LiveTarget>>,
    /// The sidebar is zoomed for a dialog, as last asked.
    zoomed: bool,
    /// Counter for unique view-session names.
    views: u64,
    /// The sidebar width last seen, so a resize that left the pane at the
    /// wrong width is corrected once, not every tick.
    seen_cols: u16,
    /// This executable, for the pane commands.
    aiw: String,
    /// The cockpit server's socket, for the pane commands that retitle.
    socket: std::path::PathBuf,
}

impl CockpitRuntime {
    /// Prefix of every view session this sidebar creates on AIW's socket.
    pub fn view_prefix() -> String {
        format!("{}{}-", cockpit::VIEW_SESSION_PREFIX, std::process::id())
    }
}

impl App {
    /// The dashboard is running as the cockpit's sidebar.
    pub fn cockpit_active(&self) -> bool {
        self.fork_cockpit.link.is_some()
    }

    /// Take the link at startup: pin the sidebar width from the config and
    /// colour the active pane border from the theme.
    pub fn cockpit_attach(&mut self, link: Link, aiw: String, socket: std::path::PathBuf) {
        link.send(Request::SidebarWidth(self.config.aiw.sidebar_width));
        if let Some(rgb) = self.theme.primary.as_rgb() {
            link.send(Request::BorderColour(cockpit::tmux_colour(rgb)));
        }
        self.fork_cockpit.link = Some(link);
        self.fork_cockpit.aiw = aiw;
        self.fork_cockpit.socket = socket;
        // The launcher built the pane with the placeholder already.
        self.fork_cockpit.last_target = Some(None);
    }

    fn cockpit_view_session_name(&mut self) -> String {
        self.fork_cockpit.views += 1;
        format!(
            "{}{}",
            CockpitRuntime::view_prefix(),
            self.fork_cockpit.views
        )
    }

    /// `(command, title)` for the agent pane showing `target`.
    fn cockpit_agent_command(&mut self, target: &LiveTarget) -> (String, String) {
        let (tmux_bin, aiw_socket) = crate::tmux::TmuxManager::runtime_binary_and_socket();
        let view = self.cockpit_view_session_name();
        let command = cockpit::agent_pane_command(
            &tmux_bin.to_string_lossy(),
            aiw_socket.as_deref(),
            &self.fork_cockpit.socket,
            &target.session,
            &target.window,
            &view,
            &self.fork_cockpit.aiw,
        );
        (command, target.pane_title())
    }

    fn cockpit_pane_for(&mut self, target: Option<&LiveTarget>) -> (String, String) {
        match target {
            Some(target) => self.cockpit_agent_command(target),
            None => (
                cockpit::placeholder_command(&self.fork_cockpit.aiw),
                cockpit::PLACEHOLDER_TITLE.to_string(),
            ),
        }
    }

    fn cockpit_send(&self, request: Request) {
        if let Some(link) = &self.fork_cockpit.link {
            link.send(request);
        }
    }

    /// Show `target` in the agent pane now, if it is not what is shown.
    fn cockpit_show(&mut self, target: Option<LiveTarget>) -> bool {
        if self.fork_cockpit.last_target.as_ref() == Some(&target) {
            return false;
        }
        let (command, title) = self.cockpit_pane_for(target.as_ref());
        self.cockpit_send(Request::Respawn { command, title });
        self.fork_cockpit.last_target = Some(target);
        true
    }

    /// Zoom the sidebar for something drawn outside the main loop (the
    /// first-run icon check); the tick keeps the state in step afterwards.
    pub fn cockpit_zoom(&mut self, zoom: bool) {
        if self.cockpit_active() && self.fork_cockpit.zoomed != zoom {
            self.cockpit_send(Request::Zoom(zoom));
            self.fork_cockpit.zoomed = zoom;
        }
    }

    /// Main-loop step, after `workspace_tick`: the agent pane follows the
    /// committed selection and the sidebar zooms while a dialog is open.
    /// Returns whether a request was sent.
    pub fn cockpit_tick(&mut self) -> bool {
        if !self.cockpit_active() {
            return false;
        }
        let mut sent = false;
        let normal = matches!(self.mode, AppMode::Normal);
        // The resize hook pins the width, but a resize that races the layout
        // can leave the pane wider or narrower; the sidebar sees its own
        // width and asks once per change (never while zoomed: that is the
        // whole window, by design).
        if normal && self.viewport_cols != 0 && self.viewport_cols != self.fork_cockpit.seen_cols {
            self.fork_cockpit.seen_cols = self.viewport_cols;
            let want = cockpit::sidebar_width(self.config.aiw.sidebar_width);
            if self.viewport_cols != want && !self.fork_cockpit.zoomed {
                self.cockpit_send(Request::SidebarWidth(want));
                sent = true;
            }
        }
        if !normal && !self.fork_cockpit.zoomed {
            self.cockpit_send(Request::Zoom(true));
            self.fork_cockpit.zoomed = true;
            sent = true;
        } else if normal && self.fork_cockpit.zoomed {
            self.cockpit_send(Request::Zoom(false));
            self.fork_cockpit.zoomed = false;
            sent = true;
        }
        if normal {
            let live = self.fork_workspace.live.clone();
            sent |= self.cockpit_show(live);
        }
        sent
    }

    /// `Enter` on a running session or feature: the pane shows it at once
    /// (a pending selection is committed) and takes the keyboard. Returns
    /// `false`, with a message, when nothing under the selection runs.
    pub fn cockpit_focus_agent(&mut self) -> bool {
        self.workspace_commit_now();
        let Some(live) = self.fork_workspace.live.clone() else {
            self.message = Some(match self.selection {
                Selection::Session(..) => "Nothing to open — the session is not running".into(),
                Selection::Feature(..) => "Nothing to open — start the feature first (c)".into(),
                Selection::Project(_) => "Nothing to open — pick a feature or session".into(),
            });
            return false;
        };
        self.cockpit_show(Some(live.clone()));
        self.mark_finished_seen(&live.session_id);
        // Looking at the agent answers its generic "needs input" notes (a
        // diff review is a dialog of its own and stays).
        self.pending_inputs.retain(|input| {
            if input.project_name.as_deref() == Some(&live.project_name)
                && input.feature_name.as_deref() == Some(&live.feature_name)
                && input.notification_type != "diff-review"
            {
                let _ = std::fs::remove_file(&input.file_path);
                false
            } else {
                true
            }
        });
        self.message = None;
        self.cockpit_send(Request::FocusAgent);
        true
    }

    /// What every "open this session" path does in the cockpit: a TODOs
    /// session opens its overlay, anything else focuses the agent pane.
    pub fn cockpit_open_selected(&mut self) -> Result<()> {
        let (pi, fi, si) = match &self.selection {
            Selection::Session(pi, fi, si) => (*pi, *fi, Some(*si)),
            Selection::Feature(pi, fi) => (*pi, *fi, None),
            _ => return Ok(()),
        };
        let is_todos = si
            .and_then(|si| {
                self.store
                    .projects
                    .get(pi)
                    .and_then(|p| p.features.get(fi))
                    .and_then(|f| f.sessions.get(si))
            })
            .is_some_and(|s| s.kind == SessionKind::Todos);
        if is_todos {
            return self.open_todos_view(pi, fi);
        }
        if self.block_if_feature_pending_worktree_script(pi, fi) {
            return Ok(());
        }
        // Whatever asked to open the session (an overlay, a picker) is done.
        self.mode = AppMode::Normal;
        self.cockpit_focus_agent();
        Ok(())
    }

    /// `v`: `aiw grid` for the selected feature (or project) in the agent
    /// pane; Alt+q there brings the selected agent back.
    pub fn cockpit_show_grid(&mut self) {
        // FORK: §19.3 — a fan-out sibling tiles its whole group.
        if self.cockpit_show_group_grid() {
            return;
        }
        let (name, project) = match self.selection {
            Selection::Session(pi, fi, _) | Selection::Feature(pi, fi) => {
                let Some(feature) = self.store.projects.get(pi).and_then(|p| p.features.get(fi))
                else {
                    return;
                };
                (
                    format!("{}/{}", self.store.projects[pi].name, feature.name),
                    false,
                )
            }
            Selection::Project(pi) => {
                let Some(project) = self.store.projects.get(pi) else {
                    return;
                };
                (project.name.clone(), true)
            }
        };
        if self.desired_live_target().is_none() {
            self.message = Some(format!(
                "Tiles: no running agent sessions in {name} — start it first (c)"
            ));
            return;
        }
        self.workspace_commit_now();
        let live = self.fork_workspace.live.clone();
        let (after, after_title) = self.cockpit_pane_for(live.as_ref());
        let (tmux_bin, _) = crate::tmux::TmuxManager::runtime_binary_and_socket();
        let command = cockpit::grid_pane_command(
            &self.fork_cockpit.aiw,
            &name,
            project,
            &tmux_bin.to_string_lossy(),
            &self.fork_cockpit.socket,
            &after_title,
            &after,
        );
        self.cockpit_send(Request::Respawn {
            command,
            title: format!("tiles: {name} — Alt+q closes"),
        });
        // Whatever the pane shows after Alt+q is `live`, so the tick stays
        // quiet until the selection moves.
        self.fork_cockpit.last_target = Some(live);
        self.message = Some("Tiles in the agent pane — Alt+q there closes them".into());
    }

    /// FORK: §19.3 — tile the first agent of each of `features`
    /// (`project/feature` names), titled after `label`.
    pub(crate) fn cockpit_show_grid_for(&mut self, label: &str, features: &[String]) {
        if self.desired_live_target().is_none() {
            self.message = Some(format!(
                "Tiles: no running agent sessions in {label} — start it first (c)"
            ));
            return;
        }
        self.workspace_commit_now();
        let live = self.fork_workspace.live.clone();
        let (after, after_title) = self.cockpit_pane_for(live.as_ref());
        let (tmux_bin, _) = crate::tmux::TmuxManager::runtime_binary_and_socket();
        let command = cockpit::grid_group_pane_command(
            &self.fork_cockpit.aiw,
            features,
            &tmux_bin.to_string_lossy(),
            &self.fork_cockpit.socket,
            &after_title,
            &after,
        );
        self.cockpit_send(Request::Respawn {
            command,
            title: format!("tiles: ⇶ {label} — Alt+q closes"),
        });
        self.fork_cockpit.last_target = Some(live);
        self.message = Some(format!(
            "Tiles: {} siblings of '{label}' — Alt+q there closes them",
            features.len()
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::fork_workspace::{SELECTION_DEBOUNCE, workspace_app};
    use std::time::Duration;

    fn cockpit_app(sessions: &[SessionKind]) -> App {
        let mut app = workspace_app(sessions);
        app.cockpit_attach(
            Link::recording(),
            "/bin/aiw".into(),
            "/s/cockpit.sock".into(),
        );
        app.fork_cockpit.link.as_ref().unwrap().clear_recorded();
        app
    }

    fn recorded(app: &App) -> Vec<Request> {
        app.fork_cockpit.link.as_ref().unwrap().recorded()
    }

    fn settle(app: &mut App) {
        app.workspace_tick();
        std::thread::sleep(SELECTION_DEBOUNCE + Duration::from_millis(20));
        app.workspace_tick();
        app.cockpit_tick();
    }

    #[test]
    fn attaching_pins_the_width_and_the_theme_colour() {
        let mut app = workspace_app(&[SessionKind::Claude]);
        app.cockpit_attach(
            Link::recording(),
            "/bin/aiw".into(),
            "/s/cockpit.sock".into(),
        );
        let requests = recorded(&app);
        assert_eq!(requests[0], Request::SidebarWidth(36));
        assert!(matches!(&requests[1], Request::BorderColour(c) if c.starts_with('#')));
        assert!(app.cockpit_active());
        assert!(app.workspace_active(), "the sidebar needs no 72 columns");
    }

    #[test]
    fn the_agent_pane_follows_the_selection_after_the_debounce() {
        let mut app = cockpit_app(&[SessionKind::Claude, SessionKind::Codex]);
        assert!(!app.cockpit_tick(), "nothing committed yet");
        settle(&mut app);
        let requests = recorded(&app);
        assert_eq!(requests.len(), 1);
        let Request::Respawn { command, title } = &requests[0] else {
            panic!("{requests:?}");
        };
        assert!(
            command.contains("new-session -t =aiw-hx-12 -s aiw-view-ck"),
            "{command}"
        );
        assert!(command.contains("select-window -t :=claude"), "{command}");
        assert!(command.ends_with("exec env -u TMUX /bin/aiw sidebar --placeholder"));
        assert_eq!(title, "RS › hx-12 › Claude 1");
        // Same target: quiet.
        assert!(!app.cockpit_tick());
        // The next row, after its rest.
        app.selection = Selection::Session(0, 0, 1);
        settle(&mut app);
        let requests = recorded(&app);
        assert_eq!(requests.len(), 2);
        assert!(
            matches!(&requests[1], Request::Respawn { title, .. } if title.ends_with("Codex 1"))
        );
        // A stopped row: the placeholder.
        app.store.projects[0].features[0].sessions[1].stopped = true;
        settle(&mut app);
        let requests = recorded(&app);
        assert!(
            matches!(&requests[2], Request::Respawn { title, command } if title == "select a session" && command.contains("--placeholder"))
        );
    }

    #[test]
    fn a_dialog_zooms_the_sidebar_and_normal_restores_it() {
        let mut app = cockpit_app(&[SessionKind::Claude]);
        settle(&mut app);
        app.mode = AppMode::Help(crate::app::HelpState {
            from_view: None,
            scroll_offset: 0,
        });
        assert!(app.cockpit_tick());
        assert!(!app.cockpit_tick(), "sent once");
        app.mode = AppMode::Normal;
        assert!(app.cockpit_tick());
        let requests = recorded(&app);
        assert_eq!(requests[1], Request::Zoom(true));
        assert_eq!(requests[2], Request::Zoom(false));
        assert_eq!(
            requests.len(),
            3,
            "the pane target did not change: {requests:?}"
        );
    }

    #[test]
    fn enter_commits_the_pending_selection_shows_it_and_focuses_the_pane() {
        let mut app = cockpit_app(&[SessionKind::Claude]);
        app.workspace_tick();
        assert!(app.fork_workspace.live.is_none());
        assert!(app.cockpit_focus_agent());
        let requests = recorded(&app);
        assert!(matches!(&requests[0], Request::Respawn { .. }));
        assert_eq!(requests[1], Request::FocusAgent);
        assert!(app.fork_workspace.live.is_some());
        // Nothing running: a message, no focus.
        app.store.projects[0].features[0].sessions[0].stopped = true;
        app.selection = Selection::Session(0, 0, 0);
        settle(&mut app);
        assert!(!app.cockpit_focus_agent());
        assert!(app.message.as_deref().unwrap().contains("not running"));
        assert!(
            !recorded(&app)
                .iter()
                .skip(3)
                .any(|r| *r == Request::FocusAgent)
        );
    }

    #[test]
    fn v_puts_the_grid_in_the_pane_and_comes_back_to_the_agent() {
        let mut app = cockpit_app(&[SessionKind::Claude]);
        settle(&mut app);
        app.cockpit_show_grid();
        let requests = recorded(&app);
        let Request::Respawn { command, title } = requests.last().unwrap() else {
            panic!("{requests:?}");
        };
        assert!(
            command.starts_with("env -u TMUX /bin/aiw grid RS/hx-12; "),
            "{command}"
        );
        assert!(command.contains("@aiw_title 'RS › hx-12 › Claude 1'; env -u TMUX "));
        assert!(command.contains("select-window -t :=claude"));
        assert!(title.starts_with("tiles: RS/hx-12"));
        assert!(!app.cockpit_tick(), "the selection did not move");
        app.selection = Selection::Project(0);
        app.cockpit_show_grid();
        assert!(
            matches!(recorded(&app).last().unwrap(), Request::Respawn { command, .. } if command.contains("grid --project RS;"))
        );
        // Nothing running: no respawn, a message.
        app.store.projects[0].features[0].sessions[0].stopped = true;
        let before = recorded(&app).len();
        app.cockpit_show_grid();
        assert_eq!(recorded(&app).len(), before);
        assert!(app.message.as_deref().unwrap().contains("no running agent"));
    }
}
