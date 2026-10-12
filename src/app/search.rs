use super::util::shorten_path;
use super::*;

impl App {
    pub fn start_search(&mut self) {
        self.mode = AppMode::Searching(SearchState {
            query: String::new(),
            matches: Vec::new(),
            selected_match: 0,
        });
        self.message = None;
        self.perform_search();
    }

    pub fn perform_search(&mut self) {
        let query = match &self.mode {
            AppMode::Searching(state) => state.query.to_lowercase(),
            _ => return,
        };

        let mut matches = Vec::new();

        for (pi, project) in self.store.projects.iter().enumerate() {
            if project.name.to_lowercase().contains(&query) {
                matches.push(SearchMatch {
                    item: VisibleItem::Project(pi),
                    label: project.name.clone(),
                    context: shorten_path(&project.repo),
                });
            }

            for (fi, feature) in project.features.iter().enumerate() {
                let display_name = feature.nickname.as_ref().unwrap_or(&feature.name);
                if feature.name.to_lowercase().contains(&query)
                    || display_name.to_lowercase().contains(&query)
                {
                    matches.push(SearchMatch {
                        item: VisibleItem::Feature(pi, fi),
                        label: display_name.clone(),
                        context: format!("{} / {}", project.name, shorten_path(&feature.workdir)),
                    });
                } else if let Some(note) = self.feature_note(&feature.id)
                    && crate::fork::organize::note_matches(note, &query)
                {
                    // FORK: §19.4 — a note matches too; the result shows
                    // the matching line.
                    matches.push(SearchMatch {
                        item: VisibleItem::Feature(pi, fi),
                        label: display_name.clone(),
                        context: format!(
                            "✎ {}",
                            crate::fork::organize::note_match_line(note, &query)
                        ),
                    });
                } else if let Some(group) = self.feature_group(pi, fi)
                    && !query.is_empty()
                    && group.name.to_lowercase().contains(&query)
                {
                    // FORK: §19.4 — and so does the feature's group name.
                    matches.push(SearchMatch {
                        item: VisibleItem::Feature(pi, fi),
                        label: display_name.clone(),
                        context: format!("{} / group {}", project.name, group.name),
                    });
                }

                for (si, session) in feature.sessions.iter().enumerate() {
                    if session.label.to_lowercase().contains(&query) {
                        matches.push(SearchMatch {
                            item: VisibleItem::Session(pi, fi, si),
                            label: session.label.clone(),
                            context: format!("{} / {}", project.name, display_name),
                        });
                    } else if let Some(note) = self.session_note(&session.id)
                        && crate::fork::organize::note_matches(note, &query)
                    {
                        matches.push(SearchMatch {
                            item: VisibleItem::Session(pi, fi, si),
                            label: session.label.clone(),
                            context: format!(
                                "{} · ✎ {}",
                                display_name,
                                crate::fork::organize::note_match_line(note, &query)
                            ),
                        });
                    }
                }
            }
        }

        if let AppMode::Searching(state) = &mut self.mode {
            state.matches = matches;
            if state.selected_match >= state.matches.len() {
                state.selected_match = 0;
            }
        }
    }

    pub fn jump_to_search_match(&mut self) {
        let match_item = match &self.mode {
            AppMode::Searching(state) => state.matches.get(state.selected_match).cloned(),
            _ => return,
        };

        if let Some(m) = match_item {
            self.selection = match m.item {
                VisibleItem::Project(pi) => Selection::Project(pi),
                VisibleItem::Feature(pi, fi) => {
                    self.store.projects[pi].collapsed = false;
                    self.reveal_feature_group(pi, fi); // FORK: §19.4
                    Selection::Feature(pi, fi)
                }
                VisibleItem::Session(pi, fi, si) => {
                    self.store.projects[pi].collapsed = false;
                    self.store.projects[pi].features[fi].collapsed = false;
                    self.reveal_feature_group(pi, fi); // FORK: §19.4
                    Selection::Session(pi, fi, si)
                }
            };
            self.mode = AppMode::Normal;
            self.message = None;
        }
    }

    pub fn cancel_search(&mut self) {
        self.mode = AppMode::Normal;
        self.message = None;
    }

    pub fn select_next_search_match(&mut self) {
        if let AppMode::Searching(state) = &mut self.mode
            && !state.matches.is_empty()
        {
            state.selected_match = (state.selected_match + 1) % state.matches.len();
        }
    }

    pub fn select_prev_search_match(&mut self) {
        if let AppMode::Searching(state) = &mut self.mode
            && !state.matches.is_empty()
        {
            state.selected_match = if state.selected_match == 0 {
                state.matches.len() - 1
            } else {
                state.selected_match - 1
            };
        }
    }
}
