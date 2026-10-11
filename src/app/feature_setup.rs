//! Shared state and pure transitions for compact feature setup dialogs.
//!
//! The issue browser opens a small settings dialog before making a feature
//! for an issue. The state transitions live here so any GitHub-backed
//! workflow can use the same dialog contract.

use crate::extension::FeaturePreset;
use crate::project::{AgentKind, VibeMode};

/// One editable row in a compact companion-feature setup dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureSetupRow {
    /// Apply a configured feature preset (or "Manual", which changes nothing).
    Preset,
    /// Which agent harness the feature runs.
    Harness,
    /// The feature's permission/vibe mode.
    Mode,
    /// Chrome/browser automation.
    Chrome,
    /// The branch name. Pre-filled and editable.
    Branch,
}

impl FeatureSetupRow {
    pub const ALL: [Self; 5] = [
        Self::Preset,
        Self::Harness,
        Self::Mode,
        Self::Chrome,
        Self::Branch,
    ];
}

/// State of the compact feature setup dialog.
#[derive(Debug, Clone)]
pub struct FeatureSetupState {
    /// Index 0 is the implicit "Manual" choice; preset `i` is at `i + 1`.
    pub presets: Vec<FeaturePreset>,
    pub preset_index: usize,
    /// Harnesses allowed for the selected project.
    pub agents: Vec<AgentKind>,
    pub agent_index: usize,
    pub mode: VibeMode,
    pub enable_chrome: bool,
    /// The isolated companion branch, pre-filled by the owning workflow.
    pub branch: String,
    /// Focused row index.
    pub row: usize,
    /// Inline validation/creation error.
    pub error: Option<String>,
}

impl FeatureSetupState {
    /// The chosen preset, or `None` for "Manual".
    pub fn selected_preset(&self) -> Option<&FeaturePreset> {
        self.preset_index
            .checked_sub(1)
            .and_then(|i| self.presets.get(i))
    }

    /// Display text for the preset row.
    pub fn preset_label(&self) -> String {
        self.selected_preset()
            .map_or_else(|| "Manual".to_string(), |preset| preset.name.clone())
    }

    /// The focused row, or `Branch` if state loaded from an older/invalid
    /// snapshot contains an out-of-range cursor.
    pub fn focused_row(&self) -> FeatureSetupRow {
        FeatureSetupRow::ALL
            .get(self.row)
            .copied()
            .unwrap_or(FeatureSetupRow::Branch)
    }

    pub fn agent(&self) -> AgentKind {
        self.agents
            .get(self.agent_index)
            .cloned()
            .unwrap_or_default()
    }

    /// Adjust the focused setting. Returns the newly selected preset so the
    /// caller can apply it to the dependent rows without knowing about the
    /// dialog's ownership or `AppMode`.
    pub fn adjust(&mut self, delta: isize) -> Option<FeaturePreset> {
        self.error = None;
        let applied = match self.focused_row() {
            FeatureSetupRow::Preset => {
                let len = self.presets.len() as isize + 1;
                self.preset_index = ((self.preset_index as isize + delta).rem_euclid(len)) as usize;
                self.selected_preset().cloned()
            }
            FeatureSetupRow::Harness => {
                let len = self.agents.len().max(1) as isize;
                self.agent_index = ((self.agent_index as isize + delta).rem_euclid(len)) as usize;
                None
            }
            FeatureSetupRow::Mode => {
                let all = VibeMode::ALL;
                let current = all.iter().position(|mode| *mode == self.mode).unwrap_or(0);
                self.mode =
                    all[(current as isize + delta).rem_euclid(all.len() as isize) as usize].clone();
                None
            }
            FeatureSetupRow::Chrome => {
                self.enable_chrome = !self.enable_chrome;
                None
            }
            FeatureSetupRow::Branch => None,
        };
        if let Some(preset) = &applied {
            self.apply_preset(preset);
        }
        applied
    }

    /// Apply a preset's settings while keeping the branch's meaningful suffix.
    pub fn apply_preset(&mut self, preset: &FeaturePreset) {
        if let Some(index) = self.agents.iter().position(|agent| *agent == preset.agent) {
            self.agent_index = index;
        }
        self.mode = preset.mode.clone();
        self.enable_chrome = preset.enable_chrome;
        if let Some(prefix) = &preset.branch_prefix {
            let base = self
                .branch
                .rsplit('/')
                .next()
                .unwrap_or(&self.branch)
                .to_string();
            self.branch = format!("{prefix}{base}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> FeatureSetupState {
        FeatureSetupState {
            presets: Vec::new(),
            preset_index: 0,
            agents: vec![AgentKind::Claude],
            agent_index: 0,
            mode: VibeMode::Vibeless,
            enable_chrome: false,
            branch: "feature".to_string(),
            row: 0,
            error: Some("old error".to_string()),
        }
    }

    #[test]
    fn shared_setup_transitions_match_the_compact_dialog_contract() {
        let mut state = setup();
        state.row = 2;
        assert_eq!(state.focused_row(), FeatureSetupRow::Mode);
        state.adjust(1);
        assert_eq!(state.mode, VibeMode::Vibe);
        assert!(state.error.is_none());
    }
}
