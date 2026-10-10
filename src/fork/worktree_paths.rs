//! Where AIW puts feature worktrees, and which folders it owns.
//!
//! New worktrees live outside the repository:
//! `<data-local-dir>/ai_workstation/worktrees/<project-slug>/<name>`
//! (Linux `~/.local/share/…`, macOS `~/Library/Application Support/…`), or
//! under the `worktree_base_dir` config key. Features created by AIW keep their
//! stored `<repo>/.worktrees/<name>` paths, so both locations are "app-owned":
//! scans cover both, and cleanup only ever deletes inside them.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};

use super::config::AiwConfig;

pub(crate) const APP_DIR_NAME: &str = "ai_workstation";
const LEGACY_DIR_NAME: &str = ".worktrees";
/// How deep leftover scans descend below an app-owned root. Branch names with
/// slashes (`feat/x`) nest worktrees one level per slash.
const SCAN_DEPTH: usize = 4;

/// The default worktree base directory (no config override).
pub(crate) fn default_base_dir() -> PathBuf {
    #[cfg(test)]
    {
        test_data_root().join(APP_DIR_NAME).join("worktrees")
    }
    #[cfg(not(test))]
    {
        dirs::data_local_dir()
            .or_else(|| dirs::home_dir().map(|home| home.join(".local").join("share")))
            .unwrap_or_else(std::env::temp_dir)
            .join(APP_DIR_NAME)
            .join("worktrees")
    }
}

/// Per-process stand-in for the data dir while unit tests run, so the suite
/// never creates worktrees under the developer's real `~/.local/share`.
#[cfg(test)]
pub(crate) fn test_data_root() -> &'static PathBuf {
    static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    ROOT.get_or_init(|| {
        let root = std::env::temp_dir().join(format!("aiw-test-data-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&root);
        root
    })
}

/// The base directory new worktrees go under, honouring `worktree_base_dir`.
pub(crate) fn base_dir() -> PathBuf {
    base_dir_from(&super::config::current())
}

fn base_dir_from(config: &AiwConfig) -> PathBuf {
    config
        .worktree_base_dir
        .as_deref()
        .map(str::trim)
        .filter(|raw| !raw.is_empty())
        .map(super::paths::expand_user_path)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(default_base_dir)
}

/// A stable, readable directory name for a project: the repo folder name plus
/// a short hash of its full path, so two repos both called `api` never share
/// a worktree folder.
pub(crate) fn project_slug(repo: &Path) -> String {
    let name = repo
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut slug: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    slug = slug
        .split('-')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    if slug.is_empty() {
        slug = "project".to_string();
    }
    let digest = Sha256::digest(repo.to_string_lossy().as_bytes());
    let hash: String = digest.iter().take(4).map(|b| format!("{b:02x}")).collect();
    format!("{slug}-{hash}")
}

/// Where a new worktree called `name` for `repo` is created.
pub(crate) fn new_worktree_path(repo: &Path, name: &str) -> PathBuf {
    new_worktree_path_in(&base_dir(), repo, name)
}

fn new_worktree_path_in(base: &Path, repo: &Path, name: &str) -> PathBuf {
    base.join(project_slug(repo)).join(name)
}

/// AIW's in-repo worktree folder, still used by features created before AIW.
pub(crate) fn legacy_root(repo: &Path) -> PathBuf {
    repo.join(LEGACY_DIR_NAME)
}

/// Every folder AIW owns for `repo`'s worktrees: the legacy in-repo folder,
/// the configured base, and the default base (in case the config changed).
pub(crate) fn app_owned_roots(repo: &Path) -> Vec<PathBuf> {
    app_owned_roots_with(repo, &base_dir())
}

fn app_owned_roots_with(repo: &Path, base: &Path) -> Vec<PathBuf> {
    let mut roots = vec![
        legacy_root(repo),
        base.join(project_slug(repo)),
        default_base_dir().join(project_slug(repo)),
    ];
    let mut seen = HashSet::new();
    roots.retain(|root| seen.insert(root.clone()));
    roots
}

/// True when `path` has no `..`/`.` components, so `starts_with` checks on it
/// can't be fooled lexically.
fn is_normal(path: &Path) -> bool {
    path.is_absolute()
        && path.components().all(|c| {
            matches!(
                c,
                Component::RootDir | Component::Prefix(_) | Component::Normal(_)
            )
        })
}

/// The app-owned root strictly containing `path`, if any.
pub(crate) fn owning_root(roots: &[PathBuf], path: &Path) -> Option<PathBuf> {
    if !is_normal(path) {
        return None;
    }
    roots
        .iter()
        .find(|root| path != root.as_path() && path.starts_with(root))
        .cloned()
}

/// Remove `path` and then its parents while they are empty directories,
/// stopping at (and including) the app-owned root that contains it. Never
/// touches anything outside an app-owned root and never deletes a non-empty
/// directory. Used after a failed `git worktree add` and after a feature's
/// worktree is removed.
pub(crate) fn prune_empty_dirs(repo: &Path, path: &Path) {
    prune_empty_dirs_within(&app_owned_roots(repo), path);
}

fn prune_empty_dirs_within(roots: &[PathBuf], path: &Path) {
    let Some(root) = owning_root(roots, path) else {
        return;
    };
    let mut current = Some(path.to_path_buf());
    while let Some(dir) = current {
        if !dir.starts_with(&root) {
            break;
        }
        // Only real directories: a symlink here is not ours to follow.
        match std::fs::symlink_metadata(&dir) {
            Ok(meta) if meta.is_dir() => {
                // `remove_dir` refuses non-empty directories, which is the
                // stopping condition.
                if std::fs::remove_dir(&dir).is_err() {
                    break;
                }
            }
            Ok(_) => break,
            Err(_) => {}
        }
        if dir == root {
            break;
        }
        current = dir.parent().map(Path::to_path_buf);
    }
}

/// Directories inside `repo`'s app-owned worktree folders that no git
/// worktree or feature accounts for: folders holding a `.git` entry that is
/// not in `known`, and empty folders left by an interrupted create.
pub(crate) fn leftover_dirs(repo: &Path, known: &HashSet<PathBuf>) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for root in app_owned_roots(repo) {
        scan(&root, known, 0, &mut found);
    }
    found
}

fn scan(dir: &Path, known: &HashSet<PathBuf>, depth: usize, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let path = entry.path();
        if known.contains(&path) {
            continue;
        }
        if path.join(".git").exists() {
            found.push(path);
            continue;
        }
        let is_empty = std::fs::read_dir(&path)
            .map(|mut it| it.next().is_none())
            .unwrap_or(false);
        if is_empty {
            found.push(path);
        } else if depth + 1 < SCAN_DEPTH {
            scan(&path, known, depth + 1, found);
        }
    }
}

/// The checkout where `branch` is already checked out, parsed from
/// `git worktree list --porcelain` output.
pub(crate) fn branch_checkout_in_porcelain(porcelain: &str, branch: &str) -> Option<PathBuf> {
    let wanted = format!("branch refs/heads/{branch}");
    let mut current: Option<PathBuf> = None;
    for line in porcelain.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            current = Some(PathBuf::from(path));
        } else if line == wanted {
            return current;
        }
    }
    None
}

/// Where `branch` is already checked out among `repo`'s worktrees, if it is.
pub(crate) fn branch_checked_out_at(repo: &Path, branch: &str) -> Option<PathBuf> {
    let output = std::process::Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(repo)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    branch_checkout_in_porcelain(&String::from_utf8_lossy(&output.stdout), branch)
}

/// The message shown when a branch is already checked out elsewhere.
pub(crate) fn branch_in_use_message(branch: &str, path: Option<&Path>) -> String {
    let location = path
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "another worktree".to_string());
    format!(
        "Branch '{branch}' is already checked out in {location} — choose a new branch name or turn off 'use worktree'"
    )
}

/// The checkout path named in git's "branch already checked out" error, in
/// either the old ("is already checked out at '<path>'") or the git 2.4x+
/// ("is already used by worktree at '<path>'") wording.
pub(crate) fn branch_in_use_path_from_git_error(detail: &str) -> Option<Option<PathBuf>> {
    for marker in [
        "is already used by worktree at",
        "is already checked out at",
    ] {
        if let Some(idx) = detail.find(marker) {
            let rest = detail[idx + marker.len()..].trim();
            let path = rest
                .trim_start_matches(['\'', '"', '`'])
                .split(['\'', '"', '`', '\n'])
                .next()
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(PathBuf::from);
            return Some(path);
        }
    }
    if detail.contains("already checked out") {
        return Some(None);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn slug_is_readable_and_distinguishes_same_named_repos() {
        let a = project_slug(Path::new("/home/me/work/My API"));
        let b = project_slug(Path::new("/home/me/play/My API"));
        assert!(a.starts_with("my-api-"), "{a}");
        assert_ne!(a, b);
        assert_eq!(a, project_slug(Path::new("/home/me/work/My API")));
        assert!(project_slug(Path::new("/")).starts_with("project-"));
    }

    #[test]
    fn new_worktree_path_is_outside_the_repo() {
        let repo = Path::new("/home/me/repo");
        let path = new_worktree_path_in(Path::new("/data/acc/worktrees"), repo, "repo-feat");
        assert!(path.starts_with("/data/acc/worktrees"));
        assert!(!path.starts_with(repo));
        assert!(path.ends_with("repo-feat"));
    }

    #[test]
    fn base_dir_honours_config_and_expands_tilde() {
        let config = AiwConfig {
            worktree_base_dir: Some("/srv/wt".into()),
            ..AiwConfig::default()
        };
        assert_eq!(base_dir_from(&config), PathBuf::from("/srv/wt"));

        let relative = AiwConfig {
            worktree_base_dir: Some("relative/wt".into()),
            ..AiwConfig::default()
        };
        assert_eq!(base_dir_from(&relative), default_base_dir());
        assert_eq!(base_dir_from(&AiwConfig::default()), default_base_dir());

        if let Some(home) = dirs::home_dir() {
            let tilde = AiwConfig {
                worktree_base_dir: Some("~/wt".into()),
                ..AiwConfig::default()
            };
            assert_eq!(base_dir_from(&tilde), home.join("wt"));
        }
    }

    #[test]
    fn default_base_dir_is_sandboxed_in_tests() {
        assert!(default_base_dir().starts_with(std::env::temp_dir()));
        assert!(default_base_dir().ends_with("ai_workstation/worktrees"));
    }

    #[test]
    fn owning_root_rejects_outside_and_dotdot_paths() {
        let repo = Path::new("/r/repo");
        let roots = app_owned_roots_with(repo, Path::new("/b"));
        assert!(owning_root(&roots, Path::new("/r/repo/.worktrees/x")).is_some());
        assert!(owning_root(&roots, &Path::new("/b").join(project_slug(repo)).join("x")).is_some());
        assert!(owning_root(&roots, Path::new("/r/repo/.worktrees")).is_none());
        assert!(owning_root(&roots, Path::new("/r/repo/src")).is_none());
        assert!(owning_root(&roots, Path::new("/r/repo/.worktrees/../src")).is_none());
        assert!(owning_root(&roots, Path::new("/b/other-slug/x")).is_none());
    }

    #[test]
    fn prune_removes_only_empty_dirs_inside_the_root() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        let legacy = legacy_root(&repo);
        let nested = legacy.join("feat").join("x");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(repo.join("keep.txt"), "x").unwrap();
        let roots = app_owned_roots_with(&repo, &tmp.path().join("base"));

        prune_empty_dirs_within(&roots, &nested);
        assert!(!legacy.exists(), "empty .worktrees/ should be removed");
        assert!(repo.join("keep.txt").exists());

        // A non-empty sibling stops the climb.
        let sibling = legacy.join("other");
        std::fs::create_dir_all(&sibling).unwrap();
        std::fs::write(sibling.join("f"), "x").unwrap();
        let empty = legacy.join("gone");
        std::fs::create_dir_all(&empty).unwrap();
        prune_empty_dirs_within(&roots, &empty);
        assert!(!empty.exists());
        assert!(sibling.join("f").exists());
        assert!(legacy.exists());

        // Paths outside the roots are never touched.
        let outside = tmp.path().join("elsewhere");
        std::fs::create_dir_all(&outside).unwrap();
        prune_empty_dirs_within(&roots, &outside);
        assert!(outside.exists());
    }

    #[test]
    fn porcelain_parser_finds_branch_checkout() {
        let porcelain = "worktree /repo\nHEAD abc\nbranch refs/heads/main\n\nworktree /wt/feat\nHEAD def\nbranch refs/heads/feat\n\nworktree /wt/detached\nHEAD 123\ndetached\n";
        assert_eq!(
            branch_checkout_in_porcelain(porcelain, "main"),
            Some(PathBuf::from("/repo"))
        );
        assert_eq!(
            branch_checkout_in_porcelain(porcelain, "feat"),
            Some(PathBuf::from("/wt/feat"))
        );
        assert_eq!(branch_checkout_in_porcelain(porcelain, "fea"), None);
    }

    #[test]
    fn git_error_parser_matches_old_and_new_wording() {
        let new = "fatal: 'main' is already used by worktree at '/home/me/repo'";
        assert_eq!(
            branch_in_use_path_from_git_error(new),
            Some(Some(PathBuf::from("/home/me/repo")))
        );
        let old = "fatal: 'main' is already checked out at '/home/me/repo'";
        assert_eq!(
            branch_in_use_path_from_git_error(old),
            Some(Some(PathBuf::from("/home/me/repo")))
        );
        assert_eq!(
            branch_in_use_path_from_git_error("fatal: branch already checked out"),
            Some(None)
        );
        assert_eq!(branch_in_use_path_from_git_error("fatal: other"), None);
    }

    #[test]
    fn leftover_scan_reports_unknown_and_empty_dirs() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("repo");
        let legacy = legacy_root(&repo);
        let known_wt = legacy.join("known");
        let unknown_wt = legacy.join("unknown");
        let empty = legacy.join("ns").join("empty");
        for dir in [&known_wt, &unknown_wt, &empty] {
            std::fs::create_dir_all(dir).unwrap();
        }
        std::fs::write(known_wt.join(".git"), "gitdir: x").unwrap();
        std::fs::write(unknown_wt.join(".git"), "gitdir: y").unwrap();
        let known: HashSet<PathBuf> = [known_wt.clone()].into_iter().collect();
        let mut found = leftover_dirs(&repo, &known);
        found.sort();
        assert_eq!(found, vec![empty, unknown_wt]);
    }

    #[test]
    fn branch_in_use_message_names_the_checkout() {
        let msg = branch_in_use_message("main", Some(Path::new("/home/me/repo")));
        assert_eq!(
            msg,
            "Branch 'main' is already checked out in /home/me/repo — choose a new branch name or turn off 'use worktree'"
        );
    }
}
