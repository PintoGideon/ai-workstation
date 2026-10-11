use anyhow::{Context, Result};
use std::path::Path;
use std::process::{Command, Stdio};

pub struct ClaudeLauncher;

impl ClaudeLauncher {
    /// Find the newest working Claude Code binary.
    ///
    /// Scans ~/.local/share/claude/versions/ sorted by mtime (newest first)
    /// and returns the path of the first version that exits cleanly on
    /// --version. Falls back to "claude" (PATH lookup) if the versions
    /// directory doesn't exist or all candidates fail.
    pub fn resolve_binary() -> String {
        Self::resolve_binary_with(|path| {
            Command::new(path)
                .arg("--version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        })
    }

    /// Reuse launch's candidate order with a caller-owned readiness check.
    /// Background discovery can impose cancellation/deadlines on that check.
    pub(crate) fn resolve_binary_with(mut available: impl FnMut(&Path) -> bool) -> String {
        let Some(home) = dirs::home_dir() else {
            return "claude".to_string();
        };
        let versions_dir = home.join(".local/share/claude/versions");

        let Ok(entries) = std::fs::read_dir(&versions_dir) else {
            return "claude".to_string();
        };

        let mut candidates: Vec<(std::time::SystemTime, std::path::PathBuf)> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .filter_map(|e| {
                let mtime = e.metadata().ok()?.modified().ok()?;
                Some((mtime, e.path()))
            })
            .collect();

        candidates.sort_by_key(|candidate| std::cmp::Reverse(candidate.0));

        for (_, path) in candidates {
            if available(&path) {
                return path.to_string_lossy().into_owned();
            }
        }

        "claude".to_string()
    }

    /// Check if a working Claude CLI is available
    pub fn check_available() -> Result<()> {
        let binary = Self::resolve_binary();
        let output = Command::new(&binary)
            .arg("--version")
            .output()
            .context("claude CLI not found - is Claude Code installed?")?;

        if !output.status.success() {
            anyhow::bail!("claude CLI returned an error");
        }
        Ok(())
    }
}
