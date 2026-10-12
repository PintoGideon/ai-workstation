//! FORK: Claude's own worktrees, created in AIW's layout (DESIGN.md §19.6).
//!
//! Claude Code makes worktrees by itself: `claude --worktree`, the
//! `EnterWorktree` tool, a subagent with `isolation: worktree`, a workflow
//! agent, background-session isolation. By default those land in
//! `<repo>/.claude/worktrees/<name>` and AIW knows nothing about them. A
//! `WorktreeCreate` hook replaces Claude's `git worktree add` entirely; AIW
//! passes one (and its `WorktreeRemove` partner) in every Claude session's
//! `--settings` file, so the worktree is created here instead:
//!
//! - in AIW's worktree folder (`fork::worktree_paths`), as a real linked git
//!   worktree on branch `worktree-<name>` (Claude's own naming), branched from
//!   the requesting checkout's `HEAD` — a subagent works on the feature, not
//!   on `main`;
//! - with the repo's included files copied (`.worktreeinclude` /
//!   `worktree_include`, which Claude skips when a hook creates the worktree)
//!   and `.claude/settings.local.json` merged, through the same
//!   `WorktreeManager::create_from` a new feature uses;
//! - with its own port block, written to `<worktree>/.aiw/ports.env`
//!   (`.aiw/` is git-excluded) and exported in terminals AIW opens there;
//! - recorded in `aiw.db` (`aiw_claude_worktrees`) by the hook process itself
//!   and announced on AIW's socket, so a dashboard that is not running picks
//!   it up on its next start.
//!
//! The hook contract (hooks.md, "WorktreeCreate" / "WorktreeRemove"):
//! `WorktreeCreate` gets `name` (a slug such as `bold-oak-a3f2`) plus the
//! common fields (`session_id`, `cwd`, …; `agent_id` / `agent_type` inside a
//! subagent) on stdin and must print the worktree's absolute path as the last
//! non-empty stdout line; a failure or no path fails the create. The path must
//! be normalized and must not pass through a symlink below the repository
//! root. `WorktreeRemove` gets `worktree_path`; exit 0 means removed, a
//! non-zero exit with the directory still present refuses the removal. There
//! is no force flag: AIW removes only a clean worktree, and deletes the branch
//! only when it holds no commit beyond the base it was made from.
//!
//! Everything that touches git or the disk is in [`create`] / [`remove`]; the
//! naming and parsing above them are pure and unit-tested.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU8, Ordering};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

/// The hook script AIW installs next to its other Claude hook scripts.
pub const SCRIPT_NAME: &str = "worktree-hook.sh";
pub const SCRIPT: &str = include_str!("../../scripts/worktree-hook.sh");
/// The IPC message the hook sends after it created or removed a worktree.
pub const IPC_TYPE: &str = "claude-worktree";
/// Seconds Claude waits for the create hook (`git worktree add` plus the
/// included files) and the remove hook.
pub const CREATE_TIMEOUT_SECS: u64 = 120;
pub const REMOVE_TIMEOUT_SECS: u64 = 60;
/// Where the port block is written inside the worktree.
pub const PORTS_FILE: &str = ".aiw/ports.env";
/// Prefix of a Claude worktree's directory under AIW's worktree folder.
pub const DIR_PREFIX: &str = "claude-";
/// Prefix of its branch: Claude's own `worktree-<name>`.
pub const BRANCH_PREFIX: &str = "worktree-";
/// How many `-2`, `-3`, … suffixes are tried before giving up on a name.
const MAX_SUFFIX: usize = 50;
/// Longest slug kept from Claude's name.
const MAX_SLUG: usize = 48;

/// 0 = not decided (on), 1 = off, 2 = on. Set once at start-up from
/// `claude_worktree_hooks`.
static ENABLED: AtomicU8 = AtomicU8::new(0);

/// Decide for this process whether Claude launches carry the hooks.
pub fn set_enabled(on: bool) {
    ENABLED.store(if on { 2 } else { 1 }, Ordering::Relaxed);
}

#[cfg(test)]
thread_local! {
    static TEST_OVERRIDE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// Whether Claude launches carry the worktree hooks (default on).
pub fn enabled() -> bool {
    #[cfg(test)]
    if let Some(forced) = TEST_OVERRIDE.with(|cell| cell.get()) {
        return forced;
    }
    ENABLED.load(Ordering::Relaxed) != 1
}

/// Force the flag for the current test thread.
#[cfg(test)]
pub fn force_for_test(on: Option<bool>) {
    TEST_OVERRIDE.with(|cell| cell.set(on));
}

/// The two hook entries for a `--settings` file.
pub fn hook_entries(hooks_dir: &Path) -> Vec<(&'static str, Value)> {
    let script = hooks_dir.join(SCRIPT_NAME);
    let entry = |action: &str, timeout: u64| {
        json!({
            "hooks": [crate::app::setup::claude_exec_hook_with_timeout(&script, &[action], timeout)]
        })
    };
    vec![
        ("WorktreeCreate", entry("create", CREATE_TIMEOUT_SECS)),
        ("WorktreeRemove", entry("remove", REMOVE_TIMEOUT_SECS)),
    ]
}

/// Install (or refresh) the hook script, executable, when its content
/// changed. Errors are logged; a missing script makes Claude's create fail
/// loudly, which is the right outcome for a broken install.
pub fn ensure_script(hooks_dir: &Path) {
    let path = hooks_dir.join(SCRIPT_NAME);
    if std::fs::read_to_string(&path).ok().as_deref() == Some(SCRIPT) {
        return;
    }
    let result = (|| -> std::io::Result<()> {
        std::fs::create_dir_all(hooks_dir)?;
        let tmp = hooks_dir.join(format!(".{SCRIPT_NAME}.tmp.{}", std::process::id()));
        std::fs::write(&tmp, SCRIPT)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        }
        std::fs::rename(&tmp, &path)
    })();
    if let Err(err) = result {
        crate::debug::log_to_file(
            crate::debug::LogLevel::Warn,
            "claude_worktrees",
            &format!("could not install {}: {err}", path.display()),
        );
    }
}

// ── pure parts ──────────────────────────────────────────────────

/// What a `WorktreeCreate` hook was asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateRequest {
    /// Claude's name, as given.
    pub name: String,
    /// The checkout the request came from.
    pub cwd: PathBuf,
    /// Claude's conversation id.
    pub claude_session_id: Option<String>,
    /// Set when the hook fired inside a subagent.
    pub agent_type: Option<String>,
    /// The AIW feature session that launched Claude (`AIW_FEATURE_SESSION_ID`).
    pub aiw_session_id: Option<String>,
}

fn string_field(payload: &Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Read the create hook's stdin JSON. `cwd_fallback` is the hook process's
/// own directory (Claude runs hooks in the session's directory).
pub fn parse_create(
    payload: &Value,
    cwd_fallback: &Path,
    aiw_session_id: Option<String>,
) -> Result<CreateRequest> {
    let name = string_field(payload, "name").context("the WorktreeCreate input has no `name`")?;
    let cwd = string_field(payload, "cwd")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| cwd_fallback.to_path_buf());
    Ok(CreateRequest {
        name,
        cwd,
        claude_session_id: string_field(payload, "session_id"),
        agent_type: string_field(payload, "agent_type"),
        aiw_session_id,
    })
}

/// Read the remove hook's stdin JSON: the absolute `worktree_path`.
pub fn parse_remove(payload: &Value) -> Result<PathBuf> {
    let path = string_field(payload, "worktree_path")
        .map(PathBuf::from)
        .context("the WorktreeRemove input has no `worktree_path`")?;
    if !path.is_absolute() {
        bail!("worktree_path {} is not absolute", path.display());
    }
    Ok(path)
}

/// Claude's name as a safe directory and branch component: lowercase
/// letters, digits, `.`, `_` and `-`, no leading/trailing separators, never
/// `.` or `..`, at most [`MAX_SLUG`] characters. Empty becomes `worktree`.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() || c == '_' || c == '.' {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let mut out: String = out
        .trim_matches(|c| c == '-' || c == '.' || c == '_')
        .chars()
        .take(MAX_SLUG)
        .collect();
    while out.ends_with(['-', '.', '_']) {
        out.pop();
    }
    // `git check-ref-format` refuses `..` anywhere in a ref.
    while out.contains("..") {
        out = out.replace("..", ".");
    }
    if out.is_empty() || out == "." {
        "worktree".to_string()
    } else {
        out
    }
}

/// The `n`-th candidate (1-based) directory and branch for `slug`.
pub fn candidate(slug: &str, n: usize) -> (String, String) {
    if n <= 1 {
        (
            format!("{DIR_PREFIX}{slug}"),
            format!("{BRANCH_PREFIX}{slug}"),
        )
    } else {
        (
            format!("{DIR_PREFIX}{slug}-{n}"),
            format!("{BRANCH_PREFIX}{slug}-{n}"),
        )
    }
}

/// The first candidate whose branch and directory are both free.
pub fn pick_names(
    slug: &str,
    branch_taken: impl Fn(&str) -> bool,
    dir_taken: impl Fn(&str) -> bool,
) -> Option<(String, String)> {
    (1..=MAX_SUFFIX)
        .map(|n| candidate(slug, n))
        .find(|(dir, branch)| !branch_taken(branch) && !dir_taken(dir))
}

/// What the row says the worktree is for: `subagent` (with Claude's agent
/// type when it has one) or `worktree` (a session that moved into one).
pub fn origin_label(request: &CreateRequest) -> String {
    match request.agent_type.as_deref() {
        Some(kind) if !kind.is_empty() => format!("subagent {kind}"),
        // Claude names a subagent's worktree `agent-<hex>`.
        _ if request.name.starts_with("agent-") => "subagent".to_string(),
        _ => "worktree".to_string(),
    }
}

/// The contents of `.aiw/ports.env`.
pub fn ports_env_text(start: u16, count: u16) -> String {
    format!(
        "# AIW port block for this Claude worktree (DESIGN.md §19.6).\n\
         # `. .aiw/ports.env` in a shell; AIW's terminals here have it already.\n\
         export {}={start}\nexport {}={count}\n",
        super::ports::ENV_PORT,
        super::ports::ENV_PORT_COUNT,
    )
}

/// One worktree a Claude session created through the hook.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaudeWorktree {
    pub id: String,
    /// The main checkout of the repository.
    pub repo: PathBuf,
    /// The worktree, as printed to Claude (canonical).
    pub path: PathBuf,
    pub branch: String,
    /// The commit it was branched from.
    pub base: String,
    /// Claude's name for it.
    pub name: String,
    /// `subagent …` or `worktree`.
    pub origin: String,
    /// The AIW feature session whose Claude asked for it.
    pub parent_session_id: Option<String>,
    pub claude_session_id: Option<String>,
    /// The checkout the request came from (resolves the parent feature when
    /// the session id is unknown).
    pub requested_cwd: PathBuf,
    pub port_start: Option<u16>,
    pub port_count: Option<u16>,
    pub created_at: String,
}

impl ClaudeWorktree {
    /// `⎇ <name> · <origin>` for the sidebar.
    pub fn row_text(&self) -> String {
        format!("⎇ {} · {}", self.name, self.origin)
    }

    /// The key its port block is reserved under.
    pub fn port_key(&self) -> String {
        port_key(&self.id)
    }
}

fn port_key(id: &str) -> String {
    format!("claude-worktree:{id}")
}

/// What a removal did besides deleting the worktree.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RemoveOutcome {
    pub branch_deleted: Option<String>,
    /// The branch kept because it has commits beyond its base, and how many.
    pub branch_kept: Option<(String, usize)>,
    /// The directory was already gone; only the bookkeeping was cleared.
    pub already_gone: bool,
}

impl RemoveOutcome {
    /// One line for a toast or the hook's stderr.
    pub fn summary(&self, path: &Path) -> String {
        let what = if self.already_gone {
            format!("forgot {} (already gone)", path.display())
        } else {
            format!("removed {}", path.display())
        };
        match (&self.branch_deleted, &self.branch_kept) {
            (Some(branch), _) => format!("{what}; deleted branch {branch}"),
            (None, Some((branch, n))) => format!(
                "{what}; kept branch {branch} ({n} commit{} beyond its base)",
                if *n == 1 { "" } else { "s" }
            ),
            _ => what,
        }
    }
}

// ── git ─────────────────────────────────────────────────────────

/// Run git in `dir` without inherited repository overrides: a hook runs with
/// Claude's environment, and a stray `GIT_DIR` would point every command at
/// the wrong repository.
fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .with_context(|| format!("could not run git {}", args.join(" ")))?;
    if !output.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The main checkout of the repository containing `dir`: the first
/// `worktree` line of `git worktree list --porcelain`.
pub(crate) fn main_checkout(dir: &Path) -> Result<PathBuf> {
    let porcelain = git(dir, &["worktree", "list", "--porcelain"])
        .with_context(|| format!("{} is not inside a git repository", dir.display()))?;
    porcelain
        .lines()
        .find_map(|line| line.strip_prefix("worktree "))
        .map(|p| canonical(Path::new(p)))
        .with_context(|| format!("{} is not inside a git repository", dir.display()))
}

fn branch_exists(repo: &Path, branch: &str) -> bool {
    git(
        repo,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ],
    )
    .is_ok()
}

/// Uncommitted or untracked files in `checkout`, as `git status --porcelain`
/// lines (ignored files, such as the included `.env`, do not count).
fn dirty_entries(checkout: &Path) -> Result<Vec<String>> {
    Ok(git(
        checkout,
        &["status", "--porcelain", "--untracked-files=normal"],
    )?
    .lines()
    .filter(|l| !l.trim().is_empty())
    .map(str::to_string)
    .collect())
}

fn is_linked_worktree(path: &Path) -> bool {
    let (Ok(top), Ok(common), Ok(dir)) = (
        git(path, &["rev-parse", "--show-toplevel"]),
        git(
            path,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        ),
        git(path, &["rev-parse", "--path-format=absolute", "--git-dir"]),
    ) else {
        return false;
    };
    canonical(Path::new(&top)) == canonical(path)
        && canonical(Path::new(&common)) != canonical(Path::new(&dir))
}

// ── create / remove ─────────────────────────────────────────────

/// Where the hook keeps its record and announces it.
#[derive(Debug, Clone)]
pub struct HookPaths {
    pub db: PathBuf,
    pub socket: PathBuf,
}

impl HookPaths {
    pub fn current() -> Self {
        Self {
            db: crate::project::db_path(),
            socket: crate::ipc::socket_path(),
        }
    }
}

fn open_db(paths: &HookPaths) -> Option<crate::db::AiwDb> {
    match crate::db::AiwDb::open_for_hook(&paths.db) {
        Ok(db) => Some(db),
        Err(err) => {
            eprintln!("aiw: not recorded in AIW (no store: {err})");
            None
        }
    }
}

fn announce(paths: &HookPaths, event: &str, id: &str, path: &Path) {
    let message = json!({
        "type": IPC_TYPE,
        "event": event,
        "id": id,
        "path": path.to_string_lossy(),
    });
    // Fire-and-forget: a dashboard that is not running reads the table at
    // its next start.
    let _ = crate::ipc::send(&paths.socket, &message.to_string());
}

/// Create the worktree Claude asked for and record it. Prints nothing; the
/// caller prints the path.
pub fn create(request: &CreateRequest, paths: &HookPaths) -> Result<ClaudeWorktree> {
    let checkout = git(&request.cwd, &["rev-parse", "--show-toplevel"])
        .map(|p| canonical(Path::new(&p)))
        .with_context(|| format!("{} is not inside a git repository", request.cwd.display()))?;
    let repo = main_checkout(&checkout)?;
    let db = open_db(paths);

    // Claude asks again for a name it already has (a resumed `--worktree`
    // session): hand back the same worktree.
    if let Some(db) = db.as_ref()
        && let Ok(Some(existing)) = db.claude_worktree_by_name(&repo, &request.name)
        && existing.path.is_dir()
    {
        return Ok(existing);
    }

    let base = git(&checkout, &["rev-parse", "--verify", "HEAD^{commit}"]).with_context(|| {
        format!(
            "{} has no commit to branch a worktree from",
            checkout.display()
        )
    })?;
    let slug = slug(&request.name);
    let (dir, branch) = pick_names(
        &slug,
        |branch| branch_exists(&repo, branch),
        |dir| super::worktree_paths::new_worktree_path(&repo, dir).exists(),
    )
    .with_context(|| format!("no free worktree name left for '{slug}'"))?;

    let created = crate::worktree::WorktreeManager::create_from(&repo, &dir, &branch, &base)?;
    let path = canonical(&created);
    super::git_exclude::exclude_aiw_helper_files(&path);

    let id = format!("cwt-{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    let (port_start, port_count) = match super::ports::reserve(&port_key(&id), "") {
        Ok(block) => {
            let file = path.join(PORTS_FILE);
            if let Some(parent) = file.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(err) = std::fs::write(&file, ports_env_text(block.start, block.count)) {
                eprintln!("aiw: could not write {}: {err}", file.display());
            }
            (Some(block.start), Some(block.count))
        }
        Err(err) => {
            eprintln!("aiw: no port block for {}: {err}", path.display());
            (None, None)
        }
    };

    let record = ClaudeWorktree {
        id,
        repo,
        path,
        branch,
        base,
        name: request.name.clone(),
        origin: origin_label(request),
        parent_session_id: request.aiw_session_id.clone(),
        claude_session_id: request.claude_session_id.clone(),
        requested_cwd: checkout,
        port_start,
        port_count,
        created_at: chrono::Utc::now().to_rfc3339(),
    };
    if let Some(db) = db.as_ref()
        && let Err(err) = db.insert_claude_worktree(&record)
    {
        eprintln!("aiw: could not record the worktree: {err}");
    }
    announce(paths, "created", &record.id, &record.path);
    Ok(record)
}

/// Remove a Claude worktree: refused (as an error naming why) unless it is
/// clean. Clears the record and the port block either way it ends removed.
pub fn remove(path: &Path, paths: &HookPaths) -> Result<RemoveOutcome> {
    let db = open_db(paths);
    let path_c = canonical(path);
    let record = db
        .as_ref()
        .and_then(|db| db.claude_worktree_by_path(&path_c).ok().flatten());

    let forget = |db: Option<&crate::db::AiwDb>, record: Option<&ClaudeWorktree>| {
        if let Some(record) = record {
            super::ports::release(&record.port_key());
            if let Some(db) = db {
                let _ = db.delete_claude_worktree(&record.id);
            }
            announce(paths, "removed", &record.id, &record.path);
        }
    };

    if !path.exists() {
        if let Some(record) = record.as_ref() {
            let _ = git(&record.repo, &["worktree", "prune"]);
        }
        forget(db.as_ref(), record.as_ref());
        return Ok(RemoveOutcome {
            already_gone: true,
            ..Default::default()
        });
    }

    let repo = match record.as_ref() {
        Some(record) => record.repo.clone(),
        None => main_checkout(&path_c)?,
    };
    if record.is_none() && !is_linked_worktree(&path_c) {
        bail!(
            "{} is not a git worktree AIW created; remove it yourself",
            path.display()
        );
    }
    let dirty = dirty_entries(&path_c)?;
    if !dirty.is_empty() {
        let shown: Vec<&str> = dirty.iter().take(5).map(String::as_str).collect();
        bail!(
            "{} has uncommitted or untracked changes ({}{}); commit or discard them first",
            path.display(),
            shown.join(", "),
            if dirty.len() > shown.len() {
                ", …"
            } else {
                ""
            }
        );
    }
    git(&repo, &["worktree", "remove", &path_c.to_string_lossy()])?;
    super::worktree_paths::prune_empty_dirs(&repo, &path_c);

    let mut outcome = RemoveOutcome::default();
    if let Some(record) = record.as_ref() {
        let ahead = git(
            &repo,
            &[
                "rev-list",
                "--count",
                &format!("{}..refs/heads/{}", record.base, record.branch),
            ],
        )
        .ok()
        .and_then(|n| n.parse::<usize>().ok());
        match ahead {
            Some(0) => {
                if git(&repo, &["branch", "-D", &record.branch]).is_ok() {
                    outcome.branch_deleted = Some(record.branch.clone());
                }
            }
            Some(n) => outcome.branch_kept = Some((record.branch.clone(), n)),
            None => {}
        }
    }
    forget(db.as_ref(), record.as_ref());
    Ok(outcome)
}

// ── the CLI entry ───────────────────────────────────────────────

fn stdin_json() -> Value {
    use std::io::Read;
    let mut input = String::new();
    let _ = std::io::stdin().read_to_string(&mut input);
    serde_json::from_str(input.trim()).unwrap_or_else(|_| json!({}))
}

/// `aiw worktree-hook create|remove`: the process exit status.
pub fn run_hook(action: &str) -> i32 {
    let payload = stdin_json();
    let paths = HookPaths::current();
    match action.trim() {
        "create" => {
            let env = crate::hook_payload::HookEnv::from_process_env();
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let result = parse_create(&payload, &cwd, env.aiw_feature_session_id)
                .and_then(|request| create(&request, &paths));
            match result {
                Ok(record) => {
                    eprintln!(
                        "aiw: worktree {} on {} ({})",
                        record.path.display(),
                        record.branch,
                        record
                            .port_start
                            .map(|p| format!("AIW_PORT={p}"))
                            .unwrap_or_else(|| "no port block".into())
                    );
                    // The last stdout line is what Claude reads.
                    println!("{}", record.path.display());
                    0
                }
                Err(err) => {
                    eprintln!("aiw: could not create the worktree: {err:#}");
                    1
                }
            }
        }
        "remove" => match parse_remove(&payload)
            .and_then(|path| remove(&path, &paths).map(|outcome| (path, outcome)))
        {
            Ok((path, outcome)) => {
                eprintln!("aiw: {}", outcome.summary(&path));
                0
            }
            Err(err) => {
                eprintln!("aiw: kept the worktree: {err:#}");
                1
            }
        },
        other => {
            eprintln!("aiw: worktree-hook expects create or remove, not '{other}'");
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_safe_directory_and_branch_components() {
        assert_eq!(slug("feature-auth"), "feature-auth");
        assert_eq!(slug("Bold Oak a3f2"), "bold-oak-a3f2");
        assert_eq!(slug("../../etc/passwd"), "etc-passwd");
        assert_eq!(slug("a..b"), "a.b");
        assert_eq!(slug("feat/x"), "feat-x");
        assert_eq!(slug("--x--"), "x");
        assert_eq!(slug(""), "worktree");
        assert_eq!(slug("..."), "worktree");
        assert_eq!(slug("ünïcode"), "n-code");
        assert_eq!(slug(&"a".repeat(80)).len(), MAX_SLUG);
        assert!(!slug("x-.").ends_with(['-', '.']));
    }

    #[test]
    fn names_follow_claude_and_avoid_collisions() {
        assert_eq!(
            candidate("auth", 1),
            ("claude-auth".to_string(), "worktree-auth".to_string())
        );
        assert_eq!(
            candidate("auth", 3),
            ("claude-auth-3".to_string(), "worktree-auth-3".to_string())
        );
        // The branch is taken: next suffix.
        let picked = pick_names("auth", |b| b == "worktree-auth", |_| false).unwrap();
        assert_eq!(picked.1, "worktree-auth-2");
        // The directory is taken: next suffix too.
        let picked = pick_names("auth", |_| false, |d| d == "claude-auth").unwrap();
        assert_eq!(picked.0, "claude-auth-2");
        // Everything taken: none.
        assert!(pick_names("auth", |_| true, |_| false).is_none());
    }

    #[test]
    fn the_create_input_names_the_worktree_and_the_requester() {
        let payload = json!({
            "session_id": "conv-1",
            "cwd": "/repo/sub",
            "hook_event_name": "WorktreeCreate",
            "name": "feature-auth",
            "agent_type": "Explore"
        });
        let request = parse_create(&payload, Path::new("/elsewhere"), Some("fs-1".into())).unwrap();
        assert_eq!(request.name, "feature-auth");
        assert_eq!(request.cwd, PathBuf::from("/repo/sub"));
        assert_eq!(request.claude_session_id.as_deref(), Some("conv-1"));
        assert_eq!(request.aiw_session_id.as_deref(), Some("fs-1"));
        assert_eq!(origin_label(&request), "subagent Explore");

        // No cwd (or a relative one): the hook's own directory.
        let request = parse_create(
            &json!({"name": "x", "cwd": "rel"}),
            Path::new("/here"),
            None,
        )
        .unwrap();
        assert_eq!(request.cwd, PathBuf::from("/here"));
        assert_eq!(origin_label(&request), "worktree");
        let agent = parse_create(&json!({"name": "agent-a1b2"}), Path::new("/h"), None).unwrap();
        assert_eq!(origin_label(&agent), "subagent");
        assert!(parse_create(&json!({"cwd": "/r"}), Path::new("/h"), None).is_err());
        assert!(parse_create(&json!({"name": "  "}), Path::new("/h"), None).is_err());
    }

    #[test]
    fn the_remove_input_is_an_absolute_worktree_path() {
        assert_eq!(
            parse_remove(&json!({"worktree_path": "/w/claude-x"})).unwrap(),
            PathBuf::from("/w/claude-x")
        );
        assert!(parse_remove(&json!({"worktree_path": "w"})).is_err());
        assert!(parse_remove(&json!({})).is_err());
    }

    #[test]
    fn the_settings_entries_run_the_script_with_a_timeout() {
        let entries = hook_entries(Path::new("/c/ai_workstation/hooks"));
        let events: Vec<&str> = entries.iter().map(|(e, _)| *e).collect();
        assert_eq!(events, ["WorktreeCreate", "WorktreeRemove"]);
        let create = &entries[0].1["hooks"][0];
        assert_eq!(
            create["command"],
            "/c/ai_workstation/hooks/worktree-hook.sh"
        );
        assert_eq!(create["args"][0], "create");
        assert_eq!(create["timeout"], CREATE_TIMEOUT_SECS);
        assert_eq!(entries[1].1["hooks"][0]["args"][0], "remove");
        // No matcher: Claude's worktree events take none.
        assert!(entries[0].1.get("matcher").is_none());
    }

    #[test]
    fn the_script_passes_stdout_and_the_status_through() {
        assert!(SCRIPT.contains("exec \"$AIW_BIN\" worktree-hook"));
        assert!(!SCRIPT.contains(">/dev/null"));
        assert!(!SCRIPT.contains("AIW_ACTIVE"));
    }

    #[test]
    fn ports_env_exports_the_block() {
        let text = ports_env_text(4110, 10);
        assert!(text.contains("export AIW_PORT=4110\n"));
        assert!(text.contains("export AIW_PORT_COUNT=10\n"));
    }

    // ── create / remove against real git, in temp repos ──────────

    fn sh_git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args([
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "-c",
                "init.defaultBranch=main",
            ])
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    struct Fixture {
        _dir: tempfile::TempDir,
        repo: PathBuf,
        /// A feature worktree of the repo with one commit of its own.
        feature: PathBuf,
        paths: HookPaths,
    }

    fn fixture(with_store: bool) -> Fixture {
        let dir = tempfile::TempDir::new().unwrap();
        let root = canonical(dir.path());
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        sh_git(&repo, &["init", "-q"]);
        std::fs::write(repo.join(".gitignore"), ".env\n").unwrap();
        std::fs::write(repo.join("README.md"), "hi\n").unwrap();
        std::fs::write(repo.join(".env"), "SECRET=1\n").unwrap();
        sh_git(&repo, &["add", "."]);
        sh_git(&repo, &["commit", "-qm", "init"]);
        let feature = root.join("feature");
        sh_git(
            &repo,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feat",
                &feature.to_string_lossy(),
            ],
        );
        std::fs::write(feature.join("feature.txt"), "work\n").unwrap();
        sh_git(&feature, &["add", "."]);
        sh_git(&feature, &["commit", "-qm", "feature work"]);
        let db = root.join("aiw.db");
        if with_store {
            crate::db::AiwDb::open(&db).unwrap();
        }
        Fixture {
            repo,
            feature,
            paths: HookPaths {
                db,
                // Nothing listens here: the dashboard is not running.
                socket: root.join("no-such.sock"),
            },
            _dir: dir,
        }
    }

    fn request(fx: &Fixture, name: &str) -> CreateRequest {
        CreateRequest {
            name: name.into(),
            cwd: fx
                .feature
                .join("sub-does-not-matter")
                .parent()
                .unwrap()
                .to_path_buf(),
            claude_session_id: Some("conv-1".into()),
            agent_type: Some("Explore".into()),
            aiw_session_id: Some("fs-1".into()),
        }
    }

    fn unique(name: &str) -> String {
        format!("{name}-{}", &uuid::Uuid::new_v4().simple().to_string()[..6])
    }

    #[test]
    fn a_create_lands_in_aiws_folder_from_the_feature_head_with_files_and_ports() {
        let fx = fixture(true);
        let name = unique("auth");
        let wt = create(&request(&fx, &name), &fx.paths).unwrap();

        // In AIW's worktree folder, outside the repo, normalized.
        let base = canonical(&super::super::worktree_paths::base_dir());
        assert!(
            wt.path.starts_with(&base),
            "{} not under {}",
            wt.path.display(),
            base.display()
        );
        assert!(!wt.path.starts_with(&fx.repo));
        assert_eq!(
            wt.path.file_name().unwrap().to_string_lossy(),
            format!("claude-{name}")
        );
        assert!(wt.path.is_absolute());
        assert!(!wt.path.components().any(|c| matches!(
            c,
            std::path::Component::ParentDir | std::path::Component::CurDir
        )));
        // A real linked worktree on Claude's branch name, from the feature's HEAD.
        assert!(is_linked_worktree(&wt.path));
        assert_eq!(wt.branch, format!("worktree-{name}"));
        assert_eq!(
            sh_git(&wt.path, &["rev-parse", "--abbrev-ref", "HEAD"]),
            wt.branch
        );
        assert_eq!(wt.base, sh_git(&fx.feature, &["rev-parse", "HEAD"]));
        assert!(
            wt.path.join("feature.txt").exists(),
            "branched from the feature, not main"
        );
        // The included, git-ignored .env came along (Claude skips it with a hook).
        assert_eq!(
            std::fs::read_to_string(wt.path.join(".env")).unwrap(),
            "SECRET=1\n"
        );
        // Its own port block, in a git-ignored file.
        let ports = std::fs::read_to_string(wt.path.join(PORTS_FILE)).unwrap();
        assert!(ports.contains(&format!("export AIW_PORT={}", wt.port_start.unwrap())));
        assert!(
            dirty_entries(&wt.path).unwrap().is_empty(),
            "AIW's files leave it clean"
        );
        // Recorded for the dashboard even though it is not running.
        let db = crate::db::AiwDb::open_for_hook(&fx.paths.db).unwrap();
        let stored = db.claude_worktree_by_path(&wt.path).unwrap().unwrap();
        assert_eq!(stored, wt);
        assert_eq!(stored.parent_session_id.as_deref(), Some("fs-1"));
        assert_eq!(stored.origin, "subagent Explore");
        assert_eq!(stored.requested_cwd, canonical(&fx.feature));

        // Asking again for the same name hands back the same worktree.
        let again = create(&request(&fx, &name), &fx.paths).unwrap();
        assert_eq!(again.path, wt.path);
        assert_eq!(db.all_claude_worktrees().unwrap().len(), 1);
        remove(&wt.path, &fx.paths).unwrap();
    }

    #[test]
    fn a_taken_branch_or_folder_moves_to_the_next_suffix() {
        let fx = fixture(true);
        let name = unique("dup");
        sh_git(&fx.repo, &["branch", &format!("worktree-{name}")]);
        let wt = create(&request(&fx, &name), &fx.paths).unwrap();
        assert_eq!(wt.branch, format!("worktree-{name}-2"));
        assert!(wt.path.ends_with(format!("claude-{name}-2")));
        remove(&wt.path, &fx.paths).unwrap();
    }

    #[test]
    fn without_a_store_the_worktree_is_still_made() {
        let fx = fixture(false);
        let wt = create(&request(&fx, &unique("nostore")), &fx.paths).unwrap();
        assert!(wt.path.is_dir());
        assert!(!fx.paths.db.exists(), "the hook never creates AIW's store");
        // Removing it still works from git's view alone.
        remove(&wt.path, &fx.paths).unwrap();
        assert!(!wt.path.exists());
    }

    #[test]
    fn outside_a_repository_the_create_fails() {
        let dir = tempfile::TempDir::new().unwrap();
        let req = CreateRequest {
            name: "x".into(),
            cwd: dir.path().to_path_buf(),
            claude_session_id: None,
            agent_type: None,
            aiw_session_id: None,
        };
        let paths = HookPaths {
            db: dir.path().join("aiw.db"),
            socket: dir.path().join("s.sock"),
        };
        let err = create(&req, &paths).unwrap_err().to_string();
        assert!(err.contains("not inside a git repository"), "{err}");
    }

    #[test]
    fn a_dirty_worktree_is_kept_and_a_clean_one_goes_with_its_branch() {
        let fx = fixture(true);
        let wt = create(&request(&fx, &unique("dirty")), &fx.paths).unwrap();
        std::fs::write(wt.path.join("scratch.txt"), "wip").unwrap();
        let err = remove(&wt.path, &fx.paths).unwrap_err().to_string();
        assert!(err.contains("uncommitted or untracked"), "{err}");
        assert!(err.contains("scratch.txt"), "{err}");
        assert!(wt.path.is_dir(), "a refused removal leaves it in place");
        let db = crate::db::AiwDb::open_for_hook(&fx.paths.db).unwrap();
        assert!(db.claude_worktree_by_path(&wt.path).unwrap().is_some());

        std::fs::remove_file(wt.path.join("scratch.txt")).unwrap();
        let ports_store = crate::project::aiw_config_dir().join("aiw-ports.json");
        assert!(
            std::fs::read_to_string(&ports_store)
                .unwrap()
                .contains(&wt.port_key())
        );
        let outcome = remove(&wt.path, &fx.paths).unwrap();
        assert!(!wt.path.exists());
        assert_eq!(outcome.branch_deleted.as_deref(), Some(wt.branch.as_str()));
        assert!(!branch_exists(&fx.repo, &wt.branch));
        assert!(db.claude_worktree_by_path(&wt.path).unwrap().is_none());
        assert!(
            !std::fs::read_to_string(&ports_store)
                .unwrap()
                .contains(&wt.port_key())
        );
    }

    #[test]
    fn a_branch_with_commits_is_kept_when_its_worktree_goes() {
        let fx = fixture(true);
        let wt = create(&request(&fx, &unique("commits")), &fx.paths).unwrap();
        std::fs::write(wt.path.join("done.txt"), "x").unwrap();
        sh_git(&wt.path, &["add", "done.txt"]);
        sh_git(&wt.path, &["commit", "-qm", "subagent work"]);
        let outcome = remove(&wt.path, &fx.paths).unwrap();
        assert_eq!(outcome.branch_kept, Some((wt.branch.clone(), 1)));
        assert!(
            branch_exists(&fx.repo, &wt.branch),
            "the commit stays reachable"
        );
        sh_git(&fx.repo, &["branch", "-D", &wt.branch]);
    }

    #[test]
    fn a_vanished_worktree_is_only_forgotten() {
        let fx = fixture(true);
        let wt = create(&request(&fx, &unique("gone")), &fx.paths).unwrap();
        std::fs::remove_dir_all(&wt.path).unwrap();
        let outcome = remove(&wt.path, &fx.paths).unwrap();
        assert!(outcome.already_gone);
        let db = crate::db::AiwDb::open_for_hook(&fx.paths.db).unwrap();
        assert!(db.claude_worktree_by_path(&wt.path).unwrap().is_none());
        sh_git(&fx.repo, &["branch", "-D", &wt.branch]);
    }

    #[test]
    fn a_folder_aiw_did_not_make_is_refused() {
        let fx = fixture(true);
        let plain = fx.repo.parent().unwrap().join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        let err = remove(&plain, &fx.paths).unwrap_err().to_string();
        assert!(
            err.contains("not inside a git repository") || err.contains("not a git worktree"),
            "{err}"
        );
        assert!(plain.exists());
        // The main checkout is never removed either.
        assert!(remove(&fx.repo, &fx.paths).is_err());
        assert!(fx.repo.join("README.md").exists());
    }

    #[test]
    fn removal_summaries_say_what_happened_to_the_branch() {
        let p = Path::new("/w/claude-x");
        let mut outcome = RemoveOutcome {
            branch_deleted: Some("worktree-x".into()),
            ..Default::default()
        };
        assert_eq!(
            outcome.summary(p),
            "removed /w/claude-x; deleted branch worktree-x"
        );
        outcome.branch_deleted = None;
        outcome.branch_kept = Some(("worktree-x".into(), 2));
        assert_eq!(
            outcome.summary(p),
            "removed /w/claude-x; kept branch worktree-x (2 commits beyond its base)"
        );
        let gone = RemoveOutcome {
            already_gone: true,
            ..Default::default()
        };
        assert_eq!(gone.summary(p), "forgot /w/claude-x (already gone)");
    }
}
