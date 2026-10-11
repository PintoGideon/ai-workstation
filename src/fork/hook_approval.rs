//! One-time approval for lifecycle hooks a repository supplies.
//!
//! A repo's `aiw.json` can name `on_start`, `on_stop` and
//! `on_worktree_created` commands. Cloning a repo must not be enough to run
//! code on the user's machine, so a repo-supplied hook only runs after the
//! user has approved it once. The approval is pinned to a SHA-256 over the
//! repo path, the hook name, the exact command and — when the command runs a
//! script file — that file's content; any change asks again. Hooks from the
//! user's own global config are trusted and never ask.
//!
//! Interactive flows (TUI create/start/stop/fork) show a confirm dialog and
//! replay the action afterwards. Non-interactive flows (automation, GUI
//! contract) skip an unapproved hook with a
//! warning instead.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::Result;
use crossterm::event::KeyCode;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use sha2::{Digest, Sha256};

use crate::app::{App, AppMode};
use crate::extension::{ExtensionConfig, HookConfig};
use crate::theme::Theme;

/// Largest script file hashed and previewed; bigger files are hashed by
/// their first bytes plus their length.
const MAX_SCRIPT_BYTES: usize = 256 * 1024;
const PREVIEW_LINES: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HookName {
    Start,
    Stop,
    WorktreeCreated,
}

impl HookName {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            HookName::Start => "on_start",
            HookName::Stop => "on_stop",
            HookName::WorktreeCreated => "on_worktree_created",
        }
    }

    fn configured(self, ext: &ExtensionConfig) -> Option<&HookConfig> {
        let hooks = &ext.lifecycle_hooks;
        match self {
            HookName::Start => hooks.on_start.as_ref(),
            HookName::Stop => hooks.on_stop.as_ref(),
            HookName::WorktreeCreated => hooks.on_worktree_created.as_ref(),
        }
    }
}

/// A script file the hook command runs, captured for hashing and display.
#[derive(Debug, Clone)]
pub(crate) struct ScriptFile {
    pub path: PathBuf,
    pub bytes: Option<Vec<u8>>,
}

/// Exactly what is being approved.
#[derive(Debug, Clone)]
pub(crate) struct HookSubject {
    pub repo: PathBuf,
    pub hook: HookName,
    pub command: String,
    pub script: Option<ScriptFile>,
    pub sha256: String,
}

/// What to replay once the user has decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HookResume {
    /// Re-run `create_feature` from the restored new-feature wizard.
    Create,
    /// Re-run `create_forked_feature` from the restored fork dialog.
    Fork,
    /// Re-run `begin_start_feature(pi, fi)`.
    Start { pi: usize, fi: usize },
    /// Re-run `stop_feature` on the (unchanged) selection.
    Stop,
}

pub struct HookApprovalState {
    pub(crate) subject: HookSubject,
    pub(crate) resume: HookResume,
    pub(crate) previous_mode: Box<AppMode>,
}

/// Approvals recorded while no database is open (tests, read-only runs).
static MEMORY_APPROVALS: Mutex<Vec<(String, String, String)>> = Mutex::new(Vec::new());
/// "Skip this time" decisions waiting for the replayed action to consume.
static SKIP_ONCE: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// Build the approval subject for `command` configured as `hook` in `repo`.
/// `run_dir` is where the command will run (used to find a script file).
pub(crate) fn subject(repo: &Path, hook: HookName, command: &str, run_dir: &Path) -> HookSubject {
    let script = find_script_file(command, run_dir, repo).map(|path| ScriptFile {
        bytes: read_capped(&path),
        path,
    });
    let mut hasher = Sha256::new();
    hasher.update(b"aiw-hook-approval-v1\0");
    hasher.update(repo.to_string_lossy().as_bytes());
    hasher.update(b"\0");
    hasher.update(hook.as_str().as_bytes());
    hasher.update(b"\0");
    hasher.update(command.as_bytes());
    hasher.update(b"\0");
    match &script {
        Some(script) => {
            let shown = script
                .path
                .strip_prefix(run_dir)
                .or_else(|_| script.path.strip_prefix(repo))
                .unwrap_or(&script.path);
            hasher.update(shown.to_string_lossy().as_bytes());
            hasher.update(b"\0");
            match &script.bytes {
                Some(bytes) => hasher.update(bytes),
                None => hasher.update(b"<unreadable>"),
            }
        }
        None => hasher.update(b"<no-script-file>"),
    }
    HookSubject {
        repo: repo.to_path_buf(),
        hook,
        command: command.to_string(),
        script,
        sha256: format!("{:x}", hasher.finalize()),
    }
}

fn read_capped(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let mut bytes = Vec::new();
    file.take(MAX_SCRIPT_BYTES as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    if len as usize > MAX_SCRIPT_BYTES {
        bytes.extend_from_slice(format!("\0<truncated:{len}>").as_bytes());
    }
    Some(bytes)
}

const INTERPRETERS: &[&str] = &[
    "sh", "bash", "zsh", "dash", "fish", "python", "python3", "node", "ruby", "perl", "deno",
    "bun", "pwsh",
];

/// The script file a hook command runs, if it names one: `./setup.sh`,
/// `scripts/x.sh`, `~/bin/x`, `/abs/x`, or `bash setup.sh` style.
fn find_script_file(command: &str, run_dir: &Path, repo: &Path) -> Option<PathBuf> {
    let mut tokens = command
        .split_whitespace()
        .map(|t| t.trim_matches(|c| c == '\'' || c == '"'))
        .skip_while(|t| t.contains('=') && !t.starts_with('-') && !t.contains('/'));
    let first = tokens.next()?;
    let first_name = Path::new(first)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let candidate = if INTERPRETERS.contains(&first_name.as_str()) {
        tokens.find(|t| !t.starts_with('-'))?
    } else if first.contains('/') {
        first
    } else {
        return None;
    };
    let expanded = super::paths::expand_user_path(candidate);
    let resolved = if expanded.is_absolute() {
        expanded
    } else {
        let in_run_dir = run_dir.join(&expanded);
        if in_run_dir.is_file() {
            in_run_dir
        } else {
            repo.join(&expanded)
        }
    };
    resolved.is_file().then_some(resolved)
}

/// True when the hook named `hook` with this exact `command` comes from the
/// repository's own config (and not also, identically, from the user's
/// global config) — i.e. it needs approval.
pub(crate) fn is_repo_supplied(
    repo: &Path,
    hook: HookName,
    command: &str,
    global: &ExtensionConfig,
) -> bool {
    let Some(path) = crate::extension::resolve_project_config_path(repo) else {
        return false;
    };
    let Some(project) = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str::<ExtensionConfig>(&s).ok())
    else {
        return false;
    };
    let from_repo = hook
        .configured(&project)
        .is_some_and(|cfg| cfg.script() == command);
    let from_global = hook
        .configured(global)
        .is_some_and(|cfg| cfg.script() == command);
    from_repo && !from_global
}

fn memory_approved(subject: &HookSubject) -> bool {
    let key = (
        subject.repo.to_string_lossy().into_owned(),
        subject.hook.as_str().to_string(),
        subject.sha256.clone(),
    );
    MEMORY_APPROVALS
        .lock()
        .map(|list| list.contains(&key))
        .unwrap_or(false)
}

fn remember_in_memory(subject: &HookSubject) {
    if let Ok(mut list) = MEMORY_APPROVALS.lock() {
        list.push((
            subject.repo.to_string_lossy().into_owned(),
            subject.hook.as_str().to_string(),
            subject.sha256.clone(),
        ));
    }
}

fn mark_skip_once(sha: &str) {
    if let Ok(mut set) = SKIP_ONCE.lock() {
        set.get_or_insert_with(HashSet::new).insert(sha.to_string());
    }
}

fn take_skip_once(sha: &str) -> bool {
    SKIP_ONCE
        .lock()
        .ok()
        .and_then(|mut set| set.as_mut().map(|s| s.remove(sha)))
        .unwrap_or(false)
}

impl App {
    fn repo_hook_is_approved(&self, subject: &HookSubject) -> bool {
        if memory_approved(subject) {
            return true;
        }
        self.db.as_ref().is_some_and(|db| {
            db.hook_approval_exists(
                &subject.repo.to_string_lossy(),
                subject.hook.as_str(),
                &subject.sha256,
            )
            .unwrap_or(false)
        })
    }

    fn record_repo_hook_approval(&mut self, subject: &HookSubject) {
        let stored = self.db.as_ref().map(|db| {
            db.record_hook_approval(
                &subject.repo.to_string_lossy(),
                subject.hook.as_str(),
                &subject.sha256,
                &subject.command,
            )
        });
        match stored {
            Some(Ok(())) => {}
            Some(Err(err)) => {
                // Still honour the decision for this run.
                remember_in_memory(subject);
                self.log_warn("hooks", format!("could not persist hook approval: {err}"));
            }
            None => remember_in_memory(subject),
        }
    }

    /// Gate a lifecycle hook in an interactive flow.
    ///
    /// Returns `Some(hook)` to continue (`hook` is `None` when there is no
    /// hook or the user chose to skip it this time) and `None` when an
    /// approval dialog is now open — the caller must return and let the
    /// dialog replay `resume`.
    pub(crate) fn gate_repo_hook(
        &mut self,
        repo: &Path,
        hook: HookName,
        config: Option<HookConfig>,
        run_dir: &Path,
        resume: HookResume,
    ) -> Option<Option<HookConfig>> {
        let Some(config) = config else {
            return Some(None);
        };
        if !is_repo_supplied(repo, hook, config.script(), &self.config.extension) {
            return Some(Some(config));
        }
        let subject = subject(repo, hook, config.script(), run_dir);
        if self.repo_hook_is_approved(&subject) {
            return Some(Some(config));
        }
        if take_skip_once(&subject.sha256) {
            self.message = Some(format!(
                "Skipped the repository's {} hook (not approved)",
                hook.as_str()
            ));
            return Some(None);
        }
        let previous_mode = std::mem::replace(&mut self.mode, AppMode::Normal);
        self.mode = AppMode::ApprovingRepoHook(Box::new(HookApprovalState {
            subject,
            resume,
            previous_mode: Box::new(previous_mode),
        }));
        None
    }

    /// [`Self::gate_repo_hook`] for a hook of the feature at `(pi, fi)`.
    pub(crate) fn gate_feature_hook(
        &mut self,
        pi: usize,
        fi: usize,
        hook: HookName,
        config: Option<HookConfig>,
        resume: HookResume,
    ) -> Option<Option<HookConfig>> {
        let Some(project) = self.store.projects.get(pi) else {
            return Some(config);
        };
        let repo = project.repo.clone();
        let run_dir = project
            .features
            .get(fi)
            .map(|f| f.workdir.clone())
            .unwrap_or_else(|| repo.clone());
        self.gate_repo_hook(&repo, hook, config, &run_dir, resume)
    }

    /// Gate a lifecycle hook in a flow that can't show a dialog: an
    /// unapproved repository hook is skipped with a warning.
    pub(crate) fn approved_repo_hook_or_skip(
        &mut self,
        repo: &Path,
        hook: HookName,
        config: Option<HookConfig>,
        run_dir: &Path,
    ) -> Option<HookConfig> {
        let config = config?;
        if !is_repo_supplied(repo, hook, config.script(), &self.config.extension) {
            return Some(config);
        }
        let subject = subject(repo, hook, config.script(), run_dir);
        if self.repo_hook_is_approved(&subject) {
            return Some(config);
        }
        self.log_warn(
            "hooks",
            format!(
                "skipped unapproved {} hook from {}: {}",
                hook.as_str(),
                repo.display(),
                config.script()
            ),
        );
        self.push_toast_warning(format!(
            "Skipped the repository's {} hook — approve it once from the dashboard",
            hook.as_str()
        ));
        None
    }

    fn finish_hook_decision(&mut self, decision: Decision) -> Result<()> {
        let AppMode::ApprovingRepoHook(state) = std::mem::replace(&mut self.mode, AppMode::Normal)
        else {
            return Ok(());
        };
        let HookApprovalState {
            subject,
            resume,
            previous_mode,
        } = *state;
        self.mode = *previous_mode;
        match decision {
            Decision::Approve => self.record_repo_hook_approval(&subject),
            Decision::Skip => mark_skip_once(&subject.sha256),
            Decision::Cancel => {
                self.message = Some(format!(
                    "Cancelled: the repository's {} hook was not approved",
                    subject.hook.as_str()
                ));
                return Ok(());
            }
        }
        let result = match resume {
            HookResume::Create => self.create_feature(),
            HookResume::Fork => self.create_forked_feature(),
            HookResume::Start { pi, fi } => self.begin_start_feature(pi, fi),
            HookResume::Stop => self.stop_feature(),
        };
        // A skip that the replay didn't consume (the action bailed early)
        // must not leak into a later run.
        take_skip_once(&subject.sha256);
        result
    }
}

#[derive(Clone, Copy)]
enum Decision {
    Approve,
    Skip,
    Cancel,
}

/// Keys for the approval dialog.
pub(crate) fn handle_key(app: &mut App, key: KeyCode) -> Result<()> {
    match key {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Char('a') => {
            app.finish_hook_decision(Decision::Approve)
        }
        KeyCode::Char('s') | KeyCode::Char('n') | KeyCode::Char('N') => {
            app.finish_hook_decision(Decision::Skip)
        }
        KeyCode::Esc | KeyCode::Char('q') => app.finish_hook_decision(Decision::Cancel),
        _ => Ok(()),
    }
}

/// Lines of the dialog body, separate from drawing so tests can check that
/// the exact command is shown.
/// A `label  value` row whose value is a path: wrapped after `/` to `width`
/// columns, continuation lines indented under the value.
fn path_lines(
    label_text: &str,
    path: &str,
    width: usize,
    label: Style,
    value: Style,
) -> Vec<Line<'static>> {
    let indent = label_text.len();
    crate::ui::fork_text::wrap_path(path, width.saturating_sub(indent).max(8))
        .into_iter()
        .enumerate()
        .map(|(index, part)| {
            let head = if index == 0 {
                label_text.to_string()
            } else {
                " ".repeat(indent)
            };
            Line::from(vec![Span::styled(head, label), Span::styled(part, value)])
        })
        .collect()
}

/// The dialog body for a box `width` columns wide inside its border.
pub(crate) fn body_lines(subject: &HookSubject, width: usize, theme: &Theme) -> Vec<Line<'static>> {
    let label = Style::default().fg(theme.text_muted.to_color());
    let value = Style::default().fg(theme.text.to_color());
    let command_style = Style::default()
        .fg(theme.warning.to_color())
        .add_modifier(Modifier::BOLD);
    let mut lines = path_lines(
        "Repository  ",
        &subject.repo.display().to_string(),
        width,
        label,
        value,
    );
    lines.extend([
        Line::from(vec![
            Span::styled("Hook        ", label),
            Span::styled(
                format!("{} (from the repo's aiw.json)", subject.hook.as_str()),
                value,
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled("Command", label)),
    ]);
    // A preview, so a long line is cut with `…` rather than wrapped into
    // the margin.
    let preview = |line: Line<'static>| crate::ui::fork_text::fit_line(line, width);
    for command_line in subject.command.lines() {
        lines.push(preview(Line::from(Span::styled(
            format!("  {command_line}"),
            command_style,
        ))));
    }
    if let Some(script) = &subject.script {
        lines.push(Line::from(""));
        lines.extend(path_lines(
            "Runs script ",
            &script.path.display().to_string(),
            width,
            label,
            value,
        ));
        match &script.bytes {
            Some(bytes) => {
                let text = String::from_utf8_lossy(bytes);
                let total = text.lines().count();
                for line in text.lines().take(PREVIEW_LINES) {
                    lines.push(preview(Line::from(Span::styled(
                        format!("  │ {line}"),
                        label,
                    ))));
                }
                if total > PREVIEW_LINES {
                    lines.push(Line::from(Span::styled(
                        format!("  │ … {} more lines", total - PREVIEW_LINES),
                        label,
                    )));
                }
            }
            None => lines.push(Line::from(Span::styled(
                "  (could not read the script file)",
                label,
            ))),
        }
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "It runs with your permissions. Approval is remembered for this exact command and script; any change asks again.",
        value,
    )));
    lines
}

pub(crate) fn draw(frame: &mut Frame, state: &HookApprovalState, theme: &Theme) {
    let width = 96.min(frame.area().width);
    let body = body_lines(&state.subject, usize::from(width.saturating_sub(2)), theme);
    let height = (body.len() as u16).saturating_add(5).clamp(12, 40);
    let area = crate::ui::dialog_rect(frame.area(), width, height);
    crate::ui::draw_modal_overlay(frame, area, theme);

    let block = Block::default()
        .title(" Run this repository hook? ")
        .borders(Borders::ALL)
        .style(Style::default().bg(theme.effective_bg()))
        .border_style(Style::default().fg(theme.warning.to_color()));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);
    frame.render_widget(Paragraph::new(body).wrap(Wrap { trim: false }), chunks[0]);

    let key = Style::default().fg(theme.warning.to_color());
    let hints = Paragraph::new(Line::from(vec![
        Span::styled(" y", key),
        Span::raw(" approve & run  "),
        Span::styled("s", key),
        Span::raw(" skip this time  "),
        Span::styled("Esc", key),
        Span::raw(" cancel"),
    ]));
    frame.render_widget(hints, chunks[1]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extension::LifecycleHooks;
    use tempfile::TempDir;

    fn write_aiw_json(repo: &Path, hooks: serde_json::Value) {
        std::fs::write(
            repo.join("aiw.json"),
            serde_json::json!({ "lifecycle_hooks": hooks }).to_string(),
        )
        .unwrap();
    }

    #[test]
    fn hash_changes_with_command_hook_repo_and_script_content() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path();
        std::fs::write(repo.join("setup.sh"), "echo one\n").unwrap();
        let base = subject(repo, HookName::Start, "./setup.sh", repo);
        assert!(base.script.is_some());
        assert_eq!(
            base.sha256,
            subject(repo, HookName::Start, "./setup.sh", repo).sha256
        );
        assert_ne!(
            base.sha256,
            subject(repo, HookName::Stop, "./setup.sh", repo).sha256
        );
        assert_ne!(
            base.sha256,
            subject(repo, HookName::Start, "./setup.sh --x", repo).sha256
        );
        let other = TempDir::new().unwrap();
        assert_ne!(
            base.sha256,
            subject(other.path(), HookName::Start, "./setup.sh", repo).sha256
        );
        std::fs::write(repo.join("setup.sh"), "echo two\n").unwrap();
        assert_ne!(
            base.sha256,
            subject(repo, HookName::Start, "./setup.sh", repo).sha256
        );
    }

    #[test]
    fn finds_script_files_behind_interpreters_and_paths() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path();
        std::fs::create_dir_all(repo.join("scripts")).unwrap();
        std::fs::write(repo.join("scripts/a.sh"), "x").unwrap();
        std::fs::write(repo.join("b.py"), "x").unwrap();
        assert_eq!(
            find_script_file("scripts/a.sh arg", repo, repo),
            Some(repo.join("scripts/a.sh"))
        );
        assert_eq!(
            find_script_file("bash -e scripts/a.sh", repo, repo),
            Some(repo.join("scripts/a.sh"))
        );
        assert_eq!(
            find_script_file("FOO=1 python3 b.py", repo, repo),
            Some(repo.join("b.py"))
        );
        assert_eq!(find_script_file("npm install", repo, repo), None);
        assert_eq!(find_script_file("./missing.sh", repo, repo), None);
    }

    #[test]
    fn only_repo_supplied_hooks_need_approval() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path();
        let global = ExtensionConfig::default();
        assert!(!is_repo_supplied(
            repo,
            HookName::Start,
            "make dev",
            &global
        ));

        write_aiw_json(repo, serde_json::json!({ "on_start": "make dev" }));
        assert!(is_repo_supplied(repo, HookName::Start, "make dev", &global));
        assert!(!is_repo_supplied(repo, HookName::Stop, "make dev", &global));
        // A global hook with a different command is the user's own.
        assert!(!is_repo_supplied(
            repo,
            HookName::Start,
            "my-global",
            &global
        ));

        // The user configured the identical command globally: trusted.
        let global_same = ExtensionConfig {
            lifecycle_hooks: LifecycleHooks {
                on_start: Some(HookConfig::Script("make dev".into())),
                ..LifecycleHooks::default()
            },
            ..ExtensionConfig::default()
        };
        assert!(!is_repo_supplied(
            repo,
            HookName::Start,
            "make dev",
            &global_same
        ));
    }

    #[test]
    fn prompted_hooks_are_recognised_from_the_repo() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path();
        write_aiw_json(
            repo,
            serde_json::json!({ "on_worktree_created": {
                "script": "./bootstrap.sh",
                "prompt": { "title": "Mode?", "options": ["a", "b"] }
            }}),
        );
        assert!(is_repo_supplied(
            repo,
            HookName::WorktreeCreated,
            "./bootstrap.sh",
            &ExtensionConfig::default()
        ));
    }

    #[test]
    fn skip_once_is_consumed() {
        mark_skip_once("abc-test");
        assert!(take_skip_once("abc-test"));
        assert!(!take_skip_once("abc-test"));
    }

    #[test]
    fn dialog_shows_the_exact_command_and_script() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path();
        std::fs::write(repo.join("setup.sh"), "#!/bin/sh\nrm -rf build\n").unwrap();
        let subject = subject(repo, HookName::WorktreeCreated, "./setup.sh --fast", repo);
        let theme = Theme::default();
        let text: String = body_lines(&subject, 94, &theme)
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("./setup.sh --fast"), "{text}");
        assert!(text.contains("on_worktree_created"));
        assert!(text.contains("rm -rf build"));
    }
}
