//! FORK: requesting a native attach (DESIGN.md §15.4). The main loop does the
//! attach itself (`cli::run_pending_attach`); this resolves *what* to attach
//! to from the dashboard selection or a view, and says why when it cannot.

use super::fork_workspace::LiveTarget;
use super::{App, Selection};
use crate::fork::attach::PendingAttach;

impl App {
    /// `Enter` on the workspace sidebar: attach to the selected session, or
    /// to a feature's first running agent. Returns `false` (with a message)
    /// when the selection has nothing running.
    pub fn request_attach_selected(&mut self) -> bool {
        let Some(target) = self.desired_live_target() else {
            self.message = Some(match self.selection {
                Selection::Session(..) => {
                    "Nothing to attach to — the session is not running".into()
                }
                Selection::Feature(..) => {
                    "Nothing to attach to — start the feature first (c)".into()
                }
                Selection::Project(_) => "Nothing to attach to — pick a feature or session".into(),
            });
            return false;
        };
        self.request_attach_target(&target);
        true
    }

    /// Attach to a window by tmux names (the full view's leader `Enter`).
    pub fn request_attach_window(&mut self, session: &str, window: &str) -> bool {
        let found = self.store.projects.iter().find_map(|project| {
            project.features.iter().find_map(|feature| {
                if feature.tmux_session != session {
                    return None;
                }
                feature
                    .sessions
                    .iter()
                    .find(|s| s.tmux_window == window)
                    .map(|s| PendingAttach {
                        session: session.to_string(),
                        window: window.to_string(),
                        title: format!(
                            "{} › {} › {}",
                            project.name,
                            feature.nickname.as_deref().unwrap_or(&feature.name),
                            s.label
                        ),
                        session_id: s.id.clone(),
                    })
            })
        });
        match found {
            Some(req) => {
                self.pending_attach = Some(req);
                true
            }
            None => {
                self.message = Some("Nothing to attach to — the window is gone".into());
                false
            }
        }
    }

    fn request_attach_target(&mut self, target: &LiveTarget) {
        let _ = self.flush_view_input_batch();
        self.pending_attach = Some(PendingAttach {
            session: target.session.clone(),
            window: target.window.clone(),
            title: format!(
                "{} › {} › {}",
                target.project_name, target.feature_label, target.label
            ),
            session_id: target.session_id.clone(),
        });
    }

    /// Back from an attach: the agent has been looked at, and the pane must
    /// be captured afresh at its own size.
    pub fn note_attach_returned(&mut self, req: &PendingAttach, ok: bool) {
        self.mark_finished_seen(&req.session_id);
        self.pane_lines.clear();
        self.tmux_cursor = None;
        self.request_view_snapshot_refresh();
        self.message = Some(if ok {
            format!("Back from {}", req.title)
        } else {
            format!("Could not attach to {} — see the debug log", req.title)
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::fork_workspace::WorkspaceFocus;
    use crate::fork::config::DashboardLayout;
    use crate::project::{
        AgentKind, Feature, Project, ProjectStatus, ProjectStore, SessionKind, VibeMode,
    };
    use crate::traits::{MockTmuxOps, MockWorktreeOps};
    use std::path::PathBuf;

    fn feature(name: &str, session: &str) -> Feature {
        let mut feature = Feature::new(
            name.into(),
            name.into(),
            PathBuf::from("/tmp"),
            false,
            VibeMode::Vibeless,
            false,
            false,
            AgentKind::Claude,
            false,
            false,
        );
        feature.tmux_session = session.into();
        feature.add_session(SessionKind::Claude);
        feature
    }

    fn app() -> App {
        let mut running = feature("hx-12", "aiw-rs-hx-12");
        running.status = ProjectStatus::Active;
        running.nickname = Some("sso".into());
        let stopped = feature("hx-13", "aiw-rs-hx-13");
        let mut project = Project::new("rs".into(), PathBuf::from("/tmp"), true, AgentKind::Claude);
        project.features.push(running);
        project.features.push(stopped);
        let store = ProjectStore {
            version: 5,
            projects: vec![project],
            session_bookmarks: vec![],
            available_harnesses: vec![],
            prompt_templates: Vec::new(),
            extra: Default::default(),
        };
        let mut app = App::new_for_test(
            store,
            Box::new(MockTmuxOps::new()),
            Box::new(MockWorktreeOps::new()),
        );
        app.config.aiw.dashboard_layout = DashboardLayout::Workspace;
        app.viewport_cols = 120;
        app.viewport_rows = 37;
        app.viewport_total_rows = 40;
        app.fork_workspace.focus = WorkspaceFocus::Sidebar;
        app
    }

    #[test]
    fn enter_on_a_running_session_requests_the_attach_with_a_title() {
        let mut app = app();
        app.selection = Selection::Session(0, 0, 0);
        assert!(app.request_attach_selected());
        let req = app.pending_attach.clone().unwrap();
        assert_eq!(req.session, "aiw-rs-hx-12");
        assert_eq!(
            req.window,
            app.store.projects[0].features[0].sessions[0].tmux_window
        );
        assert!(req.title.starts_with("rs › sso › "), "{}", req.title);
    }

    #[test]
    fn enter_on_a_feature_row_attaches_to_its_first_running_agent() {
        let mut app = app();
        app.selection = Selection::Feature(0, 0);
        assert!(app.request_attach_selected());
        assert_eq!(app.pending_attach.unwrap().session, "aiw-rs-hx-12");
    }

    #[test]
    fn a_stopped_feature_says_so_instead_of_attaching() {
        let mut app = app();
        app.selection = Selection::Feature(0, 1);
        assert!(!app.request_attach_selected());
        assert!(app.pending_attach.is_none());
        assert!(
            app.message
                .as_deref()
                .unwrap()
                .contains("start the feature")
        );
    }

    #[test]
    fn a_view_attaches_by_tmux_names_and_a_gone_window_is_refused() {
        let mut app = app();
        let window = app.store.projects[0].features[0].sessions[0]
            .tmux_window
            .clone();
        assert!(app.request_attach_window("aiw-rs-hx-12", &window));
        assert_eq!(
            app.pending_attach.take().unwrap().title,
            format!(
                "rs › sso › {}",
                app.store.projects[0].features[0].sessions[0].label
            )
        );
        assert!(!app.request_attach_window("aiw-rs-hx-12", "nope"));
        assert!(app.pending_attach.is_none());
    }

    #[test]
    fn returning_clears_the_unread_mark_and_reports() {
        let mut app = app();
        app.selection = Selection::Session(0, 0, 0);
        app.request_attach_selected();
        let req = app.pending_attach.take().unwrap();
        app.note_attach_returned(&req, true);
        assert!(app.finished_seen.contains(&req.session_id));
        assert_eq!(
            app.message.as_deref(),
            Some(format!("Back from {}", req.title).as_str())
        );
    }
}
