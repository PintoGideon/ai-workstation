//! Path helpers for user-typed paths.

use std::path::{Path, PathBuf};

/// Expand a leading `~`, `~/…`, `$HOME`, `$HOME/…` or `${HOME}/…` in a path the
/// user typed. Anything else (including `~otheruser/…`) is returned unchanged.
pub(crate) fn expand_user_path(raw: &str) -> PathBuf {
    expand_user_path_with(raw, dirs::home_dir().as_deref())
}

fn expand_user_path_with(raw: &str, home: Option<&Path>) -> PathBuf {
    let trimmed = raw.trim();
    let Some(home) = home else {
        return PathBuf::from(trimmed);
    };
    for prefix in ["~", "$HOME", "${HOME}"] {
        if trimmed == prefix {
            return home.to_path_buf();
        }
        if let Some(rest) = trimmed.strip_prefix(prefix)
            && let Some(rest) = rest.strip_prefix('/')
        {
            return home.join(rest);
        }
    }
    PathBuf::from(trimmed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expand(raw: &str) -> PathBuf {
        expand_user_path_with(raw, Some(Path::new("/home/me")))
    }

    #[test]
    fn expands_tilde_forms() {
        assert_eq!(expand("~"), PathBuf::from("/home/me"));
        assert_eq!(expand("~/repo"), PathBuf::from("/home/me/repo"));
        assert_eq!(expand("~/"), PathBuf::from("/home/me/"));
        assert_eq!(expand("  ~/code/x  "), PathBuf::from("/home/me/code/x"));
    }

    #[test]
    fn expands_home_variable_forms() {
        assert_eq!(expand("$HOME"), PathBuf::from("/home/me"));
        assert_eq!(expand("$HOME/repo"), PathBuf::from("/home/me/repo"));
        assert_eq!(expand("${HOME}/repo"), PathBuf::from("/home/me/repo"));
    }

    #[test]
    fn leaves_other_paths_alone() {
        assert_eq!(expand("/abs/~/x"), PathBuf::from("/abs/~/x"));
        assert_eq!(expand("~other/repo"), PathBuf::from("~other/repo"));
        assert_eq!(expand("$HOMEDIR/x"), PathBuf::from("$HOMEDIR/x"));
        assert_eq!(expand("relative/dir"), PathBuf::from("relative/dir"));
        assert_eq!(expand(""), PathBuf::from(""));
    }

    #[test]
    fn without_home_returns_input() {
        assert_eq!(
            expand_user_path_with("~/repo", None),
            PathBuf::from("~/repo")
        );
    }
}
