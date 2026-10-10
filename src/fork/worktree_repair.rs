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

/// What state `feature`'s checkout is in.
pub(crate) fn inspect(feature: &Feature) -> Checkout {
    if !feature.is_worktree {
        return Checkout::Intact;
    }
    inspect_dir(&feature.workdir)
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
        let feature = feature_at(tmp.path().join("gone"), false);
        assert_eq!(inspect(&feature), Checkout::Intact);
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
