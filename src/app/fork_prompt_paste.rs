//! FORK: prompts handed to an agent, unsent (DESIGN.md §18).
//!
//! Every path that used to open the compose box seeded with text — a
//! a TODO spawn, a plan-interview kickoff, a prompt
//! from the library, a feature's `startup_prompt` — now pastes the text into
//! the agent's real tmux window with `paste_text` and **no Enter**, saves it
//! as the checkout's `latest-prompt.txt`, selects the session's row and
//! focuses the agent pane, where the user reads it and presses Enter
//! themselves. The task prompt is never sent on its own. A failed paste is
//! an error the caller stashes (`stash_lost_prompt`) or reports.
//!
//! Without a cockpit — the `App` embedded in the desktop GUI, or a test —
//! nothing is pasted: the prompt is kept as an [`UnsentPromptHandoff`] the
//! frontend collects with [`App::take_unsent_prompt`] and shows in its own
//! composer, exactly what it read from the compose box before. The GUI
//! contract is unchanged.

use anyhow::{Context, Result, anyhow};

use super::{App, AppMode, Selection, ViewState};

/// A prompt an embedding frontend still has to show, unsent.
#[derive(Clone)]
pub struct UnsentPromptHandoff {
    pub view: ViewState,
    pub text: String,
}

impl App {
    /// The prompt the last embedded delivery left for the frontend, once.
    pub fn take_unsent_prompt(&mut self) -> Option<UnsentPromptHandoff> {
        self.unsent_prompt_handoff.take()
    }

    /// Paste `text` into the window of the session at `(pi, fi, si)`.
    pub fn paste_prompt_unsent(
        &mut self,
        pi: usize,
        fi: usize,
        si: usize,
        text: &str,
    ) -> Result<()> {
        let (view, workdir, label, session_id) = {
            let project = self
                .store
                .projects
                .get(pi)
                .ok_or_else(|| anyhow!("the project is gone"))?;
            let feature = project
                .features
                .get(fi)
                .ok_or_else(|| anyhow!("the feature is gone"))?;
            let session = feature
                .sessions
                .get(si)
                .ok_or_else(|| anyhow!("the session is gone"))?;
            let view = ViewState::new(
                project.name.clone(),
                feature.name.clone(),
                feature.tmux_session.clone(),
                session.tmux_window.clone(),
                session.label.clone(),
                session.kind.clone(),
                feature.mode.clone(),
                feature.review,
            );
            (
                view,
                feature.workdir.clone(),
                session.label.clone(),
                session.id.clone(),
            )
        };
        if text.trim().is_empty() {
            return Ok(());
        }
        self.persist_startup_prompt(&workdir, text);
        if self.cockpit_active() {
            self.tmux
                .paste_text(&view.session, &view.window, text)
                .with_context(|| format!("could not paste the prompt into {label}"))?;
        } else {
            self.unsent_prompt_handoff = Some(UnsentPromptHandoff {
                view,
                text: text.to_string(),
            });
        }
        self.selection = Selection::Session(pi, fi, si);
        self.mode = AppMode::Normal;
        self.mark_finished_seen(&session_id);
        if self.cockpit_active() {
            self.cockpit_focus_agent();
            self.push_toast_success(format!(
                "Prompt pasted into {label} — read it, then press Enter to send"
            ));
        }
        Ok(())
    }

    /// Paste into the window named by tmux `session:window`.
    pub fn paste_prompt_unsent_to_window(
        &mut self,
        session: &str,
        window: &str,
        text: &str,
    ) -> Result<()> {
        let (pi, fi, si) = self
            .session_indices_by_window(session, window)
            .ok_or_else(|| anyhow!("no session runs in {session}:{window}"))?;
        self.paste_prompt_unsent(pi, fi, si, text)
    }

    /// Paste into the feature's first running agent.
    pub fn paste_prompt_unsent_to_feature(
        &mut self,
        pi: usize,
        fi: usize,
        text: &str,
    ) -> Result<()> {
        let previous = std::mem::replace(&mut self.selection, Selection::Feature(pi, fi));
        let Some(target) = self.desired_live_target() else {
            self.selection = previous;
            return Err(anyhow!("the feature has no running agent to paste into"));
        };
        let (pi, fi, si) = self
            .session_indices_by_id(&target.session_id)
            .ok_or_else(|| anyhow!("the session is gone"))?;
        self.paste_prompt_unsent(pi, fi, si, text)
    }

    pub(crate) fn session_indices_by_window(
        &self,
        session: &str,
        window: &str,
    ) -> Option<(usize, usize, usize)> {
        self.store.projects.iter().enumerate().find_map(|(pi, p)| {
            p.features
                .iter()
                .enumerate()
                .filter(|(_, f)| f.tmux_session == session)
                .find_map(|(fi, f)| {
                    f.sessions
                        .iter()
                        .position(|s| s.tmux_window == window)
                        .map(|si| (pi, fi, si))
                })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::fork_workspace::workspace_app;
    use crate::fork::cockpit::{Link, Request};
    use crate::project::SessionKind;
    use crate::traits::{MockTmuxOps, MockWorktreeOps};

    fn app_with(tmux: MockTmuxOps, workdir: &std::path::Path) -> App {
        let template = workspace_app(&[SessionKind::Claude, SessionKind::Codex]);
        let mut store = template.store.clone();
        store.projects[0].features[0].workdir = workdir.to_path_buf();
        let mut app = App::new_for_test(store, Box::new(tmux), Box::new(MockWorktreeOps::new()));
        app.viewport_cols = 36;
        app.viewport_rows = 57;
        app.viewport_total_rows = 60;
        app.selection = Selection::Session(0, 0, 0);
        app.cockpit_attach(
            Link::recording(),
            "/bin/aiw".into(),
            "/s/cockpit.sock".into(),
        );
        app.fork_cockpit.link.as_ref().unwrap().clear_recorded();
        app
    }

    #[test]
    fn the_prompt_is_pasted_without_enter_saved_and_the_agent_pane_focused() {
        let dir = tempfile::tempdir().unwrap();
        let mut tmux = MockTmuxOps::new();
        tmux.expect_paste_text()
            .withf(|s, w, t| s == "aiw-hx-12" && w == "codex" && t == "Do the thing")
            .times(1)
            .returning(|_, _, _| Ok(()));
        // No send_key_name expectation: an Enter would fail the mock.
        let mut app = app_with(tmux, dir.path());
        app.mode = AppMode::Help(crate::app::HelpState {
            from_view: None,
            scroll_offset: 0,
        });
        app.paste_prompt_unsent(0, 0, 1, "Do the thing").unwrap();
        assert!(matches!(app.selection, Selection::Session(0, 0, 1)));
        assert!(matches!(app.mode, AppMode::Normal), "the dialog closed");
        assert_eq!(
            std::fs::read_to_string(dir.path().join(".claude/latest-prompt.txt")).unwrap(),
            "Do the thing"
        );
        let requests = app.fork_cockpit.link.as_ref().unwrap().recorded();
        assert!(
            matches!(&requests[0], Request::Respawn { title, .. } if title.ends_with("Codex 1")),
            "{requests:?}"
        );
        assert_eq!(requests[1], Request::FocusAgent);
        assert!(
            app.toasts
                .iter()
                .any(|t| t.message.contains("press Enter to send"))
        );
    }

    #[test]
    fn without_a_cockpit_the_prompt_is_kept_for_the_frontend() {
        let dir = tempfile::tempdir().unwrap();
        let mut app = app_with(MockTmuxOps::new(), dir.path());
        app.fork_cockpit.link = None;
        app.paste_prompt_unsent(0, 0, 1, "For the GUI").unwrap();
        let handoff = app.take_unsent_prompt().expect("kept");
        assert_eq!(handoff.text, "For the GUI");
        assert_eq!(
            (handoff.view.session.as_str(), handoff.view.window.as_str()),
            ("aiw-hx-12", "codex")
        );
        assert!(app.take_unsent_prompt().is_none(), "taken once");
        assert!(matches!(app.selection, Selection::Session(0, 0, 1)));
        assert!(matches!(app.mode, AppMode::Normal));
    }

    #[test]
    fn a_failed_paste_is_an_error_and_nothing_moves() {
        let dir = tempfile::tempdir().unwrap();
        let mut tmux = MockTmuxOps::new();
        tmux.expect_paste_text()
            .returning(|_, _, _| Err(anyhow!("tmux is gone")));
        let mut app = app_with(tmux, dir.path());
        let err = app.paste_prompt_unsent(0, 0, 0, "text").unwrap_err();
        assert!(err.to_string().contains("Claude 1"), "{err}");
        assert!(
            app.fork_cockpit
                .link
                .as_ref()
                .unwrap()
                .recorded()
                .is_empty()
        );
        assert!(
            app.paste_prompt_unsent(0, 0, 0, "   ").is_ok(),
            "nothing to paste"
        );
        assert!(app.paste_prompt_unsent(0, 7, 0, "x").is_err());
    }

    #[test]
    fn by_window_and_by_feature_find_the_session() {
        let dir = tempfile::tempdir().unwrap();
        let mut tmux = MockTmuxOps::new();
        tmux.expect_paste_text()
            .withf(|_, w, _| w == "claude")
            .times(2)
            .returning(|_, _, _| Ok(()));
        let mut app = app_with(tmux, dir.path());
        app.paste_prompt_unsent_to_window("aiw-hx-12", "claude", "a")
            .unwrap();
        assert!(
            app.paste_prompt_unsent_to_window("aiw-none", "claude", "a")
                .is_err()
        );
        app.selection = Selection::Project(0);
        app.paste_prompt_unsent_to_feature(0, 0, "b").unwrap();
        assert!(matches!(app.selection, Selection::Session(0, 0, 0)));
        app.store.projects[0].features[0].status = crate::project::ProjectStatus::Stopped;
        app.selection = Selection::Project(0);
        assert!(app.paste_prompt_unsent_to_feature(0, 0, "c").is_err());
        assert!(matches!(app.selection, Selection::Project(0)), "restored");
    }
}
