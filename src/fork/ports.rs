//! A block of consecutive TCP ports per feature, so dev servers started by
//! parallel agents don't fight over `:3000`.
//!
//! Each feature gets [`BLOCK_SIZE`] ports starting at or above
//! [`RANGE_START`], skipping blocks that contain a port browsers refuse or that
//! is commonly taken by system services ([`UNSAFE_PORTS`]). Reservations are
//! persisted in `<config dir>/aiw-ports.json` and every read-modify-write holds
//! an exclusive lock on `aiw-ports.lock`, so two AIW processes never hand out
//! the same block. The first port and the block size reach every tmux window
//! of the feature as `AIW_PORT` and `AIW_PORT_COUNT`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub(crate) const BLOCK_SIZE: u16 = 10;
pub(crate) const RANGE_START: u16 = 4100;
pub(crate) const ENV_PORT: &str = "AIW_PORT";
pub(crate) const ENV_PORT_COUNT: &str = "AIW_PORT_COUNT";

/// Ports never handed out: browser-blocked ports in the range plus ports
/// commonly held by system services (5000 is macOS AirPlay).
pub(crate) const UNSAFE_PORTS: &[u16] = &[
    3659, 4045, 5000, 5060, 5061, 6000, 6566, 6665, 6666, 6667, 6668, 6669, 6697, 7000,
];

const STORE_FILE: &str = "aiw-ports.json";
const LOCK_FILE: &str = "aiw-ports.lock";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PortBlock {
    pub feature_id: String,
    pub tmux_session: String,
    pub start: u16,
    pub count: u16,
}

impl PortBlock {
    pub(crate) fn env_pairs(&self) -> Vec<(String, String)> {
        vec![
            (ENV_PORT.to_string(), self.start.to_string()),
            (ENV_PORT_COUNT.to_string(), self.count.to_string()),
        ]
    }

    fn overlaps(&self, start: u16, count: u16) -> bool {
        let (a0, a1) = (
            u32::from(self.start),
            u32::from(self.start) + u32::from(self.count),
        );
        let (b0, b1) = (u32::from(start), u32::from(start) + u32::from(count));
        a0 < b1 && b0 < a1
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Store {
    #[serde(default)]
    blocks: Vec<PortBlock>,
}

fn store_dir() -> PathBuf {
    crate::project::aiw_config_dir()
}

/// Reserve (or look up) the block for a feature and remember which tmux
/// session it runs in. Call before creating the feature's tmux session.
pub(crate) fn reserve(feature_id: &str, tmux_session: &str) -> std::io::Result<PortBlock> {
    reserve_in(&store_dir(), feature_id, tmux_session)
}

/// Free a feature's block (feature deleted).
pub(crate) fn release(feature_id: &str) {
    if let Err(err) = release_in(&store_dir(), feature_id) {
        crate::debug::log_to_file(
            crate::debug::LogLevel::Warn,
            "ports",
            &format!("failed to release port block for feature {feature_id}: {err}"),
        );
    }
}

/// `AIW_PORT` / `AIW_PORT_COUNT` for the feature running in `tmux_session`,
/// or nothing when it has no block.
pub(crate) fn env_for_tmux_session(tmux_session: &str) -> Vec<(String, String)> {
    env_for_tmux_session_in(&store_dir(), tmux_session)
}

/// Reserve the feature's block before its tmux session is created, logging
/// (not failing the start) when the store can't be written.
pub(crate) fn prepare_feature_session(feature_id: &str, tmux_session: &str) {
    if let Err(err) = reserve(feature_id, tmux_session) {
        crate::debug::log_to_file(
            crate::debug::LogLevel::Warn,
            "ports",
            &format!("failed to reserve a port block for {tmux_session}: {err}"),
        );
    }
}

fn with_locked_store<T>(
    dir: &Path,
    write: bool,
    f: impl FnOnce(&mut Store) -> T,
) -> std::io::Result<T> {
    std::fs::create_dir_all(dir)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(LOCK_FILE))?;
    lock.lock()?;
    let path = dir.join(STORE_FILE);
    let mut store: Store = match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Store::default(),
        Err(err) => return Err(err),
    };
    let result = f(&mut store);
    if write {
        let tmp = dir.join(format!("{STORE_FILE}.tmp-{}", std::process::id()));
        let rendered = serde_json::to_string_pretty(&store).map_err(std::io::Error::other)?;
        std::fs::write(&tmp, rendered + "\n")?;
        std::fs::rename(&tmp, &path)?;
    }
    Ok(result)
}

fn reserve_in(dir: &Path, feature_id: &str, tmux_session: &str) -> std::io::Result<PortBlock> {
    with_locked_store(dir, true, |store| {
        if let Some(block) = store.blocks.iter_mut().find(|b| b.feature_id == feature_id) {
            if block.tmux_session != tmux_session {
                block.tmux_session = tmux_session.to_string();
            }
            return Ok(block.clone());
        }
        let start = next_free_start(&store.blocks).ok_or_else(|| {
            std::io::Error::other("no free port block left between 4100 and 65535")
        })?;
        // A session name is unique per running feature; drop a stale claim on
        // it so lookups by session can't return another feature's block.
        for block in store.blocks.iter_mut() {
            if block.tmux_session == tmux_session {
                block.tmux_session.clear();
            }
        }
        let block = PortBlock {
            feature_id: feature_id.to_string(),
            tmux_session: tmux_session.to_string(),
            start,
            count: BLOCK_SIZE,
        };
        store.blocks.push(block.clone());
        Ok(block)
    })?
}

fn release_in(dir: &Path, feature_id: &str) -> std::io::Result<()> {
    if !dir.join(STORE_FILE).exists() {
        return Ok(());
    }
    with_locked_store(dir, true, |store| {
        store.blocks.retain(|b| b.feature_id != feature_id);
    })
}

fn env_for_tmux_session_in(dir: &Path, tmux_session: &str) -> Vec<(String, String)> {
    if tmux_session.is_empty() || !dir.join(STORE_FILE).exists() {
        return Vec::new();
    }
    with_locked_store(dir, false, |store| {
        store
            .blocks
            .iter()
            .find(|b| b.tmux_session == tmux_session)
            .map(PortBlock::env_pairs)
            .unwrap_or_default()
    })
    .unwrap_or_default()
}

/// Lowest block start >= [`RANGE_START`] that overlaps no reserved block and
/// contains no unsafe port.
fn next_free_start(taken: &[PortBlock]) -> Option<u16> {
    let mut start = u32::from(RANGE_START);
    let size = u32::from(BLOCK_SIZE);
    while start + size - 1 <= u32::from(u16::MAX) {
        let end = start + size; // exclusive
        if let Some(bad) = UNSAFE_PORTS
            .iter()
            .map(|p| u32::from(*p))
            .filter(|p| *p >= start && *p < end)
            .max()
        {
            start = bad + 1;
            continue;
        }
        let candidate = start as u16;
        if let Some(blocking) = taken
            .iter()
            .filter(|b| b.overlaps(candidate, BLOCK_SIZE))
            .map(|b| u32::from(b.start) + u32::from(b.count))
            .max()
        {
            start = blocking;
            continue;
        }
        return Some(candidate);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn block_ports(block: &PortBlock) -> std::ops::Range<u16> {
        block.start..block.start + block.count
    }

    #[test]
    fn first_block_starts_at_range_start() {
        let dir = TempDir::new().unwrap();
        let block = reserve_in(dir.path(), "f1", "aiw-a").unwrap();
        assert_eq!(block.start, 4100);
        assert_eq!(block.count, 10);
        assert_eq!(
            block.env_pairs(),
            vec![
                ("AIW_PORT".to_string(), "4100".to_string()),
                ("AIW_PORT_COUNT".to_string(), "10".to_string())
            ]
        );
    }

    #[test]
    fn blocks_are_disjoint_and_stable() {
        let dir = TempDir::new().unwrap();
        let a = reserve_in(dir.path(), "f1", "aiw-a").unwrap();
        let b = reserve_in(dir.path(), "f2", "aiw-b").unwrap();
        assert_eq!(b.start, 4110);
        assert!(!a.overlaps(b.start, b.count));
        // Re-reserving returns the same block, and follows a renamed session.
        let again = reserve_in(dir.path(), "f1", "aiw-a2").unwrap();
        assert_eq!(again.start, a.start);
        assert_eq!(env_for_tmux_session_in(dir.path(), "aiw-a").len(), 0);
        assert_eq!(
            env_for_tmux_session_in(dir.path(), "aiw-a2")[0].1,
            "4100".to_string()
        );
    }

    #[test]
    fn released_block_is_reused() {
        let dir = TempDir::new().unwrap();
        reserve_in(dir.path(), "f1", "aiw-a").unwrap();
        reserve_in(dir.path(), "f2", "aiw-b").unwrap();
        release_in(dir.path(), "f1").unwrap();
        assert!(env_for_tmux_session_in(dir.path(), "aiw-a").is_empty());
        let c = reserve_in(dir.path(), "f3", "aiw-c").unwrap();
        assert_eq!(c.start, 4100);
    }

    #[test]
    fn unsafe_ports_are_never_handed_out() {
        let mut taken = Vec::new();
        for i in 0..400 {
            let Some(start) = next_free_start(&taken) else {
                break;
            };
            let block = PortBlock {
                feature_id: format!("f{i}"),
                tmux_session: format!("s{i}"),
                start,
                count: BLOCK_SIZE,
            };
            for port in block_ports(&block) {
                assert!(!UNSAFE_PORTS.contains(&port), "handed out unsafe {port}");
            }
            taken.push(block);
        }
        // 4100.. → the block containing 5000 is skipped.
        assert!(taken.iter().all(|b| !b.overlaps(5000, 1)));
        assert!(taken.iter().any(|b| b.start == 5001));
    }

    #[test]
    fn concurrent_reservations_never_collide() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_path_buf();
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || {
                    reserve_in(&path, &format!("f{i}"), &format!("s{i}")).unwrap()
                })
            })
            .collect();
        let mut starts: Vec<u16> = handles
            .into_iter()
            .map(|h| h.join().unwrap().start)
            .collect();
        starts.sort();
        starts.dedup();
        assert_eq!(starts.len(), 8);
    }

    #[test]
    fn missing_store_means_no_env() {
        let dir = TempDir::new().unwrap();
        assert!(env_for_tmux_session_in(dir.path(), "aiw-x").is_empty());
        release_in(dir.path(), "nope").unwrap();
        assert!(!dir.path().join(STORE_FILE).exists());
    }
}
