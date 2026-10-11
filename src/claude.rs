use anyhow::{Context, Result};
use std::io::Write;
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

    /// Spawn a non-blocking headless Claude command; callers poll the
    /// returned `Child`'s piped stdout/stderr for the result. The prompt is
    /// piped over stdin rather than passed as an argument so large reviews do
    /// not hit the platform's argv limit. The write happens on a detached
    /// thread so this stays non-blocking — the child's stdout isn't read until
    /// the caller starts polling, so a synchronous write-then-return here
    /// would risk a pipe deadlock.
    ///
    /// `model`, when set, is passed as `--model <name>` so a bounded,
    /// single-purpose pass (a per-file walkthrough, an AI co-review) can run
    /// on a cheaper model than whatever the interactive session uses,
    /// independent of the feature's own harness/model.
    pub fn spawn_headless<'a>(
        workdir: &Path,
        prompt: &str,
        model: impl Into<crate::headless::ModelSel<'a>>,
    ) -> Result<crate::headless::LeasedChild> {
        let sel = model.into();
        let binary = Self::resolve_binary();
        let mut args = vec![
            "-p".to_string(),
            "--output-format".to_string(),
            "text".to_string(),
        ];
        if let Some(model) = sel.model {
            args.push("--model".to_string());
            args.push(model.to_string());
        }
        if let Some(level) = sel.reasoning {
            args.extend(crate::headless::reasoning_args(
                &crate::project::AgentKind::Claude,
                level,
            ));
        }
        let mut child = Command::new(&binary)
            .args(&args)
            .current_dir(workdir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to spawn claude in headless mode")?;

        if let Some(mut stdin) = child.stdin.take() {
            let prompt = prompt.to_string();
            std::thread::spawn(move || {
                let _ = stdin.write_all(prompt.as_bytes());
            });
        }

        // Wrapped so the run counts toward the agent concurrency limit for as
        // long as the caller holds the child, however that ends.
        Ok(crate::headless::LeasedChild::new(child))
    }
}
