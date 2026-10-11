//! Keep files AIW writes into a checkout out of commits.
//!
//! Patterns go into the repository's `info/exclude`, which git honours like a
//! `.gitignore` but which is never itself committed or shared. All worktrees
//! of a repository share one `info/exclude` in the *common* git dir, so a
//! linked worktree resolves `git rev-parse --git-common-dir` rather than its
//! own per-worktree git dir.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

const HEADER: &str = "# Added by AI Workstation: helper files it writes into checkouts";

/// Append each pattern in `patterns` that is not already present (as an exact
/// line) to the `info/exclude` of the repository containing `checkout`.
///
/// A `checkout` that is not inside a git repository (or a machine without
/// `git`) is not an error: there is nothing to exclude from. Patterns that are
/// empty or span several lines are skipped.
pub fn ensure_excluded(checkout: &Path, patterns: &[&str]) -> std::io::Result<()> {
    let Some(common_dir) = git_common_dir(checkout) else {
        return Ok(());
    };
    // Unit tests must never touch a real repository's exclude file (a test
    // pointed at the source checkout would otherwise edit it). Both sides are
    // resolved: git reports a linked worktree's common dir with symlinks
    // resolved, and macOS's temp dir `/var/folders/…` is `/private/var/…`.
    #[cfg(test)]
    {
        let resolve = |p: PathBuf| p.canonicalize().unwrap_or(p);
        if !resolve(common_dir.clone()).starts_with(resolve(std::env::temp_dir())) {
            return Ok(());
        }
    }
    ensure_excluded_in(&common_dir, patterns)
}

/// Helper files AIW (and, before it, upstream AMF) writes into checkouts that
/// must never be committed. Files users may intentionally commit
/// (`aiw.json`, `amf.json`, `AGENTS.md`) are deliberately
/// absent.
pub(crate) const AIW_HELPER_PATTERNS: &[&str] = &[
    // Worktrees created before AIW moved them out of the repo, and AIW's
    // per-checkout state (interview docs, legacy config).
    ".worktrees/",
    ".aiw/",
    // Claude Code
    "CLAUDE.local.md",
    ".claude/settings.local.json",
    ".claude/aiw-hook-state.json",
    ".claude/.gitignore",
    ".claude/notifications/",
    ".claude/latest-prompt.txt",
    ".claude/context.md",
    ".claude/skills/aiw-*/",
    // Codex
    ".codex/aiw-codex-notify.sh",
    ".agents/skills/aiw-*/",
    // FORK: what upstream AMF wrote into the same checkouts, so worktrees
    // shared with (or migrated from) AMF stay clean too.
    ".amf/",
    ".claude/amf-hook-state.json",
    ".claude/skills/amf-*/",
    ".codex/amf-codex-notify.sh",
    ".agents/skills/amf-*/",
];

/// Exclude every AIW helper file in `checkout`'s repository, logging (not
/// failing) on error.
pub(crate) fn exclude_aiw_helper_files(checkout: &Path) {
    if let Err(err) = ensure_excluded(checkout, AIW_HELPER_PATTERNS) {
        crate::debug::log_to_file(
            crate::debug::LogLevel::Warn,
            "git_exclude",
            &format!(
                "could not update info/exclude for {}: {err}",
                checkout.display()
            ),
        );
    }
}

/// The repository's common git dir (shared by all its worktrees), absolute.
pub(crate) fn git_common_dir(checkout: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--git-common-dir"])
        .current_dir(checkout)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if raw.is_empty() {
        return None;
    }
    let path = PathBuf::from(raw);
    Some(if path.is_absolute() {
        path
    } else {
        checkout.join(path)
    })
}

fn ensure_excluded_in(common_dir: &Path, patterns: &[&str]) -> std::io::Result<()> {
    let wanted: Vec<&str> = patterns
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty() && !p.contains('\n') && !p.contains('\r'))
        .collect();
    if wanted.is_empty() {
        return Ok(());
    }

    let info_dir = common_dir.join("info");
    std::fs::create_dir_all(&info_dir)?;
    let path = info_dir.join("exclude");
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .append(true)
        .create(true)
        .open(&path)?;
    // Several AIW processes (or the TUI and a CLI call) may start features of
    // the same repo at once; serialize the read-modify-append.
    file.lock()?;

    let mut current = String::new();
    file.seek(SeekFrom::Start(0))?;
    file.read_to_string(&mut current)?;
    let existing: std::collections::HashSet<&str> = current.lines().map(str::trim).collect();

    let mut missing: Vec<&str> = Vec::new();
    for pattern in wanted {
        if !existing.contains(pattern) && !missing.contains(&pattern) {
            missing.push(pattern);
        }
    }
    if missing.is_empty() {
        return Ok(());
    }

    let mut out = String::new();
    if !current.is_empty() && !current.ends_with('\n') {
        out.push('\n');
    }
    if !existing.contains(HEADER) {
        out.push_str(HEADER);
        out.push('\n');
    }
    for pattern in missing {
        out.push_str(pattern);
        out.push('\n');
    }
    file.write_all(out.as_bytes())?;
    file.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            status.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&status.stderr)
        );
    }

    fn init_repo() -> TempDir {
        let dir = TempDir::new().unwrap();
        git(dir.path(), &["init", "-q", "-b", "main"]);
        git(dir.path(), &["config", "user.email", "t@example.com"]);
        git(dir.path(), &["config", "user.name", "t"]);
        std::fs::write(dir.path().join("README.md"), "hi\n").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-qm", "init"]);
        dir
    }

    fn exclude_of(repo: &Path) -> String {
        std::fs::read_to_string(repo.join(".git/info/exclude")).unwrap_or_default()
    }

    fn is_ignored(dir: &Path, rel: &str) -> bool {
        Command::new("git")
            .args(["check-ignore", "-q", rel])
            .current_dir(dir)
            .status()
            .unwrap()
            .success()
    }

    #[test]
    fn appends_missing_patterns_once() {
        let repo = init_repo();
        ensure_excluded(repo.path(), &[".claude/skills/aiw-*/", ".aiw/"]).unwrap();
        ensure_excluded(repo.path(), &[".aiw/", ".codex/aiw-codex-notify.sh"]).unwrap();
        let exclude = exclude_of(repo.path());
        assert_eq!(exclude.matches(".aiw/\n").count(), 1);
        assert_eq!(exclude.matches(HEADER).count(), 1);
        assert!(exclude.contains(".claude/skills/aiw-*/\n"));
        assert!(exclude.contains(".codex/aiw-codex-notify.sh\n"));
        assert!(is_ignored(
            repo.path(),
            ".claude/skills/aiw-configure/SKILL.md"
        ));
        assert!(is_ignored(repo.path(), ".codex/aiw-codex-notify.sh"));
        assert!(!is_ignored(repo.path(), "aiw.json"));
    }

    #[test]
    fn keeps_existing_user_content_and_adds_trailing_newline() {
        let repo = init_repo();
        let info = repo.path().join(".git/info");
        std::fs::create_dir_all(&info).unwrap();
        std::fs::write(info.join("exclude"), "# mine\nscratch/").unwrap();
        ensure_excluded(repo.path(), &["scratch/", ".aiw/"]).unwrap();
        let exclude = exclude_of(repo.path());
        assert!(exclude.starts_with("# mine\nscratch/\n"));
        assert_eq!(exclude.matches("scratch/").count(), 1);
        assert!(exclude.ends_with(".aiw/\n"));
    }

    #[test]
    fn linked_worktree_writes_the_common_exclude() {
        let repo = init_repo();
        let wt_parent = TempDir::new().unwrap();
        let wt = wt_parent.path().join("wt");
        git(
            repo.path(),
            &["worktree", "add", "-q", "-b", "feat", &wt.to_string_lossy()],
        );
        ensure_excluded(&wt, &[".agents/skills/aiw-*/"]).unwrap();
        assert!(exclude_of(repo.path()).contains(".agents/skills/aiw-*/\n"));
        assert!(is_ignored(&wt, ".agents/skills/aiw-x/SKILL.md"));
    }

    #[test]
    fn helper_patterns_hide_aiw_files_but_not_user_files() {
        let repo = init_repo();
        exclude_aiw_helper_files(repo.path());
        for hidden in [
            ".claude/skills/aiw-configure/SKILL.md",
            ".claude/settings.local.json",
            ".codex/aiw-codex-notify.sh",
            ".agents/skills/aiw-add-hook/SKILL.md",
            ".worktrees/x/file",
            ".aiw/interview-docs/state.json",
            "CLAUDE.local.md",
            // Upstream AMF's helper files stay hidden too.
            ".amf/session-status/x.json",
            ".claude/skills/amf-configure/SKILL.md",
            ".codex/amf-codex-notify.sh",
            ".agents/skills/amf-add-hook/SKILL.md",
        ] {
            assert!(
                is_ignored(repo.path(), hidden),
                "{hidden} should be excluded"
            );
        }
        for visible in [
            "aiw.json",
            "amf.json",
            "AGENTS.md",
            ".claude/settings.json",
            ".claude/skills/mine/SKILL.md",
            ".codex/config.toml",
        ] {
            assert!(
                !is_ignored(repo.path(), visible),
                "{visible} must stay committable"
            );
        }
    }

    #[test]
    fn non_git_directory_is_a_no_op() {
        let dir = TempDir::new().unwrap();
        ensure_excluded(dir.path(), &[".aiw/"]).unwrap();
        assert!(!dir.path().join(".git").exists());
    }

    #[test]
    fn skips_empty_and_multiline_patterns() {
        let repo = init_repo();
        ensure_excluded(repo.path(), &["", "  ", "a\nb"]).unwrap();
        assert!(!exclude_of(repo.path()).contains(HEADER));
    }
}
