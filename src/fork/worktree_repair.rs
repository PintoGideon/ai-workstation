//! FORK: recreate a feature's worktree when its checkout vanished on disk.
//!
//! A worktree directory can disappear while AIW is not looking (`rm -rf`, a
//! cleanup script, a machine that was wiped). Starting the feature then used
//! to "succeed": writing the hook files recreated the folder, tmux started in
//! it, and the agent ran in a directory holding nothing but AIW's helper
//! files while git listed the worktree as prunable.
//!
//! Before a start, a worktree feature whose directory has no `.git` is
//! inspected:
//! - the directory is missing, or holds only files AIW itself writes → the
//!   stale git registration is pruned and the worktree recreated on the
//!   feature's branch (which still exists in the repository);
//! - the directory holds anything else → the start is refused with a message
//!   naming the folder, because deleting it could lose the user's work.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use crate::project::Feature;
use crate::traits::WorktreeOps;

/// Entries AIW writes into a checkout on its own; a folder holding only these
/// is AIW's doing, not the user's.
const AIW_OWNED_ENTRIES: &[&str] = &[
    ".agents",
    ".aiw",
    ".claude",
    ".codex",
    ".cursor",
    ".gemini",
    ".gitignore",
];

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Checkout {
    /// A git checkout (or not a worktree feature at all): nothing to do.
    Intact,
    /// Gone, or only AIW's helper files are left: safe to recreate.
    Missing,
    /// No `.git`, but other files are there: do not touch it.
    Unsafe(Vec<String>),
}

/// What state `feature`'s checkout is in. A checkout has "vanished" only
/// when git still has it registered (`git worktree list`) and the folder is
/// gone: that is what `rm -rf` leaves behind, and what `recreate` prunes and
/// adds again. A path git never knew (or already pruned) is left alone.
pub(crate) fn inspect(feature: &Feature, repo: &Path) -> Checkout {
    if !feature.is_worktree || !repo.join(".git").exists() {
        return Checkout::Intact;
    }
    match inspect_dir(&feature.workdir) {
        Checkout::Intact => Checkout::Intact,
        state if registered(repo, &feature.workdir) => state,
        _ => Checkout::Intact,
    }
}

/// Whether git lists `workdir` as one of `repo`'s worktrees. git prints the
/// path as registered (resolved), while AIW's may go through a symlink, so
/// both are compared through their (existing) parent folder.
fn registered(repo: &Path, workdir: &Path) -> bool {
    let Ok(output) = std::process::Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(repo)
        .output()
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let wanted = resolved(workdir);
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.strip_prefix("worktree "))
        .any(|path| resolved(Path::new(path)) == wanted)
}

/// `path` with its parent canonicalized (the path itself may not exist).
fn resolved(path: &Path) -> PathBuf {
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => std::fs::canonicalize(parent)
            .map(|p| p.join(name))
            .unwrap_or_else(|_| path.to_path_buf()),
        _ => path.to_path_buf(),
    }
}

pub(crate) fn inspect_dir(workdir: &Path) -> Checkout {
    if workdir.join(".git").exists() {
        return Checkout::Intact;
    }
    let Ok(entries) = std::fs::read_dir(workdir) else {
        return Checkout::Missing;
    };
    let foreign: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| !AIW_OWNED_ENTRIES.contains(&name.as_str()))
        .collect();
    if foreign.is_empty() {
        Checkout::Missing
    } else {
        Checkout::Unsafe(foreign)
    }
}

/// Recreate the checkout for `feature` (whose [`inspect`] said `Missing`):
/// clear whatever AIW left in the folder, prune git's stale registration and
/// add the worktree again on the feature's branch. Returns the new path (the
/// same one unless the worktree location setting changed since).
pub(crate) fn recreate(
    worktree: &dyn WorktreeOps,
    repo: &Path,
    project_name: &str,
    feature: &Feature,
) -> Result<PathBuf> {
    if feature.workdir.exists() {
        std::fs::remove_dir_all(&feature.workdir)?;
    }
    worktree.prune(repo)?;
    let name = crate::project::worktree_name(project_name, &feature.name);
    worktree.create(repo, &name, &feature.branch)
}

/// The message for a checkout that cannot be recreated automatically.
pub(crate) fn unsafe_message(feature: &Feature, foreign: &[String]) -> String {
    format!(
        "'{}' has no git checkout at {} but the folder still holds {} — move it away (or `git worktree repair`) and start again",
        feature.name,
        feature.workdir.display(),
        foreign.join(", ")
    )
}

/// Refuse to start on an unsafe checkout, so the caller can `?` it.
pub(crate) fn refuse(feature: &Feature, foreign: &[String]) -> Result<()> {
    bail!(unsafe_message(feature, foreign))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{AgentKind, VibeMode};
    use crate::traits::MockWorktreeOps;

    fn feature_at(workdir: PathBuf, is_worktree: bool) -> Feature {
        Feature::new(
            "hx-12".into(),
            "hx-12".into(),
            workdir,
            is_worktree,
            VibeMode::Vibe,
            false,
            false,
            AgentKind::Claude,
            false,
            false,
        )
    }

    #[test]
    fn a_repo_dir_feature_is_never_inspected() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".git")).unwrap();
        let feature = feature_at(tmp.path().join("gone"), false);
        assert_eq!(inspect(&feature, tmp.path()), Checkout::Intact);
    }

    fn git(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {:?}", out.stderr);
    }

    #[test]
    fn only_a_registered_worktree_of_a_real_repository_counts_as_vanished() {
        let tmp = tempfile::tempdir().unwrap();
        let feature = feature_at(tmp.path().join("gone"), true);
        // No `.git` in the repository folder: nothing to recreate from.
        assert_eq!(inspect(&feature, tmp.path()), Checkout::Intact);

        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(
            &repo,
            &[
                "-c",
                "user.email=t@x",
                "-c",
                "user.name=t",
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                "base",
            ],
        );
        // A path git never registered is left alone.
        assert_eq!(inspect(&feature, &repo), Checkout::Intact);

        let wt = tmp.path().join("wt");
        git(
            &repo,
            &["worktree", "add", "-q", wt.to_str().unwrap(), "-b", "hx-12"],
        );
        let feature = feature_at(wt.clone(), true);
        assert_eq!(inspect(&feature, &repo), Checkout::Intact);
        std::fs::remove_dir_all(&wt).unwrap();
        assert_eq!(
            inspect(&feature, &repo),
            Checkout::Missing,
            "registered but gone"
        );
        git(&repo, &["worktree", "prune"]);
        assert_eq!(
            inspect(&feature, &repo),
            Checkout::Intact,
            "pruned: nothing to recreate"
        );
    }

    #[test]
    fn a_checkout_with_a_git_file_is_intact() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(".git"), "gitdir: /elsewhere").unwrap();
        assert_eq!(inspect_dir(tmp.path()), Checkout::Intact);
    }

    #[test]
    fn a_missing_dir_or_only_aiw_files_is_missing() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(inspect_dir(&tmp.path().join("gone")), Checkout::Missing);
        std::fs::create_dir_all(tmp.path().join(".codex")).unwrap();
        std::fs::create_dir_all(tmp.path().join(".agents")).unwrap();
        std::fs::write(tmp.path().join(".gitignore"), "").unwrap();
        assert_eq!(inspect_dir(tmp.path()), Checkout::Missing);
    }

    #[test]
    fn other_files_make_the_folder_unsafe_to_replace() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join(".codex")).unwrap();
        std::fs::write(tmp.path().join("notes.txt"), "mine").unwrap();
        assert_eq!(
            inspect_dir(tmp.path()),
            Checkout::Unsafe(vec!["notes.txt".to_string()])
        );
        let feature = feature_at(tmp.path().to_path_buf(), true);
        let message = unsafe_message(&feature, &["notes.txt".to_string()]);
        assert!(
            message.contains("notes.txt") && message.contains("hx-12"),
            "{message}"
        );
    }

    #[test]
    fn recreate_clears_prunes_and_adds_on_the_branch() {
        let tmp = tempfile::tempdir().unwrap();
        let workdir = tmp.path().join("wt");
        std::fs::create_dir_all(workdir.join(".codex")).unwrap();
        let repo = tmp.path().join("repo");
        let feature = feature_at(workdir.clone(), true);

        let mut worktree = MockWorktreeOps::new();
        let expected_repo = repo.clone();
        worktree
            .expect_prune()
            .withf(move |r| r == expected_repo)
            .times(1)
            .returning(|_| Ok(()));
        let expected_repo = repo.clone();
        let new_path = workdir.clone();
        worktree
            .expect_create()
            .withf(move |r, name, branch| {
                r == expected_repo && name == "rs-hx-12" && branch == "hx-12"
            })
            .times(1)
            .returning(move |_, _, _| Ok(new_path.clone()));

        let path = recreate(&worktree, &repo, "RS", &feature).unwrap();
        assert_eq!(path, workdir);
        assert!(!workdir.exists(), "AIW's leftovers are cleared first");
    }
}
