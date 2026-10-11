//! FORK: `q` on the dashboard asks first when agents are running.
//!
//! Off unless `confirm_quit` is set in `config.json` (the e2e driver and
//! long-time users rely on `q` quitting at once). When on, and at least one
//! agent session is running, the first `q` reports how many and arms a
//! confirmation; a second `q` within [`WINDOW`] quits, `Esc` (or waiting)
//! takes it back. Quitting leaves every agent running in tmux either way —
//! the question is only whether the user meant to leave them unattended.

use std::time::{Duration, Instant};

use crate::app::App;
use crate::project::ProjectStatus;

/// How long the second `q` has to arrive.
pub(crate) const WINDOW: Duration = Duration::from_secs(5);

/// Agent sessions that are up right now.
pub(crate) fn running_agents(app: &App) -> usize {
    app.store
        .projects
        .iter()
        .flat_map(|p| p.features.iter())
        .filter(|f| f.status != ProjectStatus::Stopped)
        .flat_map(|f| f.sessions.iter())
        .filter(|s| s.kind.is_agent_harness() && s.runs_with_feature())
        .count()
}

/// `q` on the dashboard.
pub(crate) fn request_quit(app: &mut App) {
    if !app.config.aiw.confirm_quit {
        app.should_quit = true;
        return;
    }
    let running = running_agents(app);
    if running == 0 {
        app.should_quit = true;
        return;
    }
    if app
        .quit_confirm_armed_at
        .is_some_and(|armed| armed.elapsed() <= WINDOW)
    {
        app.quit_confirm_armed_at = None;
        app.should_quit = true;
        return;
    }
    app.quit_confirm_armed_at = Some(Instant::now());
    app.message = Some(format!(
        "{running} agent session{} still running — press q again to quit (they keep running in tmux), Esc to stay",
        if running == 1 { "" } else { "s" }
    ));
}

/// `Esc` on the dashboard: take back a pending confirmation. A parked plan
/// interview is "something open", so Esc still returns to its abort gate,
/// as the old quit path did; otherwise it does nothing.
pub(crate) fn cancel(app: &mut App) {
    if app.quit_confirm_armed_at.take().is_some() {
        app.message = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{ProjectStore, SessionKind};
    use crate::traits::{MockTmuxOps, MockWorktreeOps};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn app_with_running_agent(confirm: bool) -> App {
        let mut store = ProjectStore::empty();
        let mut project = crate::project::Project::new(
            "p".into(),
            std::path::PathBuf::from("/tmp/p"),
            true,
            crate::project::AgentKind::default(),
        );
        let mut feature = crate::project::Feature::new(
            "f".into(),
            "f".into(),
            std::path::PathBuf::from("/tmp/p"),
            false,
            crate::project::VibeMode::default(),
            false,
            crate::project::AgentKind::default(),
            false,
        );
        feature.status = ProjectStatus::Active;
        feature.add_session(SessionKind::Claude);
        project.features.push(feature);
        store.projects.push(project);
        let mut app = App::new_for_test(
            store,
            Box::new(MockTmuxOps::new()),
            Box::new(MockWorktreeOps::new()),
        );
        app.config.aiw.confirm_quit = confirm;
        app
    }

    fn press(app: &mut App, code: KeyCode) {
        crate::handlers::handle_normal_key(app, KeyEvent::new(code, KeyModifiers::NONE)).unwrap();
    }

    #[test]
    fn off_by_default_q_quits_at_once_with_agents_running() {
        let mut app = app_with_running_agent(false);
        assert_eq!(running_agents(&app), 1);
        press(&mut app, KeyCode::Char('q'));
        assert!(app.should_quit);
    }

    #[test]
    fn on_q_asks_then_quits_on_the_second_press() {
        let mut app = app_with_running_agent(true);
        press(&mut app, KeyCode::Char('q'));
        assert!(!app.should_quit);
        assert!(app.quit_confirm_armed_at.is_some());
        let msg = app.message.clone().unwrap_or_default();
        assert!(msg.contains("1 agent session still running"), "{msg}");
        assert!(msg.contains("press q again"), "{msg}");

        press(&mut app, KeyCode::Char('q'));
        assert!(app.should_quit);
    }

    #[test]
    fn esc_takes_the_question_back_and_a_stale_arm_asks_again() {
        let mut app = app_with_running_agent(true);
        press(&mut app, KeyCode::Char('q'));
        press(&mut app, KeyCode::Esc);
        assert!(!app.should_quit);
        assert!(app.quit_confirm_armed_at.is_none());
        assert!(app.message.is_none());

        press(&mut app, KeyCode::Char('q'));
        app.quit_confirm_armed_at = Some(Instant::now() - WINDOW - Duration::from_secs(1));
        press(&mut app, KeyCode::Char('q'));
        assert!(!app.should_quit, "a second q after the window asks again");
        assert!(app.quit_confirm_armed_at.is_some());
    }

    #[test]
    fn on_with_nothing_running_q_quits_at_once() {
        let mut app = app_with_running_agent(true);
        app.store.projects[0].features[0].status = ProjectStatus::Stopped;
        assert_eq!(running_agents(&app), 0);
        press(&mut app, KeyCode::Char('q'));
        assert!(app.should_quit);
    }
}
