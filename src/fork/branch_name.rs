//! FORK: validate a feature/branch name before git sees it.
//!
//! A name with a space (or any other character git refuses) used to reach
//! `git worktree add`, whose raw error then surfaced in the dialog. These
//! are the rules of `git check-ref-format --branch`, implemented locally so
//! the dialog can say what is wrong in plain words, plus a length cap: the
//! name becomes a tmux session name, a worktree folder and a branch, and a
//! 200-character feature name serves none of them.

/// Longest feature name accepted.
pub const MAX_LEN: usize = 120;

/// Why `name` cannot be a branch name, in the words the dialog shows.
/// `Ok(())` means git will accept it.
pub fn validate_branch_name(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Feature name cannot be empty".to_string());
    }
    if name.chars().count() > MAX_LEN {
        return Err(format!(
            "Feature name is too long ({} characters, max {MAX_LEN})",
            name.chars().count()
        ));
    }
    if name.starts_with('-') {
        return Err("Feature name cannot start with '-'".to_string());
    }
    if name.starts_with('/') || name.ends_with('/') {
        return Err("Feature name cannot start or end with '/'".to_string());
    }
    if name.ends_with('.') {
        return Err("Feature name cannot end with '.'".to_string());
    }
    if name.ends_with(".lock") || name.contains(".lock/") {
        return Err("Feature name cannot end a part with '.lock'".to_string());
    }
    if name == "@" {
        return Err("Feature name cannot be just '@'".to_string());
    }
    if name.contains("..") {
        return Err("Feature name cannot contain '..'".to_string());
    }
    if name.contains("@{") {
        return Err("Feature name cannot contain '@{'".to_string());
    }
    if name.contains("//") {
        return Err("Feature name cannot contain '//'".to_string());
    }
    if name.contains("/.") || name.starts_with('.') {
        return Err("Feature name parts cannot start with '.'".to_string());
    }
    for c in name.chars() {
        if c.is_whitespace() {
            return Err("Feature name cannot contain spaces".to_string());
        }
        if c.is_control() || c == '\u{7f}' {
            return Err("Feature name cannot contain control characters".to_string());
        }
        if matches!(c, '~' | '^' | ':' | '?' | '*' | '[' | '\\') {
            return Err(format!("Feature name cannot contain '{c}'"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_friendly_names_pass() {
        for name in [
            "feature",
            "fix/login-page",
            "release/v1.2.3",
            "feat_42",
            "Ünïcödé-名前",
            "a.b",
            "x/y/z",
            "has@sign",
        ] {
            assert_eq!(validate_branch_name(name), Ok(()), "{name}");
        }
    }

    #[test]
    fn names_git_refuses_get_a_plain_message() {
        let cases = [
            ("", "empty"),
            ("   ", "empty"),
            ("bad name", "spaces"),
            ("tab\tname", "spaces"),
            ("-leading", "start with '-'"),
            ("a..b", "'..'"),
            ("a~b", "'~'"),
            ("a^b", "'^'"),
            ("a:b", "':'"),
            ("a?b", "'?'"),
            ("a*b", "'*'"),
            ("a[b", "'['"),
            ("a\\b", "'\\'"),
            ("trailing/", "'/'"),
            ("/leading", "'/'"),
            ("ends.", "end with '.'"),
            ("name.lock", "'.lock'"),
            ("dir.lock/x", "'.lock'"),
            ("a@{b", "'@{'"),
            ("@", "'@'"),
            ("a//b", "'//'"),
            (".hidden", "start with '.'"),
            ("a/.b", "start with '.'"),
            ("ctl\u{1}x", "control"),
        ];
        for (name, expect) in cases {
            let err = validate_branch_name(name).unwrap_err();
            assert!(err.contains(expect), "{name:?}: {err}");
        }
    }

    #[test]
    fn length_is_capped_with_a_clear_message() {
        let ok = "x".repeat(MAX_LEN);
        assert_eq!(validate_branch_name(&ok), Ok(()));
        let long = "x".repeat(200);
        let err = validate_branch_name(&long).unwrap_err();
        assert!(err.contains("too long"), "{err}");
        assert!(err.contains("200"), "{err}");
        assert!(err.contains("120"), "{err}");
    }
}
