//! POSIX shell quoting, shared by every module that builds a command line.

/// Single-quote `value` for a POSIX shell. Always quotes, so the output is
/// safe to splice anywhere a word is expected.
pub fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Like [`quote`], but leaves a plain word (letters, digits and `-_./=:@%+,`)
/// unquoted so displayed commands stay readable.
pub fn quote_min(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:@%+,".contains(c))
    {
        return value.to_string();
    }
    quote(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_always_wraps_and_escapes_single_quotes() {
        assert_eq!(quote("plain"), "'plain'");
        assert_eq!(quote("it's"), "'it'\\''s'");
        assert_eq!(quote(""), "''");
    }

    #[test]
    fn quote_min_leaves_plain_words_alone() {
        assert_eq!(quote_min("a-b.c/d=e:f@g%h+i,j"), "a-b.c/d=e:f@g%h+i,j");
        assert_eq!(quote_min("has space"), "'has space'");
        assert_eq!(quote_min(""), "''");
    }
}
