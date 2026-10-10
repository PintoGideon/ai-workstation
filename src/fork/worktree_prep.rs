//! Copy the git-ignored files a fresh worktree needs (`.env`, local secrets)
//! from the source checkout.
//!
//! A file is copied only when it is
//! - untracked *and* ignored by the source checkout's normal ignore rules
//!   (`.gitignore`, `info/exclude`, `core.excludesFile`), so tracked files are
//!   never touched, and
//! - matched by the include patterns: the repo's `.worktreeinclude` (gitignore
//!   syntax, at the repo root) when present, else the `worktree_include`
//!   config key (default `[".env*"]`).
//!
//! Safety rules: a destination that already exists is left alone, paths that
//! would escape either checkout are rejected, symlinks are never copied or
//! created (so `node_modules` and friends are never linked), and only regular
//! files are copied.

use std::path::{Component, Path, PathBuf};
use std::process::Command;

pub(crate) const INCLUDE_FILE: &str = ".worktreeinclude";

/// What [`copy_included_files`] did, for logging.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct PrepReport {
    pub copied: Vec<PathBuf>,
    pub skipped_existing: Vec<PathBuf>,
    pub rejected: Vec<PathBuf>,
}

/// Run after a worktree is created: copy the included files from the source
/// checkout, honouring the `worktree_include` config key, and log the result.
/// Never fails the create — a missing `.env` is better than no worktree.
pub(crate) fn prepare_new_worktree(source: &Path, dest: &Path) {
    let patterns = super::config::current().worktree_include;
    match copy_included_files(source, dest, &patterns) {
        Ok(report) => {
            if !report.copied.is_empty() || !report.rejected.is_empty() {
                crate::debug::log_to_file(
                    crate::debug::LogLevel::Info,
                    "worktree",
                    &format!(
                        "prepared {}: copied {:?}, kept existing {:?}, rejected {:?}",
                        dest.display(),
                        report.copied,
                        report.skipped_existing,
                        report.rejected
                    ),
                );
            }
        }
        Err(err) => crate::debug::log_to_file(
            crate::debug::LogLevel::Warn,
            "worktree",
            &format!(
                "could not copy included files into {}: {err}",
                dest.display()
            ),
        ),
    }
}

/// Copy included, git-ignored files from `source` (a checkout) into `dest`
/// (a fresh worktree of the same repo). `config_patterns` is the
/// `worktree_include` config value, used when `source` has no
/// `.worktreeinclude`.
pub(crate) fn copy_included_files(
    source: &Path,
    dest: &Path,
    config_patterns: &[String],
) -> std::io::Result<PrepReport> {
    let mut report = PrepReport::default();
    let candidates = included_ignored_files(source, config_patterns)?;
    if candidates.is_empty() {
        return Ok(report);
    }
    let source_root = source.canonicalize()?;
    let dest_root = dest.canonicalize()?;

    for rel in candidates {
        if !is_safe_relative(&rel) {
            report.rejected.push(rel);
            continue;
        }
        let src = source_root.join(&rel);
        let dst = dest_root.join(&rel);

        // Source must be a regular file (not a symlink) that really lives
        // under the source checkout.
        let Ok(meta) = std::fs::symlink_metadata(&src) else {
            continue;
        };
        if !meta.file_type().is_file() {
            report.rejected.push(rel);
            continue;
        }
        match src.canonicalize() {
            Ok(real) if real.starts_with(&source_root) => {}
            _ => {
                report.rejected.push(rel);
                continue;
            }
        }

        if std::fs::symlink_metadata(&dst).is_ok() {
            report.skipped_existing.push(rel);
            continue;
        }

        let Some(parent) = dst.parent() else {
            report.rejected.push(rel);
            continue;
        };
        if !create_dirs_within(&dest_root, parent)? {
            report.rejected.push(rel);
            continue;
        }

        copy_new_file(&src, &dst)?;
        report.copied.push(rel);
    }
    Ok(report)
}

/// Patterns to use: `.worktreeinclude` lines when that file exists, else the
/// config patterns.
fn include_patterns(source: &Path, config_patterns: &[String]) -> Vec<String> {
    match std::fs::read_to_string(source.join(INCLUDE_FILE)) {
        Ok(content) => content
            .lines()
            .map(str::trim_end)
            .filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#'))
            .map(ToOwned::to_owned)
            .collect(),
        Err(_) => config_patterns
            .iter()
            .map(|p| p.trim().to_string())
            .filter(|p| !p.is_empty())
            .collect(),
    }
}

/// Untracked files in `source` that match the include patterns *and* are
/// ignored by the checkout's own ignore rules, as repo-relative paths.
fn included_ignored_files(
    source: &Path,
    config_patterns: &[String],
) -> std::io::Result<Vec<PathBuf>> {
    let patterns = include_patterns(source, config_patterns);
    if patterns.is_empty() {
        return Ok(Vec::new());
    }

    // `--others --ignored` with only our `--exclude` patterns lists untracked
    // files the *include* patterns match (git applies gitignore semantics).
    let mut list = Command::new("git");
    list.args(["ls-files", "-z", "--others", "--ignored"])
        .current_dir(source);
    for pattern in &patterns {
        list.arg(format!("--exclude={pattern}"));
    }
    let output = list.output()?;
    if !output.status.success() {
        return Ok(Vec::new());
    }
    let matched: Vec<String> = output
        .stdout
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect();
    if matched.is_empty() {
        return Ok(Vec::new());
    }

    // Keep only those the checkout itself ignores.
    let mut check = Command::new("git")
        .args(["check-ignore", "-z", "--stdin"])
        .current_dir(source)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()?;
    {
        use std::io::Write;
        let mut stdin = check.stdin.take().expect("piped stdin");
        let mut payload = Vec::new();
        for path in &matched {
            payload.extend_from_slice(path.as_bytes());
            payload.push(0);
        }
        // Writing everything before reading is fine: check-ignore answers
        // only after the input ends, and the input is bounded.
        stdin.write_all(&payload)?;
    }
    let output = check.wait_with_output()?;
    // Exit 1 just means "nothing ignored".
    Ok(output
        .stdout
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| PathBuf::from(String::from_utf8_lossy(s).into_owned()))
        .collect())
}

/// Relative, with only normal components: no `..`, no root, no prefix.
fn is_safe_relative(rel: &Path) -> bool {
    !rel.as_os_str().is_empty() && rel.components().all(|c| matches!(c, Component::Normal(_)))
}

/// Create `dir` (and parents) but only while every existing ancestor below
/// `root` is a real directory, so a symlinked folder in the worktree can't
/// redirect the copy outside it. Returns `false` when that check fails.
fn create_dirs_within(root: &Path, dir: &Path) -> std::io::Result<bool> {
    let Ok(rel) = dir.strip_prefix(root) else {
        return Ok(false);
    };
    let mut current = root.to_path_buf();
    for component in rel.components() {
        let Component::Normal(part) = component else {
            return Ok(false);
        };
        current.push(part);
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => return Ok(false),
            Err(_) => std::fs::create_dir(&current)?,
        }
    }
    Ok(true)
}

/// Copy `src` to a destination that must not exist yet (`create_new`), keeping
/// the source permissions.
fn copy_new_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    let mut input = std::fs::File::open(src)?;
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(dst)?;
    std::io::copy(&mut input, &mut output)?;
    let permissions = input.metadata()?.permissions();
    output.set_permissions(permissions)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn git(dir: &Path, args: &[&str]) {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// A repo with `.env*`, `node_modules/` and `secret/` ignored, plus a
    /// linked worktree to copy into.
    fn fixture() -> (TempDir, PathBuf, PathBuf) {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.email", "t@example.com"]);
        git(&repo, &["config", "user.name", "t"]);
        std::fs::write(
            repo.join(".gitignore"),
            ".env*\nnode_modules/\nsecret/\n!.env.example\n",
        )
        .unwrap();
        std::fs::write(repo.join(".env.example"), "TRACKED=1\n").unwrap();
        std::fs::write(repo.join("README.md"), "hi\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "init"]);
        let wt = tmp.path().join("wt");
        git(
            &repo,
            &["worktree", "add", "-q", "-b", "feat", &wt.to_string_lossy()],
        );
        (tmp, repo, wt)
    }

    fn defaults() -> Vec<String> {
        crate::fork::config::default_worktree_include()
    }

    #[test]
    fn copies_ignored_env_files_including_nested() {
        let (_tmp, repo, wt) = fixture();
        std::fs::write(repo.join(".env"), "A=1\n").unwrap();
        std::fs::create_dir_all(repo.join("apps/web")).unwrap();
        std::fs::write(repo.join("apps/web/.env.local"), "B=2\n").unwrap();
        std::fs::create_dir_all(repo.join("node_modules/x")).unwrap();
        std::fs::write(repo.join("node_modules/x/index.js"), "").unwrap();

        let report = copy_included_files(&repo, &wt, &defaults()).unwrap();
        let mut copied = report.copied.clone();
        copied.sort();
        assert_eq!(
            copied,
            vec![PathBuf::from(".env"), PathBuf::from("apps/web/.env.local")]
        );
        assert_eq!(std::fs::read_to_string(wt.join(".env")).unwrap(), "A=1\n");
        assert!(!wt.join("node_modules").exists());
    }

    #[test]
    fn never_overwrites_or_copies_tracked_files() {
        let (_tmp, repo, wt) = fixture();
        std::fs::write(repo.join(".env"), "SOURCE\n").unwrap();
        std::fs::write(wt.join(".env"), "MINE\n").unwrap();
        // Tracked file that matches the include pattern but is not ignored.
        std::fs::write(repo.join(".env.example"), "CHANGED\n").unwrap();

        let report = copy_included_files(&repo, &wt, &defaults()).unwrap();
        assert!(report.copied.is_empty(), "{report:?}");
        assert_eq!(report.skipped_existing, vec![PathBuf::from(".env")]);
        assert_eq!(std::fs::read_to_string(wt.join(".env")).unwrap(), "MINE\n");
        assert_eq!(
            std::fs::read_to_string(wt.join(".env.example")).unwrap(),
            "TRACKED=1\n"
        );
    }

    #[test]
    fn untracked_but_not_ignored_files_are_not_copied() {
        let (_tmp, repo, wt) = fixture();
        // Matches the include pattern via .worktreeinclude, but not ignored.
        std::fs::write(repo.join(".worktreeinclude"), "notes.txt\n").unwrap();
        std::fs::write(repo.join("notes.txt"), "x\n").unwrap();
        let report = copy_included_files(&repo, &wt, &defaults()).unwrap();
        assert!(report.copied.is_empty());
        assert!(!wt.join("notes.txt").exists());
    }

    #[test]
    fn worktreeinclude_overrides_config_patterns() {
        let (_tmp, repo, wt) = fixture();
        std::fs::write(repo.join(".worktreeinclude"), "# comment\nsecret/\n").unwrap();
        std::fs::create_dir_all(repo.join("secret")).unwrap();
        std::fs::write(repo.join("secret/key.pem"), "k\n").unwrap();
        std::fs::write(repo.join(".env"), "A=1\n").unwrap();

        let report = copy_included_files(&repo, &wt, &defaults()).unwrap();
        assert_eq!(report.copied, vec![PathBuf::from("secret/key.pem")]);
        assert!(!wt.join(".env").exists());
    }

    #[test]
    fn config_patterns_apply_without_worktreeinclude() {
        let (_tmp, repo, wt) = fixture();
        std::fs::create_dir_all(repo.join("secret")).unwrap();
        std::fs::write(repo.join("secret/key.pem"), "k\n").unwrap();
        let report = copy_included_files(&repo, &wt, &["secret/*.pem".to_string()]).unwrap();
        assert_eq!(report.copied, vec![PathBuf::from("secret/key.pem")]);
        assert!(
            copy_included_files(&repo, &wt, &[])
                .unwrap()
                .copied
                .is_empty()
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_never_copied_or_followed() {
        let (tmp, repo, wt) = fixture();
        let outside = tmp.path().join("outside.txt");
        std::fs::write(&outside, "secret\n").unwrap();
        std::os::unix::fs::symlink(&outside, repo.join(".env")).unwrap();
        let report = copy_included_files(&repo, &wt, &defaults()).unwrap();
        assert!(report.copied.is_empty());
        assert!(std::fs::symlink_metadata(wt.join(".env")).is_err());

        // A symlinked directory in the worktree must not redirect writes.
        let target = tmp.path().join("redirect");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::remove_file(repo.join(".env")).unwrap();
        std::fs::create_dir_all(repo.join("apps")).unwrap();
        std::fs::write(repo.join("apps/.env"), "x\n").unwrap();
        std::os::unix::fs::symlink(&target, wt.join("apps")).unwrap();
        let report = copy_included_files(&repo, &wt, &defaults()).unwrap();
        assert!(report.copied.is_empty(), "{report:?}");
        assert!(!target.join(".env").exists());
    }

    #[test]
    fn rejects_unsafe_relative_paths() {
        assert!(is_safe_relative(Path::new("a/b/.env")));
        assert!(!is_safe_relative(Path::new("../x")));
        assert!(!is_safe_relative(Path::new("/etc/passwd")));
        assert!(!is_safe_relative(Path::new("a/../../x")));
        assert!(!is_safe_relative(Path::new("")));
    }
}
