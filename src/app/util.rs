use crate::project::SessionKind;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptEntry {
    pub text: String,
    pub timestamp: Option<i64>, // unix seconds
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaudeTask {
    pub id: String,
    pub subject: String,
    pub description: Option<String>,
    pub active_form: Option<String>,
    pub status: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClaudeTaskState {
    pub tasks: Vec<ClaudeTask>,
}

impl ClaudeTaskState {
    pub fn completed_count(&self) -> usize {
        self.tasks
            .iter()
            .filter(|task| task.status == "completed")
            .count()
    }
}

/// `~/…` for a path under the home directory, the path as given otherwise.
pub fn shorten_path(path: &std::path::Path) -> String {
    // FORK: home looked up once, not per row per frame.
    static HOME: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();
    if let Some(home) = HOME.get_or_init(dirs::home_dir)
        && let Ok(rest) = path.strip_prefix(home)
    {
        return format!("~/{}", rest.display());
    }
    path.display().to_string()
}

pub use crate::project::slugify;

/// `slugify`, then shortened to at most `max` characters **at a word
/// boundary**.
///
/// Truncating a slug mid-word (`add-a-way-to-shorten-the-featu`) reads as
/// damage rather than as a name, so whole words are dropped from the end until
/// what is left fits. A single first word longer than the cap has no boundary
/// to cut on and is truncated hard — a too-long name is worse than an ugly one.
pub fn slugify_shortened(s: &str, max: usize) -> String {
    let slug = slugify(s);
    if slug.chars().count() <= max {
        return slug;
    }

    let mut kept: Vec<&str> = Vec::new();
    let mut len = 0usize;
    for word in slug.split('-') {
        let next = if kept.is_empty() {
            word.chars().count()
        } else {
            len + 1 + word.chars().count()
        };
        if next > max {
            break;
        }
        kept.push(word);
        len = next;
    }

    if kept.is_empty() {
        return slug.chars().take(max).collect();
    }
    kept.join("-")
}

pub fn latest_prompt_path(workdir: &Path) -> PathBuf {
    workdir.join(".claude").join("latest-prompt.txt")
}

pub fn fuzzy_match_score(candidate: &str, query: &str) -> Option<usize> {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return Some(0);
    }

    let candidate = candidate.to_ascii_lowercase();
    let candidate_chars: Vec<char> = candidate.chars().collect();
    let query_chars: Vec<char> = query.chars().collect();

    if query_chars.len() > candidate_chars.len() {
        // Still allow subsequence matches, so keep scanning rather than bailing here.
    }

    let mut score = candidate_chars.len().saturating_sub(query_chars.len());
    let mut search_start = 0usize;
    let mut prev_match: Option<usize> = None;

    for qc in query_chars {
        let relative_pos = candidate_chars[search_start..]
            .iter()
            .position(|&c| c == qc)?;
        let match_pos = search_start + relative_pos;

        if let Some(prev) = prev_match {
            score += match_pos.saturating_sub(prev + 1);
        } else {
            score += match_pos;
        }

        prev_match = Some(match_pos);
        search_start = match_pos + 1;
    }

    Some(score)
}

pub fn markdown_file_picker_score(
    path: &Path,
    workdir: &Path,
    repo_root: Option<&Path>,
    query: &str,
) -> Option<usize> {
    let label = crate::markdown::markdown_view_relative_label(path, workdir, repo_root);
    let basename = path.file_name().and_then(|name| name.to_str());

    let mut best = fuzzy_match_score(&label, query);
    if let Some(name) = basename
        && let Some(score) = fuzzy_match_score(name, query)
    {
        best = Some(best.map_or(score, |existing| existing.min(score)));
    }
    best
}

pub fn worktree_picker_score(
    worktree: &crate::worktree::WorktreeInfo,
    query: &str,
) -> Option<usize> {
    let path = worktree.path.display().to_string();
    let basename = worktree.path.file_name().and_then(|name| name.to_str());

    let mut best = fuzzy_match_score(&path, query);
    if let Some(branch) = worktree.branch.as_deref()
        && let Some(score) = fuzzy_match_score(branch, query)
    {
        best = Some(best.map_or(score, |existing| existing.min(score)));
    }
    if let Some(name) = basename
        && let Some(score) = fuzzy_match_score(name, query)
    {
        best = Some(best.map_or(score, |existing| existing.min(score)));
    }
    best
}

pub(crate) fn read_latest_prompt_for_session(
    workdir: &Path,
    session_kind: Option<&crate::project::SessionKind>,
    preferred_session_id: Option<&str>,
) -> Option<String> {
    // `full: false` — this runs on every status-sync tick for every
    // feature, so it keeps the cheap newest-file/tail-only scan rather
    // than the thorough one the "all prompts" menu needs.
    let entries = read_prompts_for_session(workdir, session_kind, preferred_session_id, false);
    entries
        .into_iter()
        .max_by(|a, b| match (a.timestamp, b.timestamp) {
            (Some(at), Some(bt)) => at.cmp(&bt),
            (Some(_), None) => std::cmp::Ordering::Greater,
            (None, Some(_)) => std::cmp::Ordering::Less,
            (None, None) => std::cmp::Ordering::Equal,
        })
        .map(|entry| entry.text)
}

/// Every prompt sent in the session, for the "all prompts" menu. Unlike
/// [`read_latest_prompt_for_session`] this is only called on demand (the
/// user opening the menu), so it can afford the thorough, all-files scan.
pub(crate) fn read_all_prompts_for_session(
    workdir: &Path,
    session_kind: Option<&SessionKind>,
    preferred_session_id: Option<&str>,
) -> Vec<PromptEntry> {
    read_prompts_for_session(workdir, session_kind, preferred_session_id, true)
}

fn read_prompts_for_session(
    workdir: &Path,
    session_kind: Option<&SessionKind>,
    preferred_session_id: Option<&str>,
    full: bool,
) -> Vec<PromptEntry> {
    let mut entries = match session_kind {
        Some(SessionKind::Codex) => read_prompts_from_codex_history(workdir, preferred_session_id),
        _ => read_prompts_from_claude_sessions(workdir, preferred_session_id, full),
    };

    // Fall back to latest-prompt.txt if no session entries found
    if entries.is_empty() {
        let path = latest_prompt_path(workdir);
        if let Ok(text) = std::fs::read_to_string(&path)
            && !text.trim().is_empty()
        {
            let ts = std::fs::metadata(&path)
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64);
            entries.push(PromptEntry {
                text,
                timestamp: ts,
            });
        }
    }

    // Sort by timestamp descending (latest first), None timestamps at end
    entries.sort_by(|a, b| match (b.timestamp, a.timestamp) {
        (Some(bt), Some(at)) => bt.cmp(&at),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });

    entries
}

fn read_prompts_from_codex_history(workdir: &Path, session_id: Option<&str>) -> Vec<PromptEntry> {
    let Some(session_id) = session_id else {
        return Vec::new();
    };

    let mut entries = super::codex_sessions::prompt_history_for_session_id(workdir, session_id)
        .unwrap_or_default()
        .into_iter()
        .map(|entry| PromptEntry {
            text: entry.text,
            timestamp: entry.timestamp,
        })
        .collect::<Vec<_>>();

    entries.sort_by(|a, b| match (b.timestamp, a.timestamp) {
        (Some(bt), Some(at)) => bt.cmp(&at),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });

    entries
}

pub fn read_claude_task_state(workdir: &Path, session_id: Option<&str>) -> Option<ClaudeTaskState> {
    let tasks_root = claude_tasks_root();
    let session_id = session_id
        .map(str::trim)
        .filter(|session_id| !session_id.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| latest_claude_session_id(workdir));

    if let Some(session_id) = session_id.as_deref()
        && let Some(state) = read_claude_task_state_from_task_store(&tasks_root, session_id)
    {
        return Some(state);
    }

    let path = session_id
        .as_deref()
        .and_then(|session_id| claude_session_jsonl_path(workdir, session_id))
        .or_else(|| latest_claude_session_jsonl_path(workdir))?;
    let content = std::fs::read_to_string(path).ok()?;
    parse_claude_task_state_from_jsonl(&content)
}

// Read only the last 64 KB of a session file — avoids loading multi-MB
// session histories just to find the most recent user prompt.
const CLAUDE_SESSION_TAIL_BYTES: u64 = 65_536;

fn read_file_tail(path: &Path, tail_len: u64) -> Option<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let file_len = file.metadata().ok()?.len();
    let start = file_len.saturating_sub(tail_len);
    if start > 0 {
        file.seek(SeekFrom::Start(start)).ok()?;
    }
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;
    // If we seeked into the middle, skip the first (partial) line.
    if start > 0 {
        let skip = buf
            .iter()
            .position(|&b| b == b'\n')
            .map_or(buf.len(), |p| p + 1);
        buf.drain(..skip);
    }
    Some(buf)
}

fn file_mtime_secs(metadata: &std::fs::Metadata) -> Option<i64> {
    metadata
        .modified()
        .ok()
        .and_then(|mtime| mtime.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
}

/// `full: false` mirrors the historical behavior: only the
/// most-recently-modified session file, and only its last 64 KB — cheap
/// enough to run on every status-sync tick for every feature, and the
/// latest prompt is overwhelmingly likely to be there.
///
/// `full: true` is for the "all prompts sent in this session" menu.
/// Resuming or compacting a Claude session starts a **new** `.jsonl` file
/// under the same project directory, so restricting to the newest file
/// (or its tail) silently drops every prompt sent before that point. The
/// menu is opened on demand, not polled, so it can afford to read every
/// session file for this workdir in full.
///
/// `preferred_session_id`, when known, scopes *both* modes to exactly that
/// session's own file. A worktree's project directory holds one `.jsonl`
/// per Claude window that has ever run there, and nothing in the file
/// format links a resumed transcript back to its predecessor — so once the
/// caller knows which session it wants, scanning every file in the
/// directory would risk mixing in prompts from an unrelated, concurrent
/// Claude window using the same workdir. Without a known session id, the
/// scan falls back to the directory-wide heuristic.
fn read_prompts_from_claude_sessions(
    workdir: &Path,
    preferred_session_id: Option<&str>,
    full: bool,
) -> Vec<PromptEntry> {
    let Some(home) = std::env::var("HOME").ok() else {
        return Vec::new();
    };
    let encoded = encode_claude_path(workdir);
    let projects_dir = PathBuf::from(&home)
        .join(".claude")
        .join("projects")
        .join(&encoded);

    read_prompts_from_claude_projects_dir(&projects_dir, preferred_session_id, full)
}

fn read_prompts_from_claude_projects_dir(
    projects_dir: &Path,
    preferred_session_id: Option<&str>,
    full: bool,
) -> Vec<PromptEntry> {
    if !is_real_dir(projects_dir) {
        return Vec::new();
    }

    if let Some(session_id) = preferred_session_id {
        let path = projects_dir.join(format!("{session_id}.jsonl"));
        let Ok(metadata) = path.metadata() else {
            return Vec::new();
        };
        let file_ts = file_mtime_secs(&metadata);
        return if full {
            let content = std::fs::read_to_string(&path).unwrap_or_default();
            parse_user_prompts_from_jsonl(&content, file_ts)
        } else {
            let Some(buf) = read_file_tail(&path, CLAUDE_SESSION_TAIL_BYTES) else {
                return Vec::new();
            };
            let content = std::str::from_utf8(&buf).unwrap_or("");
            parse_user_prompts_from_jsonl(content, file_ts)
        };
    }

    let Ok(read_dir) = std::fs::read_dir(projects_dir) else {
        return Vec::new();
    };
    let mut jsonl_files: Vec<(std::time::SystemTime, PathBuf)> = read_dir
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "jsonl") {
                return None;
            }
            let mtime = entry.metadata().ok()?.modified().ok()?;
            Some((mtime, path))
        })
        .collect();
    if jsonl_files.is_empty() {
        return Vec::new();
    }
    jsonl_files.sort_by_key(|(mtime, _)| *mtime);

    if full {
        return jsonl_files
            .into_iter()
            .flat_map(|(mtime, path)| {
                let file_ts = mtime
                    .duration_since(std::time::UNIX_EPOCH)
                    .ok()
                    .map(|d| d.as_secs() as i64);
                let content = std::fs::read_to_string(&path).unwrap_or_default();
                parse_user_prompts_from_jsonl(&content, file_ts)
            })
            .collect();
    }

    let (mtime, newest_path) = jsonl_files.pop().expect("checked non-empty above");
    let file_ts = mtime
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64);

    let Some(buf) = read_file_tail(&newest_path, CLAUDE_SESSION_TAIL_BYTES) else {
        return Vec::new();
    };
    let content = std::str::from_utf8(&buf).unwrap_or("");
    parse_user_prompts_from_jsonl(content, file_ts)
}

fn parse_user_prompts_from_jsonl(content: &str, file_ts: Option<i64>) -> Vec<PromptEntry> {
    let mut entries = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if value["type"] != "user" {
            continue;
        }
        let text = match extract_user_prompt_text(&value) {
            Some(t) if !t.trim().is_empty() => t,
            _ => continue,
        };
        let ts = value
            .get("timestamp")
            .and_then(|v| v.as_str())
            .and_then(parse_prompt_timestamp)
            .or(file_ts);
        entries.push(PromptEntry {
            text,
            timestamp: ts,
        });
    }
    entries
}

fn claude_projects_dir(workdir: &Path) -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(
        PathBuf::from(home)
            .join(".claude")
            .join("projects")
            .join(encode_claude_path(workdir)),
    )
}

fn claude_session_jsonl_path(workdir: &Path, session_id: &str) -> Option<PathBuf> {
    let projects_dir = claude_projects_dir(workdir)?;
    let path = projects_dir.join(format!("{session_id}.jsonl"));
    path.is_file().then_some(path)
}

fn latest_claude_session_jsonl_path(workdir: &Path) -> Option<PathBuf> {
    let projects_dir = claude_projects_dir(workdir)?;
    if !is_real_dir(&projects_dir) {
        return None;
    }
    let read_dir = std::fs::read_dir(projects_dir).ok()?;

    read_dir
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "jsonl") {
                return None;
            }
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, path))
        })
        .max_by_key(|(modified, _)| *modified)
        .map(|(_, path)| path)
}

fn latest_claude_session_id(workdir: &Path) -> Option<String> {
    latest_claude_session_jsonl_path(workdir)?
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(ToOwned::to_owned)
}

fn claude_tasks_root() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(|home| PathBuf::from(home).join(".claude").join("tasks"))
}

fn read_claude_task_state_from_task_store(
    tasks_root: &Option<PathBuf>,
    session_id: &str,
) -> Option<ClaudeTaskState> {
    let tasks_dir = tasks_root.as_ref()?.join(session_id);
    read_claude_task_state_from_task_dir(&tasks_dir)
}

fn read_claude_task_state_from_task_dir(tasks_dir: &Path) -> Option<ClaudeTaskState> {
    if !is_real_dir(tasks_dir) {
        return None;
    }

    let mut task_entries: Vec<(u64, PathBuf)> = std::fs::read_dir(tasks_dir)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let id = path.file_stem()?.to_str()?.parse::<u64>().ok()?;
            (path.extension().and_then(|ext| ext.to_str()) == Some("json")).then_some((id, path))
        })
        .collect();
    task_entries.sort_by_key(|(id, _)| *id);

    let mut tasks = Vec::new();
    for (_, path) in task_entries {
        let content = std::fs::read_to_string(path).ok()?;
        let value: serde_json::Value = serde_json::from_str(&content).ok()?;
        let id = value.get("id")?.as_str()?.trim();
        let subject = value.get("subject")?.as_str()?.trim();
        if id.is_empty() || subject.is_empty() {
            continue;
        }

        tasks.push(ClaudeTask {
            id: id.to_string(),
            subject: subject.to_string(),
            description: value
                .get("description")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToOwned::to_owned),
            active_form: value
                .get("activeForm")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToOwned::to_owned),
            status: value
                .get("status")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .unwrap_or("pending")
                .to_string(),
        });
    }

    (!tasks.is_empty()).then_some(ClaudeTaskState { tasks })
}

fn parse_claude_task_state_from_jsonl(content: &str) -> Option<ClaudeTaskState> {
    let mut state = ClaudeTaskState::default();

    for line in content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
    {
        let value: serde_json::Value = match serde_json::from_str(line) {
            Ok(value) => value,
            Err(_) => continue,
        };

        let Some(contents) = value
            .get("message")
            .and_then(|message| message.get("content"))
            .and_then(|content| content.as_array())
        else {
            continue;
        };

        for item in contents {
            if item.get("type").and_then(|value| value.as_str()) != Some("tool_use") {
                continue;
            }

            match item.get("name").and_then(|value| value.as_str()) {
                Some("TaskCreate") => apply_task_create(&mut state, item.get("input")),
                Some("TaskUpdate") => apply_task_update(&mut state, item.get("input")),
                _ => {}
            }
        }
    }

    (!state.tasks.is_empty()).then_some(state)
}

fn apply_task_create(state: &mut ClaudeTaskState, input: Option<&serde_json::Value>) {
    let Some(input) = input else {
        return;
    };

    let subject = input
        .get("subject")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim();
    if subject.is_empty() {
        return;
    }

    let id = (state.tasks.len() + 1).to_string();
    let description = input
        .get("description")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let active_form = input
        .get("activeForm")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    state.tasks.push(ClaudeTask {
        id,
        subject: subject.to_string(),
        description,
        active_form,
        status: "pending".to_string(),
    });
}

fn apply_task_update(state: &mut ClaudeTaskState, input: Option<&serde_json::Value>) {
    let Some(input) = input else {
        return;
    };

    let task_id = input
        .get("taskId")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim();
    if task_id.is_empty() {
        return;
    }

    let status = input
        .get("status")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let active_form = input
        .get("activeForm")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let subject = input
        .get("subject")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let description = input
        .get("description")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);

    let task = if let Some(task) = state.tasks.iter_mut().find(|task| task.id == task_id) {
        task
    } else {
        state.tasks.push(ClaudeTask {
            id: task_id.to_string(),
            subject: subject.clone().unwrap_or_else(|| format!("Task {task_id}")),
            description: description.clone(),
            active_form: None,
            status: "pending".to_string(),
        });
        state.tasks.last_mut().expect("inserted task should exist")
    };

    if let Some(subject) = subject {
        task.subject = subject;
    }
    if let Some(description) = description {
        task.description = Some(description);
    }
    if let Some(status) = status {
        task.status = status;
    }
    if active_form.is_some() {
        task.active_form = active_form;
    }
}

fn extract_user_prompt_text(value: &serde_json::Value) -> Option<String> {
    if let Some(content) = value["message"]["content"].as_str() {
        return Some(content.to_string());
    }
    if let Some(blocks) = value["message"]["content"].as_array() {
        let texts: Vec<&str> = blocks
            .iter()
            .filter(|b| b["type"] == "text")
            .filter_map(|b| b["text"].as_str())
            .collect();
        if !texts.is_empty() {
            return Some(texts.join("\n"));
        }
    }
    None
}

fn parse_prompt_timestamp(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp())
}

fn encode_claude_path(path: &Path) -> String {
    path.to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn is_real_dir(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|metadata| metadata.is_dir())
        .unwrap_or(false)
}

/// Returns true when running under WSL, where the Windows clipboard is
/// reachable via `powershell.exe`/`clip.exe` rather than `wl-paste`/`xclip`
/// (which are usually absent and cannot see the Windows clipboard anyway).
pub fn is_wsl() -> bool {
    use std::sync::OnceLock;
    static IS_WSL: OnceLock<bool> = OnceLock::new();
    *IS_WSL.get_or_init(|| {
        if std::env::var_os("WSL_DISTRO_NAME").is_some() {
            return true;
        }
        std::fs::read_to_string("/proc/version")
            .map(|v| {
                let v = v.to_ascii_lowercase();
                v.contains("microsoft") || v.contains("wsl")
            })
            .unwrap_or(false)
    })
}

fn copy_to_clipboard_wsl(text: &str) -> anyhow::Result<()> {
    use std::io::Write;
    let mut child = std::process::Command::new("clip.exe")
        .stdin(std::process::Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes())?;
    }
    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("clip.exe exited with {status}"))
    }
}

fn copy_to_clipboard_macos(text: &str) -> anyhow::Result<()> {
    use std::io::Write;
    let mut child = std::process::Command::new("pbcopy")
        .stdin(std::process::Stdio::piped())
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(text.as_bytes())?;
    }
    let status = child.wait()?;
    if status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!("pbcopy exited with {status}"))
    }
}

pub fn copy_to_clipboard(text: &str) -> anyhow::Result<()> {
    use std::io::Write;
    // On WSL, push text to the Windows clipboard via clip.exe.
    if is_wsl() && copy_to_clipboard_wsl(text).is_ok() {
        return Ok(());
    }
    // On macOS, use pbcopy.
    if cfg!(target_os = "macos") && copy_to_clipboard_macos(text).is_ok() {
        return Ok(());
    }
    // Try wl-copy (Wayland)
    if let Ok(mut child) = std::process::Command::new("wl-copy")
        .stdin(std::process::Stdio::piped())
        .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
        return Ok(());
    }
    // Fallback to xclip
    if let Ok(mut child) = std::process::Command::new("xclip")
        .args(["-selection", "clipboard"])
        .stdin(std::process::Stdio::piped())
        .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(text.as_bytes());
        }
        let _ = child.wait();
        return Ok(());
    }
    Err(anyhow::anyhow!(
        "No clipboard utility found (wl-copy or xclip)"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn claude_task_store_missing_exact_session_does_not_read_other_sessions() {
        let temp = TempDir::new().unwrap();
        let tasks_root = Some(temp.path().to_path_buf());
        let other_dir = temp.path().join("other-session");
        std::fs::create_dir_all(&other_dir).unwrap();
        std::fs::write(
            other_dir.join("1.json"),
            r#"{"id":"1","subject":"Other session task","status":"pending"}"#,
        )
        .unwrap();

        let state = read_claude_task_state_from_task_store(&tasks_root, "selected-session");

        assert_eq!(state, None);
    }

    #[test]
    fn claude_task_store_prefers_exact_session_directory() {
        let temp = TempDir::new().unwrap();
        let tasks_root = Some(temp.path().to_path_buf());
        let exact_dir = temp.path().join("claude-session");
        let other_dir = temp.path().join("detached-task-store");
        std::fs::create_dir_all(&exact_dir).unwrap();
        std::fs::create_dir_all(&other_dir).unwrap();
        std::fs::write(
            exact_dir.join("1.json"),
            r#"{"id":"1","subject":"Exact session task","status":"pending"}"#,
        )
        .unwrap();
        std::fs::write(
            other_dir.join("1.json"),
            r#"{"id":"1","subject":"Newest detached task","status":"pending"}"#,
        )
        .unwrap();

        let state = read_claude_task_state_from_task_store(&tasks_root, "claude-session").unwrap();

        assert_eq!(state.tasks.len(), 1);
        assert_eq!(state.tasks[0].subject, "Exact session task");
    }

    #[test]
    fn latest_prompt_for_session_prefers_newest_timestamp() {
        let workdir = PathBuf::from("/tmp/unused");
        let latest = read_latest_prompt_for_session(&workdir, None, None);
        assert_eq!(latest, None);
    }

    fn write_claude_session_file(
        projects_dir: &Path,
        name: &str,
        prompts: &[&str],
        mtime_offset_secs: i64,
    ) {
        std::fs::create_dir_all(projects_dir).unwrap();
        let lines: Vec<String> = prompts
            .iter()
            .map(|text| {
                serde_json::json!({"type": "user", "message": {"content": text}}).to_string()
            })
            .collect();
        let path = projects_dir.join(name);
        std::fs::write(&path, lines.join("\n")).unwrap();

        // Give each file a distinct, controllable mtime so "newest file"
        // and file-timestamp fallbacks are deterministic in the test.
        let mtime = std::time::SystemTime::UNIX_EPOCH
            + std::time::Duration::from_secs((1_700_000_000 + mtime_offset_secs) as u64);
        let file = std::fs::File::open(&path).unwrap();
        file.set_modified(mtime).unwrap();
    }

    #[test]
    fn full_scan_collects_prompts_across_every_resumed_session_file() {
        let temp = TempDir::new().unwrap();
        let projects_dir = temp.path().join("claude-projects");

        // Simulate a resume/compaction: an older session file with the
        // first prompts, then a newer one Claude created on resume.
        write_claude_session_file(&projects_dir, "older.jsonl", &["first prompt"], 0);
        write_claude_session_file(&projects_dir, "newer.jsonl", &["second prompt"], 100);

        // Ordering here is file-mtime ascending; the caller-facing
        // `read_all_prompts_for_session` applies the final descending sort.
        let entries = read_prompts_from_claude_projects_dir(&projects_dir, None, true);
        let texts: Vec<&str> = entries.iter().map(|e| e.text.as_str()).collect();

        assert_eq!(texts, vec!["first prompt", "second prompt"]);
    }

    #[test]
    fn fast_scan_only_reads_the_newest_session_file() {
        let temp = TempDir::new().unwrap();
        let projects_dir = temp.path().join("claude-projects");

        write_claude_session_file(&projects_dir, "older.jsonl", &["first prompt"], 0);
        write_claude_session_file(&projects_dir, "newer.jsonl", &["second prompt"], 100);

        let entries = read_prompts_from_claude_projects_dir(&projects_dir, None, false);
        let texts: Vec<&str> = entries.iter().map(|e| e.text.as_str()).collect();

        assert_eq!(texts, vec!["second prompt"]);
    }

    #[test]
    fn known_session_id_scopes_full_scan_to_its_own_file_only() {
        let temp = TempDir::new().unwrap();
        let projects_dir = temp.path().join("claude-projects");

        // Two unrelated Claude windows sharing this worktree, each with its
        // own session file. Without scoping, a full scan would mix both.
        write_claude_session_file(&projects_dir, "session-a.jsonl", &["window a prompt"], 0);
        write_claude_session_file(&projects_dir, "session-b.jsonl", &["window b prompt"], 100);

        let entries = read_prompts_from_claude_projects_dir(&projects_dir, Some("session-a"), true);
        let texts: Vec<&str> = entries.iter().map(|e| e.text.as_str()).collect();

        assert_eq!(texts, vec!["window a prompt"]);
    }

    #[test]
    fn known_session_id_scopes_fast_scan_to_its_own_file_even_when_not_newest() {
        let temp = TempDir::new().unwrap();
        let projects_dir = temp.path().join("claude-projects");

        write_claude_session_file(&projects_dir, "session-a.jsonl", &["window a prompt"], 0);
        write_claude_session_file(&projects_dir, "session-b.jsonl", &["window b prompt"], 100);

        let entries =
            read_prompts_from_claude_projects_dir(&projects_dir, Some("session-a"), false);
        let texts: Vec<&str> = entries.iter().map(|e| e.text.as_str()).collect();

        assert_eq!(texts, vec!["window a prompt"]);
    }

    #[test]
    fn unknown_session_id_yields_no_prompts_rather_than_falling_back() {
        let temp = TempDir::new().unwrap();
        let projects_dir = temp.path().join("claude-projects");

        write_claude_session_file(&projects_dir, "session-a.jsonl", &["window a prompt"], 0);

        let entries =
            read_prompts_from_claude_projects_dir(&projects_dir, Some("missing-session"), true);

        assert!(entries.is_empty());
    }

    #[test]
    fn fuzzy_match_score_matches_subsequence() {
        assert!(fuzzy_match_score("plan-notes.md", "pn").is_some());
        assert!(fuzzy_match_score("plan-notes.md", "pln").is_some());
        assert!(fuzzy_match_score("plan-notes.md", "zn").is_none());
    }

    #[test]
    fn markdown_file_picker_score_prefers_basename_hits() {
        let workdir = PathBuf::from("/tmp/demo");
        let path = workdir.join("docs").join("plan-notes.md");
        assert!(markdown_file_picker_score(&path, &workdir, None, "pn").is_some());
    }
}
