//! FORK: what the embedded session view's command set left behind
//! (DESIGN.md §18). The view itself is gone — the agent's real tmux pane
//! beside the sidebar is the view — but the tools it opened still make
//! sense for the *selected* session: the markdown/plan viewer, the
//! latest-prompt dialog (re-paste a past or unsent prompt, unsent), and the
//! leader-key timer. Every "back to the session" lands on the dashboard
//! with that session's row selected (`App::return_to_dashboard`).

use anyhow::Result;

use super::*;

/// Result of the background scan started by
/// [`App::open_latest_prompt_for_selection`], delivered through
/// `App::latest_prompt_menu_bg`.
pub(crate) struct LatestPromptScanResult {
    view: ViewState,
    prompts: Vec<crate::app::util::PromptEntry>,
}

impl App {
    /// The selected session as the dialogs describe one: the agent under
    /// the selection, else the selected feature's first session (or the
    /// feature alone, with an empty window), so a dialog can name it.
    pub(crate) fn selection_view_state(&self) -> Option<ViewState> {
        if let Some(target) = self.desired_live_target() {
            return Some(target.view_state());
        }
        let (pi, fi, si) = match self.selection {
            Selection::Session(pi, fi, si) => (pi, fi, Some(si)),
            Selection::Feature(pi, fi) => (pi, fi, None),
            Selection::Project(_) => return None,
        };
        let project = self.store.projects.get(pi)?;
        let feature = project.features.get(fi)?;
        let session = si
            .and_then(|si| feature.sessions.get(si))
            .or_else(|| feature.sessions.first());
        Some(ViewState::new(
            project.name.clone(),
            feature.name.clone(),
            feature.tmux_session.clone(),
            session.map(|s| s.tmux_window.clone()).unwrap_or_default(),
            session.map(|s| s.label.clone()).unwrap_or_default(),
            session
                .map(|s| s.kind.clone())
                .unwrap_or(crate::project::SessionKind::Terminal),
            feature.mode.clone(),
        ))
    }

    /// "Open the selected session": the agent's real pane in the cockpit
    /// (`cockpit_open_selected`). The name is the one every caller has used
    /// since the embedded view; there is no view to enter any more.
    pub fn enter_view(&mut self) -> Result<()> {
        self.cockpit_open_selected()
    }

    pub(crate) fn enter_view_without_auto_compose(&mut self) -> Result<()> {
        self.cockpit_open_selected()
    }

    fn feature_workdir_for_view(&self, view: &ViewState) -> Option<PathBuf> {
        self.store
            .projects
            .iter()
            .find(|project| project.name == view.project_name)
            .and_then(|project| {
                project
                    .features
                    .iter()
                    .find(|feature| feature.name == view.feature_name)
            })
            .map(|feature| feature.workdir.clone())
    }

    fn feature_markdown_context(
        &self,
        from_view: Option<&ViewState>,
    ) -> Option<(PathBuf, Option<PathBuf>)> {
        let workdir = if let Some(view) = from_view {
            self.feature_workdir_for_view(view)?
        } else {
            self.selected_feature()
                .map(|(_, feature)| feature.workdir.clone())?
        };

        let repo_root = self
            .worktree
            .repo_root(&workdir)
            .ok()
            .filter(|root| root != &workdir);

        Some((workdir, repo_root))
    }

    /// `Ctrl+Space l`: the prompts this session has been given (and any
    /// unsent prompt stashed for its checkout), to paste one again.
    pub fn open_latest_prompt_for_selection(&mut self) {
        if self.latest_prompt_menu_bg.is_some() {
            self.message = Some("Already loading latest prompts...".into());
            return;
        }
        let Some(view) = self.selection_view_state() else {
            self.push_toast_warning("Select a session first");
            return;
        };
        if !view.session_kind.is_agent_harness() {
            self.push_toast_warning("Prompts belong to agent sessions");
            return;
        }

        let feature_prompt_context = self
            .store
            .projects
            .iter()
            .find(|project| project.name == view.project_name)
            .and_then(|project| {
                project
                    .features
                    .iter()
                    .find(|feature| feature.name == view.feature_name)
            })
            .map(|feature| {
                // The session this window is actually running, not just any
                // session sharing the workdir — two harness windows in the
                // same worktree write their transcripts into the same
                // on-disk directory, and without this the scan below could
                // surface (and let the user inject) another session's
                // prompts.
                let preferred_session_id = feature
                    .sessions
                    .iter()
                    .find(|session| session.tmux_window == view.window)
                    .and_then(super::session_ops::persisted_resume_id);

                (
                    feature.workdir.clone(),
                    view.session_kind.clone(),
                    preferred_session_id,
                )
            });

        let Some((workdir, session_kind, preferred_session_id)) = feature_prompt_context else {
            self.message = Some("Error: Could not resolve feature workdir".into());
            return;
        };

        let (tx, rx) = std::sync::mpsc::channel();
        self.latest_prompt_menu_bg = Some(rx);
        let result_view = view.clone();
        std::thread::spawn(move || {
            // Reading and parsing every transcript file for a session can be
            // slow (large or numerous histories), so it happens off the UI
            // thread; the result is still the complete list, just delivered
            // once `poll_latest_prompt_menu_bg` picks it up.
            let prompts = crate::app::util::read_all_prompts_for_session(
                &workdir,
                Some(&session_kind),
                preferred_session_id.as_deref(),
            );
            let _ = tx.send(LatestPromptScanResult {
                view: result_view,
                prompts,
            });
        });
        self.message = Some("Loading latest prompts...".into());
    }

    /// Drain the background "all prompts" scan started by
    /// [`Self::open_latest_prompt_for_selection`]. Called from the main loop
    /// beside the other `poll_*_bg` calls; returns true when the UI should
    /// redraw.
    pub fn poll_latest_prompt_menu_bg(&mut self) -> bool {
        let Some(rx) = self.latest_prompt_menu_bg.as_ref() else {
            return false;
        };
        match rx.try_recv() {
            Ok(result) => {
                self.latest_prompt_menu_bg = None;
                // The user may have opened a dialog while the scan ran; the
                // menu only lands on the dashboard.
                if matches!(self.mode, AppMode::Normal) {
                    // Unsent entries lead the list — a prompt AIW failed to
                    // deliver is the thing most worth acting on, ahead of
                    // history that already reached the agent.
                    let unsent = self
                        .feature_workdir_for_view(&result.view)
                        .zip(self.db.as_ref())
                        .and_then(|(workdir, db)| db.load_unsent_prompts_for_workdir(&workdir).ok())
                        .unwrap_or_default();
                    let mut prompts: Vec<LatestPromptItem> =
                        unsent.into_iter().map(LatestPromptItem::Unsent).collect();
                    prompts.extend(result.prompts.into_iter().map(LatestPromptItem::Sent));

                    self.mode = AppMode::LatestPrompt(LatestPromptState {
                        prompts,
                        selected: 0,
                        view: result.view,
                    });
                    self.message = None;
                } else {
                    self.log_debug(
                        "latest_prompt",
                        "discarding latest-prompt scan result: a dialog opened during load"
                            .to_string(),
                    );
                }
                true
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.latest_prompt_menu_bg = None;
                false
            }
        }
    }

    /// Paste the selected past (or unsent) prompt into the session's pane,
    /// unsent — the user reads it there and presses Enter.
    pub fn inject_latest_prompt(&mut self) -> Result<()> {
        let state = match std::mem::replace(&mut self.mode, AppMode::Normal) {
            AppMode::LatestPrompt(state) => state,
            other => {
                self.mode = other;
                return Ok(());
            }
        };

        let selected = state.prompts.get(state.selected);
        let prompt = selected
            .map(|e| e.text().trim().to_string())
            .filter(|p| !p.is_empty());
        // Consumed once it's actually delivered: this row exists only to
        // recover a prompt that never reached a session, and it has now
        // reached one — leaving it would re-offer already-sent work.
        let consumed_unsent_id = match selected {
            Some(LatestPromptItem::Unsent(u)) => Some(u.id.clone()),
            _ => None,
        };

        let Some(prompt) = prompt else {
            self.mode = AppMode::LatestPrompt(state);
            self.message = Some("No saved prompt to inject".into());
            return Ok(());
        };

        self.paste_prompt_unsent_to_window(&state.view.session, &state.view.window, &prompt)?;

        if let Some(id) = consumed_unsent_id
            && let Some(db) = &self.db
            && let Err(e) = db.delete_unsent_prompt(&id)
        {
            self.log_warn(
                "latest_prompt",
                format!("couldn't clear delivered unsent prompt: {e}"),
            );
        }
        Ok(())
    }

    pub fn save_latest_prompt_as_template(&mut self) {
        let state = match std::mem::replace(&mut self.mode, AppMode::Normal) {
            AppMode::LatestPrompt(state) => state,
            other => {
                self.mode = other;
                return;
            }
        };

        let text = state
            .prompts
            .get(state.selected)
            .map(|e| e.text().trim().to_string())
            .filter(|p| !p.is_empty());

        let Some(text) = text else {
            self.mode = AppMode::LatestPrompt(state);
            self.message = Some("No prompt to save".into());
            return;
        };

        let dest_path = self.template_source_path(crate::prompt_library::PromptSource::User, None);
        self.mode = AppMode::PromptEditor(crate::app::PromptEditorState {
            editing_id: None,
            editing_source: crate::prompt_library::PromptSource::User,
            original_template: None,
            name: String::new(),
            tags: String::new(),
            focus: crate::app::PromptEditorFocus::Name,
            editor: crate::editor::TextEditor::with_vim(text),
            return_to: Box::new(AppMode::Normal),
            dest_path,
        });
    }

    pub fn copy_selected_prompt_to_clipboard(&mut self) -> Result<()> {
        let text = match &self.mode {
            AppMode::LatestPrompt(state) => state
                .prompts
                .get(state.selected)
                .map(|e| e.text().to_string())
                .filter(|t| !t.trim().is_empty()),
            _ => return Ok(()),
        };

        let Some(text) = text else {
            self.message = Some("No prompt to copy".into());
            return Ok(());
        };

        match crate::app::util::copy_to_clipboard(&text) {
            Ok(()) => self.message = Some("Copied to clipboard".into()),
            Err(e) => self.message = Some(format!("Clipboard error: {e}")),
        }
        Ok(())
    }

    /// Resolve the Remote Control session URL from the current Claude pane,
    /// or set a helpful message and return `None`. Only meaningful while
    /// viewing a Claude session.
    pub fn latest_prompt_select_next(&mut self) {
        if let AppMode::LatestPrompt(state) = &mut self.mode
            && !state.prompts.is_empty()
            && state.selected + 1 < state.prompts.len()
        {
            state.selected += 1;
        }
    }

    pub fn latest_prompt_select_prev(&mut self) {
        if let AppMode::LatestPrompt(state) = &mut self.mode
            && state.selected > 0
        {
            state.selected -= 1;
        }
    }

    /// `Ctrl+Space m`: the Markdown files of the selected feature's checkout.
    pub fn open_markdown_viewer_for_selection(&mut self) -> Result<()> {
        let Some(view) = self.selection_view_state() else {
            self.push_toast_warning("Select a feature or session first");
            return Ok(());
        };
        self.mode = AppMode::MarkdownLoading(crate::app::MarkdownLoadingState {
            title: "Finding markdown files...".into(),
            from_view: Some(view.clone()),
            operation: crate::app::MarkdownLoadingOperation::DiscoverFromView { view },
        });
        self.message = None;
        Ok(())
    }

    pub fn open_markdown_file_picker_from_viewer(&mut self) -> Result<()> {
        let viewer = match std::mem::replace(&mut self.mode, AppMode::Normal) {
            AppMode::MarkdownViewer(viewer) => viewer,
            other => {
                self.mode = other;
                return Ok(());
            }
        };

        let Some(view) = viewer.from_view.clone() else {
            self.mode = AppMode::MarkdownViewer(viewer);
            self.message = Some("Error: Could not resolve feature workdir".into());
            return Ok(());
        };

        self.mode = AppMode::MarkdownLoading(crate::app::MarkdownLoadingState {
            title: "Finding markdown files...".into(),
            from_view: Some(view),
            operation: crate::app::MarkdownLoadingOperation::DiscoverFromViewer { viewer },
        });
        self.message = None;
        Ok(())
    }

    pub fn open_markdown_viewer_path(
        &mut self,
        path: PathBuf,
        workdir: PathBuf,
        repo_root: Option<PathBuf>,
        view: ViewState,
        return_to_picker: Option<crate::app::MarkdownFilePickerState>,
    ) -> Result<()> {
        self.mode = AppMode::MarkdownLoading(crate::app::MarkdownLoadingState {
            title: format!("Loading {}...", path.display()),
            from_view: Some(view.clone()),
            operation: crate::app::MarkdownLoadingOperation::ReadPath {
                path,
                workdir,
                repo_root,
                view,
                return_to_picker,
            },
        });
        self.message = None;
        Ok(())
    }

    pub fn markdown_loading(&self) -> bool {
        matches!(self.mode, AppMode::MarkdownLoading(_))
    }

    pub fn complete_markdown_loading(&mut self) {
        let loading = match std::mem::replace(&mut self.mode, AppMode::Normal) {
            AppMode::MarkdownLoading(loading) => loading,
            other => {
                self.mode = other;
                return;
            }
        };

        match loading.operation {
            crate::app::MarkdownLoadingOperation::DiscoverFromView { view } => {
                self.complete_markdown_discovery_from_view(view);
            }
            crate::app::MarkdownLoadingOperation::DiscoverFromViewer { viewer } => {
                self.complete_markdown_discovery_from_viewer(viewer);
            }
            crate::app::MarkdownLoadingOperation::ReadPath {
                path,
                workdir,
                repo_root,
                view,
                return_to_picker,
            } => {
                self.complete_markdown_read_path(path, workdir, repo_root, view, return_to_picker);
            }
        }
    }

    pub fn cancel_markdown_loading(&mut self) {
        let loading = match std::mem::replace(&mut self.mode, AppMode::Normal) {
            AppMode::MarkdownLoading(loading) => loading,
            other => {
                self.mode = other;
                return;
            }
        };

        self.mode = match loading.operation {
            crate::app::MarkdownLoadingOperation::DiscoverFromView { view } => {
                self.workspace_select_window(&view.session, &view.window);
                AppMode::Normal
            }
            crate::app::MarkdownLoadingOperation::DiscoverFromViewer { viewer } => {
                AppMode::MarkdownViewer(viewer)
            }
            crate::app::MarkdownLoadingOperation::ReadPath {
                view,
                return_to_picker,
                ..
            } => match return_to_picker {
                Some(picker) => AppMode::MarkdownFilePicker(picker),
                None => {
                    self.workspace_select_window(&view.session, &view.window);
                    AppMode::Normal
                }
            },
        };
    }

    fn complete_markdown_discovery_from_view(&mut self, view: ViewState) {
        let Some((workdir, repo_root)) = self.feature_markdown_context(Some(&view)) else {
            self.return_to_dashboard(&view);
            self.message = Some("Error: Could not resolve feature workdir".into());
            return;
        };

        let files = crate::markdown::collect_markdown_view_paths(&workdir, repo_root.as_deref());
        if files.is_empty() {
            self.return_to_dashboard(&view);
            self.message = Some(
                "Error: No markdown file found (.claude/*.md or top-level *.md in the worktree/repo root)"
                    .into(),
            );
            return;
        }

        if files.len() == 1 {
            self.complete_markdown_read_path(files[0].clone(), workdir, repo_root, view, None);
            return;
        }

        self.mode = AppMode::MarkdownFilePicker(crate::app::MarkdownFilePickerState {
            files,
            selected: 0,
            plan_only: true,
            search_active: false,
            query: String::new(),
            workdir,
            repo_root,
            from_view: Some(view),
        });
        self.message = None;
    }

    fn complete_markdown_discovery_from_viewer(&mut self, viewer: crate::app::MarkdownViewerState) {
        let Some(view) = viewer.from_view.clone() else {
            self.mode = AppMode::MarkdownViewer(viewer);
            self.message = Some("Error: Could not resolve feature workdir".into());
            return;
        };

        let Some((workdir, repo_root)) = self.feature_markdown_context(Some(&view)) else {
            self.mode = AppMode::MarkdownViewer(viewer);
            self.message = Some("Error: Could not resolve feature workdir".into());
            return;
        };

        let files = crate::markdown::collect_markdown_view_paths(&workdir, repo_root.as_deref());
        if files.is_empty() {
            self.mode = AppMode::MarkdownViewer(viewer);
            self.message = Some(
                "Error: No markdown file found (.claude/*.md or top-level *.md in the worktree/repo root)"
                    .into(),
            );
            return;
        }

        let selected = files
            .iter()
            .position(|path| path == &viewer.source_path)
            .unwrap_or(0);

        self.mode = AppMode::MarkdownFilePicker(crate::app::MarkdownFilePickerState {
            files,
            selected,
            plan_only: true,
            search_active: false,
            query: String::new(),
            workdir,
            repo_root,
            from_view: Some(view),
        });
        self.message = None;
    }

    fn complete_markdown_read_path(
        &mut self,
        path: PathBuf,
        workdir: PathBuf,
        repo_root: Option<PathBuf>,
        view: ViewState,
        return_to_picker: Option<crate::app::MarkdownFilePickerState>,
    ) {
        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(err) => {
                self.return_to_dashboard(&view);
                self.message = Some(format!("Error: Failed to read {}: {err}", path.display()));
                return;
            }
        };

        let title = crate::markdown::markdown_view_label(&path, &workdir, repo_root.as_deref());

        self.mode = AppMode::MarkdownViewer(crate::app::MarkdownViewerState {
            title,
            source_path: path,
            content,
            scroll_offset: 0,
            rendered_width: 0,
            rendered_lines: Vec::new(),
            return_to_picker,
            from_view: Some(view),
        });
        self.message = None;
    }

    /// Explicit viewer refresh: edits are re-read in place.
    pub fn refresh_markdown_viewer(&mut self) -> Result<()> {
        let path = match &self.mode {
            AppMode::MarkdownViewer(state) => state.source_path.clone(),
            _ => return Ok(()),
        };

        match std::fs::read_to_string(&path) {
            Ok(content) => {
                if let AppMode::MarkdownViewer(state) = &mut self.mode {
                    state.content = content;
                    state.rendered_width = 0;
                    state.rendered_lines.clear();
                }
                self.push_toast_success("Markdown refreshed");
            }
            Err(error) => {
                self.push_toast_error(format!("Could not refresh Markdown: {error}"));
            }
        }
        Ok(())
    }

    pub fn activate_leader(&mut self) {
        self.leader_active = true;
        self.leader_activated_at = Some(std::time::Instant::now());
    }

    pub fn deactivate_leader(&mut self) {
        self.leader_active = false;
        self.leader_activated_at = None;
    }

    pub fn leader_timed_out(&self) -> bool {
        let timeout_secs = self.config.leader_timeout_seconds.max(1);
        self.leader_activated_at
            .map(|t| t.elapsed() >= std::time::Duration::from_secs(timeout_secs))
            .unwrap_or(false)
    }
}
