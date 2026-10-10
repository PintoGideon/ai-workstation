//! Incremental fold over an append-only JSONL file (agent transcripts,
//! rollouts), so a 5-second status sweep reads only the bytes appended since
//! the last sweep instead of the whole file (DESIGN.md §12.3).
//!
//! The result is exactly what folding the whole file would give:
//! - only complete lines (up to the last `\n`) are folded into the saved
//!   state; a trailing line still being written is folded into a copy for this
//!   call only, as a whole-file read would see it;
//! - a file that shrank, was replaced (new inode) or was rewritten in place
//!   (the last bytes already folded no longer match) is folded again from the
//!   start;
//! - invalid UTF-8 anywhere makes the call return `None`, like
//!   `fs::read_to_string` failing.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// The lines of `path`, read lazily — for callers that stop at a header near
/// the top of a long transcript. A line that is not UTF-8 is an `Err`; callers
/// treat it as an unreadable file, as `fs::read_to_string` failing did (they
/// now only notice it if it comes before the line they stop at).
pub(crate) fn file_lines(path: &Path) -> Option<std::io::Lines<std::io::BufReader<File>>> {
    use std::io::BufRead;
    Some(std::io::BufReader::new(File::open(path).ok()?).lines())
}

/// Entries kept before the cache starts over (one per transcript path).
const MAX_ENTRIES: usize = 512;

/// Bytes before `offset` re-read and compared to spot an in-place rewrite.
const CHECK_LEN: usize = 64;

struct Entry<S> {
    identity: (u64, u64),
    offset: u64,
    /// The last (up to) [`CHECK_LEN`] bytes folded so far.
    check: Vec<u8>,
    state: S,
    invalid_utf8: bool,
}

pub(crate) struct LineFoldCache<S> {
    entries: HashMap<PathBuf, Entry<S>>,
}

impl<S> Default for LineFoldCache<S> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }
}

#[cfg(unix)]
fn identity(metadata: &std::fs::Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (metadata.dev(), metadata.ino())
}

#[cfg(not(unix))]
fn identity(_metadata: &std::fs::Metadata) -> (u64, u64) {
    (0, 0)
}

impl<S: Clone + Default> LineFoldCache<S> {
    /// Fold `apply` over every line of `path` (as `str::lines` splits them),
    /// reading only what was appended since the previous call.
    pub(crate) fn fold(&mut self, path: &Path, apply: impl Fn(&mut S, &str)) -> Option<S> {
        let mut file = File::open(path).ok()?;
        let metadata = file.metadata().ok()?;
        let len = metadata.len();
        let id = identity(&metadata);

        // Read from just before the saved offset, so the check bytes come
        // along with whatever was appended.
        let mut read = None;
        if let Some(entry) = self.entries.get(path)
            && entry.identity == id
            && entry.offset <= len
        {
            let from = entry.offset - entry.check.len() as u64;
            let mut bytes = Vec::new();
            file.seek(SeekFrom::Start(from)).ok()?;
            file.read_to_end(&mut bytes).ok()?;
            if bytes.starts_with(&entry.check) {
                bytes.drain(..entry.check.len());
                read = Some(bytes);
            }
        }
        let reuse = read.is_some();
        if !reuse {
            if self.entries.len() >= MAX_ENTRIES {
                self.entries.clear();
            }
            self.entries.insert(
                path.to_path_buf(),
                Entry {
                    identity: id,
                    offset: 0,
                    check: Vec::new(),
                    state: S::default(),
                    invalid_utf8: false,
                },
            );
        }
        let entry = self.entries.get_mut(path)?;

        let appended = match read {
            Some(bytes) => bytes,
            None => {
                let mut bytes = Vec::new();
                file.seek(SeekFrom::Start(0)).ok()?;
                file.read_to_end(&mut bytes).ok()?;
                bytes
            }
        };
        let complete_len = appended
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |index| index + 1);
        let (complete, partial) = appended.split_at(complete_len);

        if !complete.is_empty() {
            match std::str::from_utf8(complete) {
                Ok(text) => {
                    for line in text.lines() {
                        apply(&mut entry.state, line);
                    }
                }
                Err(_) => entry.invalid_utf8 = true,
            }
            entry.offset += complete.len() as u64;
            let tail = &complete[complete.len().saturating_sub(CHECK_LEN)..];
            entry.check.extend_from_slice(tail);
            let excess = entry.check.len().saturating_sub(CHECK_LEN);
            entry.check.drain(..excess);
        }
        if entry.invalid_utf8 {
            return None;
        }

        let partial = std::str::from_utf8(partial).ok()?;
        let mut state = entry.state.clone();
        for line in partial.lines() {
            apply(&mut state, line);
        }
        Some(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn whole_file(path: &Path) -> Option<Vec<String>> {
        let content = std::fs::read_to_string(path).ok()?;
        Some(content.lines().map(str::to_string).collect())
    }

    fn push(state: &mut Vec<String>, line: &str) {
        state.push(line.to_string());
    }

    #[test]
    fn appended_reads_match_a_whole_file_read_at_every_step() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        let mut cache = LineFoldCache::<Vec<String>>::default();
        let mut file = File::create(&path).unwrap();
        for chunk in [
            "{\"a\":1}\n",
            "{\"b\":",
            "2}",
            "\n{\"c\":3}\r\n",
            "",
            "tail without newline",
            " continues\nnext\n",
        ] {
            file.write_all(chunk.as_bytes()).unwrap();
            file.flush().unwrap();
            assert_eq!(
                cache.fold(&path, push),
                whole_file(&path),
                "after {chunk:?}"
            );
        }
    }

    #[test]
    fn a_truncated_or_replaced_file_is_read_again_from_the_start() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        let mut cache = LineFoldCache::<Vec<String>>::default();
        std::fs::write(&path, "one\ntwo\nthree\n").unwrap();
        assert_eq!(cache.fold(&path, push), whole_file(&path));
        std::fs::write(&path, "x\n").unwrap();
        assert_eq!(cache.fold(&path, push), whole_file(&path));
        // Rewritten in place, longer than before (same inode).
        std::fs::write(&path, "rewritten one\nrewritten two\n").unwrap();
        assert_eq!(cache.fold(&path, push), whole_file(&path));
        let replacement = dir.path().join("new.jsonl");
        std::fs::write(&replacement, "a\nb\nc\nd\n").unwrap();
        std::fs::rename(&replacement, &path).unwrap();
        assert_eq!(cache.fold(&path, push), whole_file(&path));
    }

    #[test]
    fn invalid_utf8_reads_as_unreadable_like_read_to_string() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.jsonl");
        let mut cache = LineFoldCache::<Vec<String>>::default();
        std::fs::write(&path, b"ok\n\xff\xfe\n").unwrap();
        assert_eq!(cache.fold(&path, push), None);
        assert_eq!(whole_file(&path), None);
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        file.write_all(b"more\n").unwrap();
        assert_eq!(cache.fold(&path, push), None);
    }

    #[test]
    fn a_missing_file_is_none() {
        let mut cache = LineFoldCache::<Vec<String>>::default();
        assert_eq!(
            cache.fold(Path::new("/nonexistent/aiw/t.jsonl"), push),
            None
        );
    }
}
