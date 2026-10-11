use anyhow::Result;

use super::*;

impl App {
    pub fn start_rename_session(&mut self) {
        let (pi, fi, si) = match &self.selection {
            Selection::Session(pi, fi, si) => (*pi, *fi, *si),
            _ => return,
        };

        let label = match self
            .store
            .projects
            .get(pi)
            .and_then(|p| p.features.get(fi))
            .and_then(|f| f.sessions.get(si))
        {
            Some(s) => s.label.clone(),
            None => return,
        };

        self.mode = AppMode::RenamingSession(RenameSessionState {
            project_idx: pi,
            feature_idx: fi,
            session_idx: si,
            input: label,
            return_to: RenameReturnTo::Dashboard,
        });
    }

    pub fn apply_rename_session(&mut self) -> Result<()> {
        let (pi, fi, si, input) = match &self.mode {
            AppMode::RenamingSession(state) => (
                state.project_idx,
                state.feature_idx,
                state.session_idx,
                state.input.clone(),
            ),
            _ => return Ok(()),
        };

        if input.trim().is_empty() {
            self.message = Some("Name cannot be empty".into());
            self.push_toast_warning("Name cannot be empty");
            return Ok(());
        }

        if let Some(session) = self
            .store
            .projects
            .get_mut(pi)
            .and_then(|p| p.features.get_mut(fi))
            .and_then(|f| f.sessions.get_mut(si))
        {
            session.label = input.clone();
        }
        self.save()?;

        let old_mode = std::mem::replace(&mut self.mode, AppMode::Normal);
        if let AppMode::RenamingSession(rename_state) = old_mode {
            match rename_state.return_to {
                RenameReturnTo::Dashboard => {
                    self.mode = AppMode::Normal;
                }
            }
        }

        self.message = Some(format!("Renamed to '{}'", input));
        Ok(())
    }

    pub fn cancel_rename_session(&mut self) {
        let old_mode = std::mem::replace(&mut self.mode, AppMode::Normal);
        if let AppMode::RenamingSession(state) = old_mode {
            match state.return_to {
                RenameReturnTo::Dashboard => {
                    self.mode = AppMode::Normal;
                }
            }
        }
    }
}
