//! FORK: landing a feature (DESIGN.md §19.10).
//!
//! `P` on a feature runs the end of its life in one guarded step:
//!
//! 1. **Preflight** ([`compute_preflight`], off the UI thread): the base
//!    branch, commits ahead and behind, uncommitted files, a rebase or merge
//!    already in progress, where the base branch is checked out, and the
//!    strategy that will run. Blockers stop the land before anything changes.
//! 2. **Update** onto the base: `git rebase <base>` (default) or
//!    `git merge --no-edit <base>`. The base is fetched first only when the
//!    repository has the remote; a local base needs no network.
//! 3. **Check**: the optional `land.check_command`, run in the feature's
//!    worktree with its `AIW_PORT` block. Non-zero stops the land.
//! 4. **Finish**: `local_merge` fast-forwards the base branch to the feature
//!    (guarded by the base's expected old value, so a base that moved during
//!    the land is caught); `pr` pushes the branch and runs `gh pr create`
//!    (an existing PR is reported, never duplicated, never auto-merged);
//!    `none` stops after the check.
//!
//! Everything git leaves behind on a failure is left as it is: a conflicted
//! rebase stays in progress for the user to continue or abort. Nothing is
//! ever force-pushed or auto-resolved. One land runs per repository at a time
//! ([`RepoLock`], a `flock` under AIW's state directory); a second one is
//! refused with a message rather than queued.
//!
//! The worker writes every step and every command's output to a log file
//! (`<state>/land/<feature-id>.log`), which the feature's `land` terminal
//! tails, so the user watches the land as it runs.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The automation action (`aiw automation land`).
pub const LAND_ACTION: &str = "land";
/// The label and tmux window of the terminal that shows the land's log.
pub const LAND_TERMINAL: &str = "land";
/// The default remote.
pub const DEFAULT_REMOTE: &str = "origin";
/// How long a network step (`git fetch`, `git push`, `gh`) may take.
pub const NETWORK_TIMEOUT: Duration = Duration::from_secs(180);
/// The last line of every land log; the dashboard knows the run ended.
pub const LOG_END_MARKER: &str = "=== aiw land finished";

/// How the feature is brought up to date with its base.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UpdateStrategy {
    #[default]
    Rebase,
    Merge,
}

impl UpdateStrategy {
    pub fn as_str(self) -> &'static str {
        match self {
            UpdateStrategy::Rebase => "rebase",
            UpdateStrategy::Merge => "merge",
        }
    }
}

/// What happens once the feature is up to date and checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Finish {
    /// Fast-forward the local base branch to the feature.
    LocalMerge,
    /// Push the branch and open a pull request with `gh`.
    Pr,
    /// Stop after the update and the check.
    None,
}

impl Finish {
    pub fn as_str(self) -> &'static str {
        match self {
            Finish::LocalMerge => "local_merge",
            Finish::Pr => "pr",
            Finish::None => "none",
        }
    }
}

/// The `land` block of `aiw.json` (and of the global extension config in
/// `config.json`). Every field is optional; the project's value wins over the
/// global one, field by field ([`LandConfig::merged_over`]).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LandConfig {
    /// The base branch. Default: the remote's default branch, else the
    /// first of main / master / trunk / develop that exists, else the main
    /// checkout's branch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    /// `rebase` (default) or `merge`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub update: Option<UpdateStrategy>,
    /// Run in the feature's worktree after the update; non-zero stops.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub check_command: Option<String>,
    /// `local_merge`, `pr` or `none`. Default: `pr` when the repository has
    /// a GitHub remote and `gh` is signed in, else `local_merge`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish: Option<Finish>,
    /// Delete the feature's branch after a successful land. Default: true
    /// for `local_merge`, false otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delete_branch: Option<bool>,
    /// Stop the feature's sessions, remove its worktree and the feature after
    /// a successful land. Default: true for `local_merge`, false otherwise.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remove_worktree: Option<bool>,
    /// Mark a feature that stays (not removed) as ready. Default: true.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mark_done: Option<bool>,
    /// The remote to fetch from and push to. Default `origin`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remote: Option<String>,
    /// PR title template; `{feature}`, `{branch}`, `{base}`, `{summary}`.
    /// Unset: `gh pr create --fill`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr_title: Option<String>,
    /// PR body template (same placeholders); used with `pr_title`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr_body: Option<String>,
    /// Open the PR as a draft.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pr_draft: Option<bool>,
}

impl LandConfig {
    /// `self` (the project's block) over `base` (the global block).
    pub fn merged_over(&self, base: &LandConfig) -> LandConfig {
        fn pick<T: Clone>(a: &Option<T>, b: &Option<T>) -> Option<T> {
            a.clone().or_else(|| b.clone())
        }
        LandConfig {
            base: pick(&self.base, &base.base),
            update: pick(&self.update, &base.update),
            check_command: pick(&self.check_command, &base.check_command),
            finish: pick(&self.finish, &base.finish),
            delete_branch: pick(&self.delete_branch, &base.delete_branch),
            remove_worktree: pick(&self.remove_worktree, &base.remove_worktree),
            mark_done: pick(&self.mark_done, &base.mark_done),
            remote: pick(&self.remote, &base.remote),
            pr_title: pick(&self.pr_title, &base.pr_title),
            pr_body: pick(&self.pr_body, &base.pr_body),
            pr_draft: pick(&self.pr_draft, &base.pr_draft),
        }
    }

    pub fn is_empty(&self) -> bool {
        self == &LandConfig::default()
    }
}

/// Facts about the repository that the defaults depend on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteFacts {
    /// The configured remote exists.
    pub has_remote: bool,
    /// Its URL points at GitHub.
    pub github: bool,
    /// `gh auth status` succeeded (only asked when it matters).
    pub gh_authed: bool,
}

/// The strategy that will run: every default applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Strategy {
    pub base: String,
    pub update: UpdateStrategy,
    pub check_command: Option<String>,
    pub finish: Finish,
    pub delete_branch: bool,
    pub remove_worktree: bool,
    pub mark_done: bool,
    pub remote: String,
    pub pr_title: Option<String>,
    pub pr_body: Option<String>,
    pub pr_draft: bool,
    /// Why `finish` is what it is, for the dialog.
    pub finish_reason: String,
}

/// Apply the defaults to `config` for a repository with `facts`.
pub fn resolve_strategy(config: &LandConfig, base: &str, facts: &RemoteFacts) -> Strategy {
    let (finish, finish_reason) = match config.finish {
        Some(finish) => (finish, "configured".to_string()),
        None if facts.has_remote && facts.github && facts.gh_authed => (
            Finish::Pr,
            "default: GitHub remote and gh signed in".to_string(),
        ),
        None if facts.has_remote && facts.github => (
            Finish::LocalMerge,
            "default: gh is not signed in".to_string(),
        ),
        None if facts.has_remote => (
            Finish::LocalMerge,
            "default: the remote is not on GitHub".to_string(),
        ),
        None => (Finish::LocalMerge, "default: no remote".to_string()),
    };
    let cleanup_default = finish == Finish::LocalMerge;
    Strategy {
        base: base.to_string(),
        update: config.update.unwrap_or_default(),
        check_command: config
            .check_command
            .clone()
            .filter(|c| !c.trim().is_empty()),
        finish,
        delete_branch: config.delete_branch.unwrap_or(cleanup_default),
        remove_worktree: config.remove_worktree.unwrap_or(cleanup_default),
        mark_done: config.mark_done.unwrap_or(true),
        remote: config
            .remote
            .clone()
            .filter(|r| !r.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_REMOTE.to_string()),
        pr_title: config.pr_title.clone().filter(|t| !t.trim().is_empty()),
        pr_body: config.pr_body.clone(),
        pr_draft: config.pr_draft.unwrap_or(false),
        finish_reason,
    }
}

/// Where the base branch is checked out, if anywhere.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BaseCheckout {
    pub path: PathBuf,
    /// Tracked files with uncommitted changes there.
    pub dirty: usize,
}

/// What the dialog shows before anything changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Preflight {
    pub feature: String,
    pub project: String,
    pub repo: PathBuf,
    pub workdir: PathBuf,
    pub branch: String,
    pub base: String,
    /// Where the base came from (`aiw.json land.base`, `origin/HEAD`, …).
    pub base_source: String,
    /// The ref the update runs against: `<remote>/<base>` for a PR, the
    /// local base otherwise.
    pub upstream: String,
    pub ahead: usize,
    pub behind: usize,
    /// Tracked files with changes, untracked files.
    pub modified: usize,
    pub untracked: usize,
    pub strategy: Strategy,
    pub facts_has_remote: bool,
    pub base_checkout: Option<BaseCheckout>,
    /// Stops the land.
    pub blockers: Vec<String>,
    /// Shown, does not stop.
    pub warnings: Vec<String>,
    /// The commands the land will run, in order.
    pub plan: Vec<String>,
    /// A rebase or merge is in progress in the worktree.
    pub in_progress: Option<InProgress>,
}

impl Preflight {
    pub fn landable(&self) -> bool {
        self.blockers.is_empty()
    }
}

/// An operation git left half-done in the worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InProgress {
    Rebase,
    Merge,
}

impl InProgress {
    pub fn as_str(self) -> &'static str {
        match self {
            InProgress::Rebase => "rebase",
            InProgress::Merge => "merge",
        }
    }
    pub fn abort_args(self) -> [&'static str; 2] {
        match self {
            InProgress::Rebase => ["rebase", "--abort"],
            InProgress::Merge => ["merge", "--abort"],
        }
    }
}

/// Everything [`compute_preflight`] needs (gathered on the UI thread).
#[derive(Debug, Clone)]
pub struct PreflightInput {
    pub feature: String,
    pub project: String,
    pub repo: PathBuf,
    pub workdir: PathBuf,
    pub is_worktree: bool,
    pub branch: String,
    pub config: LandConfig,
    /// Ask `gh auth status` when the default finish depends on it.
    pub probe_gh: bool,
    /// Blockers and warnings the app adds (agents working, another land …).
    pub extra_blockers: Vec<String>,
    pub extra_warnings: Vec<String>,
}

// ── git ──────────────────────────────────────────────────────────────

fn git_command(dir: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .stdin(Stdio::null());
    if std::env::var_os("GIT_SSH_COMMAND").is_none() {
        cmd.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    }
    cmd
}

/// `git <args>` in `dir`: stdout trimmed, or the error text.
pub fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let out = git_command(dir)
        .args(args)
        .output()
        .map_err(|e| format!("could not run git: {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() {
            format!("git {} failed", args.join(" "))
        } else {
            err
        })
    }
}

fn git_ok(dir: &Path, args: &[&str]) -> bool {
    git_command(dir)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

pub fn ref_exists(dir: &Path, name: &str) -> bool {
    git_ok(
        dir,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{name}^{{commit}}"),
        ],
    )
}

fn local_branch_exists(dir: &Path, name: &str) -> bool {
    ref_exists(dir, &format!("refs/heads/{name}"))
}

/// The base branch and where it came from.
pub fn resolve_base(
    repo: &Path,
    branch: &str,
    configured: Option<&str>,
    remote: &str,
) -> Option<(String, String)> {
    if let Some(base) = configured.map(str::trim).filter(|b| !b.is_empty()) {
        return Some((base.to_string(), "aiw.json land.base".into()));
    }
    if let Ok(head) = git(
        repo,
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            &format!("refs/remotes/{remote}/HEAD"),
        ],
    ) && let Some(name) = head.strip_prefix(&format!("{remote}/"))
        && name != branch
    {
        return Some((name.to_string(), format!("{remote}/HEAD")));
    }
    for name in ["main", "master", "trunk", "develop"] {
        if name != branch && local_branch_exists(repo, name) {
            return Some((name.to_string(), format!("the repository's `{name}`")));
        }
    }
    if let Ok(head) = git(repo, &["symbolic-ref", "--quiet", "--short", "HEAD"])
        && head != branch
        && !head.is_empty()
    {
        return Some((head, "the main checkout's branch".into()));
    }
    None
}

/// Facts about `remote` in `repo`; `probe_gh` asks `gh auth status`.
pub fn remote_facts(repo: &Path, remote: &str, probe_gh: bool) -> RemoteFacts {
    let url = git(repo, &["remote", "get-url", remote]).ok();
    let has_remote = url.is_some();
    let github = url.as_deref().is_some_and(is_github_url);
    let gh_authed = probe_gh && has_remote && github && gh_signed_in(repo);
    RemoteFacts {
        has_remote,
        github,
        gh_authed,
    }
}

pub fn is_github_url(url: &str) -> bool {
    let url = url.to_ascii_lowercase();
    url.contains("github.com")
}

fn gh_signed_in(dir: &Path) -> bool {
    let mut cmd = Command::new("gh");
    cmd.args(["auth", "status"])
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    run_with_timeout(cmd, Duration::from_secs(10)).is_ok_and(|code| code == Some(0))
}

/// A rebase or merge git left in progress in `workdir`.
pub fn in_progress(workdir: &Path) -> Option<InProgress> {
    let path = |name: &str| {
        git(workdir, &["rev-parse", "--git-path", name])
            .ok()
            .map(|p| {
                let p = PathBuf::from(p);
                if p.is_absolute() { p } else { workdir.join(p) }
            })
    };
    if path("rebase-merge").is_some_and(|p| p.exists())
        || path("rebase-apply").is_some_and(|p| p.exists())
    {
        return Some(InProgress::Rebase);
    }
    if path("MERGE_HEAD").is_some_and(|p| p.exists()) {
        return Some(InProgress::Merge);
    }
    None
}

/// `(modified tracked files, untracked files)` in `dir`.
pub fn dirty_counts(dir: &Path) -> (usize, usize) {
    let Ok(out) = git(dir, &["status", "--porcelain=v1", "--untracked-files=all"]) else {
        return (0, 0);
    };
    let mut modified = 0;
    let mut untracked = 0;
    for line in out.lines().filter(|l| !l.trim().is_empty()) {
        if line.starts_with("??") {
            untracked += 1;
        } else {
            modified += 1;
        }
    }
    (modified, untracked)
}

/// `(ahead, behind)` of `head` against `upstream`.
pub fn ahead_behind(dir: &Path, upstream: &str, head: &str) -> Option<(usize, usize)> {
    let out = git(
        dir,
        &[
            "rev-list",
            "--left-right",
            "--count",
            &format!("{upstream}...{head}"),
        ],
    )
    .ok()?;
    let mut parts = out.split_whitespace();
    let behind = parts.next()?.parse().ok()?;
    let ahead = parts.next()?.parse().ok()?;
    Some((ahead, behind))
}

/// The worktree where `branch` is checked out, from
/// `git worktree list --porcelain`.
pub fn checkout_of(porcelain: &str, branch: &str) -> Option<PathBuf> {
    let wanted = format!("branch refs/heads/{branch}");
    let mut current: Option<PathBuf> = None;
    for line in porcelain.lines() {
        if let Some(path) = line.strip_prefix("worktree ") {
            current = Some(PathBuf::from(path));
        } else if line.trim() == wanted {
            return current;
        } else if line.is_empty() {
            current = None;
        }
    }
    None
}

fn base_checkout(repo: &Path, base: &str) -> Option<BaseCheckout> {
    let porcelain = git(repo, &["worktree", "list", "--porcelain"]).ok()?;
    let path = checkout_of(&porcelain, base)?;
    let dirty = git(&path, &["status", "--porcelain=v1", "--untracked-files=no"])
        .map(|out| out.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0);
    Some(BaseCheckout { path, dirty })
}

/// The ref the update runs against.
pub fn upstream_ref(repo: &Path, strategy: &Strategy, has_remote: bool) -> String {
    let remote_ref = format!("{}/{}", strategy.remote, strategy.base);
    if strategy.finish == Finish::Pr && has_remote && ref_exists(repo, &remote_ref) {
        remote_ref
    } else {
        strategy.base.clone()
    }
}

/// Run the preflight: git only, no changes.
pub fn compute_preflight(input: &PreflightInput) -> Preflight {
    let mut blockers = input.extra_blockers.clone();
    let mut warnings = input.extra_warnings.clone();
    let remote = input
        .config
        .remote
        .clone()
        .filter(|r| !r.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_REMOTE.to_string());
    let probe_gh = input.probe_gh && input.config.finish.is_none();
    let facts = remote_facts(&input.repo, &remote, probe_gh);

    // The branch the worktree is really on.
    let mut branch = input.branch.clone();
    if input.is_worktree {
        match git(
            &input.workdir,
            &["symbolic-ref", "--quiet", "--short", "HEAD"],
        ) {
            Ok(actual) if !actual.is_empty() => {
                if actual != branch {
                    warnings.push(format!(
                        "the worktree is on `{actual}`, not `{branch}`; landing `{actual}`"
                    ));
                    branch = actual;
                }
            }
            _ => blockers.push("the worktree is not on a branch (detached HEAD)".into()),
        }
    } else {
        blockers.push(
            "land works on a feature in its own worktree; this one uses the repository directory"
                .into(),
        );
    }

    let (base, base_source) =
        match resolve_base(&input.repo, &branch, input.config.base.as_deref(), &remote) {
            Some(found) => found,
            None => {
                blockers.push("could not find a base branch; set land.base in aiw.json".into());
                (String::new(), String::new())
            }
        };
    if !base.is_empty() && base == branch {
        blockers.push(format!("`{branch}` is the base branch itself"));
    }
    if !base.is_empty() && !ref_exists(&input.repo, &base) {
        blockers.push(format!("the base branch `{base}` does not exist locally"));
    }

    let strategy = resolve_strategy(&input.config, &base, &facts);
    if strategy.finish == Finish::Pr && !facts.has_remote {
        blockers.push(format!(
            "finish is `pr` but the repository has no `{}` remote",
            strategy.remote
        ));
    }
    let upstream = if base.is_empty() {
        String::new()
    } else {
        upstream_ref(&input.repo, &strategy, facts.has_remote)
    };

    let in_progress = if input.is_worktree {
        in_progress(&input.workdir)
    } else {
        None
    };
    if let Some(op) = in_progress {
        blockers.push(format!(
            "a {} is in progress in the worktree — continue it or abort it (a) first",
            op.as_str()
        ));
    }

    let (modified, untracked) = if input.is_worktree {
        dirty_counts(&input.workdir)
    } else {
        (0, 0)
    };
    if in_progress.is_none() && (modified > 0 || untracked > 0) {
        blockers.push(format!(
            "the worktree has uncommitted work ({modified} changed, {untracked} untracked) — commit or stash it (t opens a terminal)"
        ));
    }

    let (ahead, behind) = if upstream.is_empty() || !input.is_worktree {
        (0, 0)
    } else {
        ahead_behind(&input.workdir, &upstream, "HEAD").unwrap_or((0, 0))
    };
    if input.is_worktree && !upstream.is_empty() && ahead == 0 {
        blockers.push(format!(
            "nothing to land: `{branch}` has no commits beyond `{upstream}`"
        ));
    }
    if behind > 0 {
        warnings.push(format!(
            "`{upstream}` is {behind} commit(s) ahead; the land {}s onto it first",
            strategy.update.as_str()
        ));
    }

    let base_checkout = if base.is_empty() {
        None
    } else {
        base_checkout(&input.repo, &base)
    };
    if strategy.finish == Finish::LocalMerge
        && let Some(checkout) = &base_checkout
        && checkout.dirty > 0
    {
        blockers.push(format!(
            "`{base}` is checked out in {} with uncommitted changes; aiw will not touch it",
            checkout.path.display()
        ));
    }
    if strategy.finish == Finish::LocalMerge && facts.has_remote {
        let remote_ref = format!("{}/{}", strategy.remote, base);
        if ref_exists(&input.repo, &remote_ref)
            && let Some((remote_ahead, _)) = ahead_behind(&input.repo, &base, &remote_ref)
            && remote_ahead > 0
        {
            warnings.push(format!(
                "`{remote_ref}` has {remote_ahead} commit(s) the local `{base}` lacks; landing locally does not pull them"
            ));
        }
    }
    if strategy.remove_worktree {
        warnings
            .push("after landing, the feature's sessions stop and its worktree is removed".into());
    }

    let plan = plan_commands(
        &strategy,
        &branch,
        &upstream,
        facts.has_remote,
        base_checkout.as_ref(),
    );
    Preflight {
        feature: input.feature.clone(),
        project: input.project.clone(),
        repo: input.repo.clone(),
        workdir: input.workdir.clone(),
        branch,
        base,
        base_source,
        upstream,
        ahead,
        behind,
        modified,
        untracked,
        strategy,
        facts_has_remote: facts.has_remote,
        base_checkout,
        blockers,
        warnings,
        plan,
        in_progress,
    }
}

/// The commands a land will run, for the dialog and `--dry-run`.
pub fn plan_commands(
    strategy: &Strategy,
    branch: &str,
    upstream: &str,
    has_remote: bool,
    base_checkout: Option<&BaseCheckout>,
) -> Vec<String> {
    let mut plan = Vec::new();
    if has_remote {
        plan.push(format!("git fetch {} {}", strategy.remote, strategy.base));
    }
    match strategy.update {
        UpdateStrategy::Rebase => plan.push(format!("git rebase {upstream}")),
        UpdateStrategy::Merge => plan.push(format!("git merge --no-edit {upstream}")),
    }
    if let Some(check) = &strategy.check_command {
        plan.push(format!("sh -c {}", shell_quote(check)));
    }
    match strategy.finish {
        Finish::LocalMerge => match base_checkout {
            Some(checkout) => plan.push(format!(
                "git -C {} merge --ff-only {branch}",
                shell_quote(&checkout.path.to_string_lossy())
            )),
            None => plan.push(format!(
                "git update-ref refs/heads/{} <{branch}> <expected {}>",
                strategy.base, strategy.base
            )),
        },
        Finish::Pr => {
            plan.push(format!("git push -u {} {branch}", strategy.remote));
            let mut gh = format!("gh pr create --base {} --head {branch}", strategy.base);
            if strategy.pr_title.is_some() {
                gh.push_str(" --title … --body …");
            } else {
                gh.push_str(" --fill");
            }
            if strategy.pr_draft {
                gh.push_str(" --draft");
            }
            plan.push(gh);
        }
        Finish::None => {}
    }
    if strategy.finish != Finish::None {
        if strategy.remove_worktree {
            plan.push("stop the sessions; git worktree remove <worktree>".into());
        }
        if strategy.delete_branch {
            plan.push(format!("git branch -d {branch}"));
        }
    }
    plan
}

pub use crate::fork::shell::quote_min as shell_quote;

/// `{feature}`, `{branch}`, `{base}` and `{summary}` in a PR template.
pub fn render_template(
    template: &str,
    feature: &str,
    branch: &str,
    base: &str,
    summary: &str,
) -> String {
    template
        .replace("{feature}", feature)
        .replace("{branch}", branch)
        .replace("{base}", base)
        .replace("{summary}", summary)
}

/// The first non-empty line of a prompt, cut to 72 characters.
pub fn summary_line(prompt: &str) -> String {
    let line = prompt
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    if line.chars().count() > 72 {
        let cut: String = line.chars().take(71).collect();
        format!("{cut}…")
    } else {
        line.to_string()
    }
}

// ── gh ───────────────────────────────────────────────────────────────

/// What `gh pr create` said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GhCreate {
    Created(String),
    Existing(String),
    Failed(String),
}

fn first_url(text: &str) -> Option<String> {
    text.split_whitespace()
        .rev()
        .find(|t| t.starts_with("https://") || t.starts_with("http://"))
        .map(|t| t.trim_end_matches(['.', ',', ')']).to_string())
}

/// Read `gh pr create`'s result.
pub fn parse_gh_create(success: bool, stdout: &str, stderr: &str) -> GhCreate {
    if success {
        return match first_url(stdout).or_else(|| first_url(stderr)) {
            Some(url) => GhCreate::Created(url),
            None => GhCreate::Created(String::new()),
        };
    }
    let all = format!("{stderr}\n{stdout}");
    if all.to_ascii_lowercase().contains("already exists")
        && let Some(url) = first_url(&all)
    {
        return GhCreate::Existing(url);
    }
    let reason = stderr
        .lines()
        .chain(stdout.lines())
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("gh pr create failed")
        .to_string();
    GhCreate::Failed(reason)
}

// ── the repo lock ────────────────────────────────────────────────────

/// One land per repository: an exclusive `flock` on
/// `<state>/land/<sha256(repo)>.lock`, held for the whole land.
pub struct RepoLock {
    _file: File,
    pub path: PathBuf,
}

pub fn lock_path(state_dir: &Path, repo: &Path) -> PathBuf {
    let digest = Sha256::digest(repo.to_string_lossy().as_bytes());
    let hex: String = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    state_dir.join("land").join(format!("{hex}.lock"))
}

impl RepoLock {
    /// Take the lock without waiting; `Err` names why.
    pub fn try_acquire(state_dir: &Path, repo: &Path) -> Result<RepoLock, String> {
        use std::os::unix::io::AsRawFd;
        let path = lock_path(state_dir, repo);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("could not create {}: {e}", parent.display()))?;
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|e| format!("could not open {}: {e}", path.display()))?;
        // SAFETY: the descriptor is valid for the life of `file`.
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc != 0 {
            return Err("another land is running in this repository".into());
        }
        Ok(RepoLock { _file: file, path })
    }
}

// ── the worker ───────────────────────────────────────────────────────

/// Where a land is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Starting,
    PreHook,
    Fetch,
    Update,
    Check,
    Finish,
    PostHook,
}

impl Stage {
    pub fn label(self) -> &'static str {
        match self {
            Stage::Starting => "starting",
            Stage::PreHook => "pre_land hook",
            Stage::Fetch => "fetching",
            Stage::Update => "updating",
            Stage::Check => "checking",
            Stage::Finish => "finishing",
            Stage::PostHook => "post_land hook",
        }
    }
}

/// Why a land stopped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    Locked,
    PreHook,
    Fetch,
    Conflict,
    Check,
    DirtyAfterCheck,
    BaseMoved,
    BaseDirty,
    NotFastForward,
    Push,
    Gh,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Failure {
    pub kind: FailureKind,
    /// A short reason for the row: `check failed (exit 1)`.
    pub reason: String,
    /// The operation left in progress (a conflicted rebase).
    pub in_progress: Option<InProgress>,
}

impl Failure {
    fn new(kind: FailureKind, reason: impl Into<String>) -> Self {
        Failure {
            kind,
            reason: reason.into(),
            in_progress: None,
        }
    }
}

/// How a land ended.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Outcome {
    pub feature_id: String,
    pub ok: bool,
    pub failure: Option<Failure>,
    /// The PR that was opened or already existed.
    pub pr_url: Option<String>,
    pub pr_existed: bool,
    /// The commit the base now points at (local merge).
    pub merged: Option<String>,
    pub log: PathBuf,
}

/// A hook to run, already approved (or trusted) by the app.
#[derive(Debug, Clone)]
pub struct HookRun {
    pub name: &'static str,
    pub script: String,
}

/// Everything the worker needs.
#[derive(Debug, Clone)]
pub struct Job {
    pub feature_id: String,
    pub feature: String,
    pub repo: PathBuf,
    pub workdir: PathBuf,
    pub branch: String,
    pub strategy: Strategy,
    pub has_remote: bool,
    pub state_dir: PathBuf,
    pub log: PathBuf,
    /// The feature's port block and other env for the check and hooks.
    pub env: Vec<(String, String)>,
    pub pre_land: Option<HookRun>,
    pub post_land: Option<HookRun>,
    /// First line of the latest prompt, for PR templates.
    pub summary: String,
    /// The `gh` binary (tests point it at a fake).
    pub gh: String,
    /// Test hook: runs right after the check, before the finish.
    pub before_finish: Option<fn(&Job)>,
}

/// What the worker reports.
#[derive(Debug, Clone)]
pub enum Event {
    Stage(Stage),
    Finished(Box<Outcome>),
}

/// The log path for a feature.
pub fn log_path(state_dir: &Path, feature_id: &str) -> PathBuf {
    let safe: String = feature_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    state_dir.join("land").join(format!("{safe}.log"))
}

struct Log {
    file: Option<File>,
}

impl Log {
    fn create(path: &Path) -> Log {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        Log {
            file: File::create(path).ok(),
        }
    }
    fn line(&mut self, text: &str) {
        if let Some(file) = self.file.as_mut() {
            let _ = writeln!(file, "{text}");
            let _ = file.flush();
        }
    }
    fn stdio(&self) -> (Stdio, Stdio) {
        let out = self
            .file
            .as_ref()
            .and_then(|f| f.try_clone().ok())
            .map(Stdio::from)
            .unwrap_or_else(Stdio::null);
        let err = self
            .file
            .as_ref()
            .and_then(|f| f.try_clone().ok())
            .map(Stdio::from)
            .unwrap_or_else(Stdio::null);
        (out, err)
    }
}

/// Wait for `cmd` with a deadline; `Ok(exit code)` (None = killed by a
/// signal), `Err` when it could not start or timed out (then killed).
pub fn run_with_timeout(mut cmd: Command, timeout: Duration) -> Result<Option<i32>, String> {
    let mut child = cmd.spawn().map_err(|e| format!("could not start: {e}"))?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status.code()),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("timed out after {}s", timeout.as_secs()));
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e.to_string()),
        }
    }
}

/// `git <args>` in `dir` with its output in the log.
fn logged_git(
    log: &mut Log,
    dir: &Path,
    args: &[&str],
    timeout: Option<Duration>,
) -> Result<(), String> {
    log.line(&format!("$ git {}", args.join(" ")));
    let (out, err) = log.stdio();
    let mut cmd = git_command(dir);
    cmd.args(args).stdout(out).stderr(err);
    let code = match timeout {
        Some(t) => run_with_timeout(cmd, t)?,
        None => cmd
            .status()
            .map_err(|e| format!("could not run git: {e}"))?
            .code(),
    };
    match code {
        Some(0) => Ok(()),
        Some(code) => Err(format!("exit {code}")),
        None => Err("killed".into()),
    }
}

/// `sh -c <script>` in `dir` with `env`, output in the log.
fn logged_shell(
    log: &mut Log,
    dir: &Path,
    script: &str,
    env: &[(String, String)],
) -> Result<(), String> {
    log.line(&format!("$ {script}"));
    let (out, err) = log.stdio();
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(script)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err);
    for (key, value) in env {
        cmd.env(key, value);
    }
    match cmd
        .status()
        .map_err(|e| format!("could not run sh: {e}"))?
        .code()
    {
        Some(0) => Ok(()),
        Some(code) => Err(format!("exit {code}")),
        None => Err("killed".into()),
    }
}

fn hook_env(job: &Job, result: Option<(&str, &str, &str)>) -> Vec<(String, String)> {
    let mut env = job.env.clone();
    let mut push = |k: &str, v: &str| env.push((k.to_string(), v.to_string()));
    push("AIW_LAND_FEATURE", &job.feature);
    push("AIW_LAND_BRANCH", &job.branch);
    push("AIW_LAND_BASE", &job.strategy.base);
    push("AIW_LAND_STRATEGY", job.strategy.update.as_str());
    push("AIW_LAND_FINISH", job.strategy.finish.as_str());
    push("AIW_LAND_REPO", &job.repo.to_string_lossy());
    push("AIW_LAND_WORKTREE", &job.workdir.to_string_lossy());
    if let Some((status, reason, pr)) = result {
        push("AIW_LAND_RESULT", status);
        push("AIW_LAND_REASON", reason);
        push("AIW_LAND_PR_URL", pr);
    }
    env
}

/// Run a land to the end. Never panics; every failure is an [`Outcome`].
pub fn run(job: Job, events: Sender<Event>) -> Outcome {
    let mut log = Log::create(&job.log);
    log.line(&format!(
        "=== aiw land: {} ({} → {}), {} then {}",
        job.feature,
        job.branch,
        job.strategy.base,
        job.strategy.update.as_str(),
        job.strategy.finish.as_str()
    ));
    let mut outcome = Outcome {
        feature_id: job.feature_id.clone(),
        ok: false,
        failure: None,
        pr_url: None,
        pr_existed: false,
        merged: None,
        log: job.log.clone(),
    };
    let result = run_steps(&job, &mut log, &events, &mut outcome);
    if let Err(failure) = result {
        log.line(&format!("!!! land stopped: {}", failure.reason));
        if let Some(op) = failure.in_progress {
            log.line(&format!(
                "    the {op} is still in progress in {}: resolve and `git {op} --continue`, or `git {op} --abort` (a in the land dialog)",
                job.workdir.display(),
                op = op.as_str()
            ));
        }
        outcome.failure = Some(failure);
    } else {
        outcome.ok = true;
    }
    if let Some(hook) = &job.post_land {
        let _ = events.send(Event::Stage(Stage::PostHook));
        log.line(&format!("--- {} hook", hook.name));
        let status = if outcome.ok { "success" } else { "failed" };
        let reason = outcome
            .failure
            .as_ref()
            .map(|f| f.reason.clone())
            .unwrap_or_default();
        let pr = outcome.pr_url.clone().unwrap_or_default();
        let env = hook_env(&job, Some((status, &reason, &pr)));
        if let Err(e) = logged_shell(&mut log, &job.repo, &hook.script, &env) {
            log.line(&format!(
                "    post_land hook failed ({e}); the land result stands"
            ));
        }
    }
    log.line(&format!(
        "{LOG_END_MARKER}: {}",
        if outcome.ok {
            match (&outcome.pr_url, outcome.merged.as_deref()) {
                (Some(url), _) if outcome.pr_existed => format!("ok (PR already open: {url})"),
                (Some(url), _) => format!("ok (PR {url})"),
                (None, Some(sha)) => format!("ok ({} is now {})", job.strategy.base, short(sha)),
                _ => "ok".to_string(),
            }
        } else {
            format!(
                "failed: {}",
                outcome
                    .failure
                    .as_ref()
                    .map(|f| f.reason.as_str())
                    .unwrap_or("unknown")
            )
        }
    ));
    let _ = events.send(Event::Finished(Box::new(outcome.clone())));
    outcome
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(10)]
}

fn run_steps(
    job: &Job,
    log: &mut Log,
    events: &Sender<Event>,
    outcome: &mut Outcome,
) -> Result<(), Failure> {
    let stage = |s: Stage| {
        let _ = events.send(Event::Stage(s));
    };
    stage(Stage::Starting);
    let _lock = RepoLock::try_acquire(&job.state_dir, &job.repo)
        .map_err(|e| Failure::new(FailureKind::Locked, e))?;

    if let Some(hook) = &job.pre_land {
        stage(Stage::PreHook);
        log.line(&format!("--- {} hook", hook.name));
        logged_shell(log, &job.workdir, &hook.script, &hook_env(job, None)).map_err(|e| {
            Failure::new(FailureKind::PreHook, format!("pre_land hook failed ({e})"))
        })?;
    }

    let s = &job.strategy;
    if job.has_remote {
        stage(Stage::Fetch);
        log.line("--- fetch");
        if let Err(e) = logged_git(
            log,
            &job.repo,
            &["fetch", "--no-tags", &s.remote, &s.base],
            Some(NETWORK_TIMEOUT),
        ) {
            if s.finish == Finish::Pr {
                return Err(Failure::new(
                    FailureKind::Fetch,
                    format!("fetch failed ({e})"),
                ));
            }
            log.line(&format!(
                "    fetch failed ({e}); landing onto the local `{}`",
                s.base
            ));
        }
    }
    let upstream = upstream_ref(&job.repo, s, job.has_remote);
    let expected_base = git(&job.repo, &["rev-parse", &format!("{upstream}^{{commit}}")])
        .map_err(|e| Failure::new(FailureKind::Other, format!("cannot read `{upstream}`: {e}")))?;

    // Update.
    stage(Stage::Update);
    log.line(&format!(
        "--- {} onto {upstream} ({})",
        s.update.as_str(),
        short(&expected_base)
    ));
    if git_ok(
        &job.workdir,
        &["merge-base", "--is-ancestor", &expected_base, "HEAD"],
    ) {
        log.line(&format!(
            "    `{}` already contains `{upstream}`",
            job.branch
        ));
    } else {
        let args: Vec<&str> = match s.update {
            UpdateStrategy::Rebase => vec!["rebase", &expected_base],
            UpdateStrategy::Merge => vec!["merge", "--no-edit", &expected_base],
        };
        if let Err(e) = logged_git(log, &job.workdir, &args, None) {
            let op = in_progress(&job.workdir);
            let mut failure = Failure::new(
                FailureKind::Conflict,
                match op {
                    Some(op) => format!("{} conflict onto {upstream}", op.as_str()),
                    None => format!("{} onto {upstream} failed ({e})", s.update.as_str()),
                },
            );
            failure.in_progress = op;
            return Err(failure);
        }
    }

    // Check.
    if let Some(check) = &s.check_command {
        stage(Stage::Check);
        log.line("--- check");
        let env = hook_env(job, None);
        logged_shell(log, &job.workdir, check, &env)
            .map_err(|e| Failure::new(FailureKind::Check, format!("check failed ({e})")))?;
        let (modified, untracked) = dirty_counts(&job.workdir);
        if modified > 0 {
            return Err(Failure::new(
                FailureKind::DirtyAfterCheck,
                format!("the check left {modified} uncommitted change(s) (untracked: {untracked})"),
            ));
        }
    }
    if let Some(hook) = job.before_finish {
        hook(job);
    }

    // Finish.
    stage(Stage::Finish);
    let head = git(&job.workdir, &["rev-parse", "HEAD"])
        .map_err(|e| Failure::new(FailureKind::Other, format!("cannot read HEAD: {e}")))?;
    match s.finish {
        Finish::None => {
            log.line("--- finish: none (update and check only)");
        }
        Finish::LocalMerge => {
            log.line(&format!(
                "--- fast-forward `{}` to {}",
                s.base,
                short(&head)
            ));
            if !git_ok(
                &job.workdir,
                &["merge-base", "--is-ancestor", &expected_base, &head],
            ) {
                return Err(Failure::new(
                    FailureKind::NotFastForward,
                    format!("`{}` cannot fast-forward to `{}`", s.base, job.branch),
                ));
            }
            match base_checkout(&job.repo, &s.base) {
                Some(checkout) => {
                    if checkout.dirty > 0 {
                        return Err(Failure::new(
                            FailureKind::BaseDirty,
                            format!(
                                "`{}` is checked out in {} with uncommitted changes",
                                s.base,
                                checkout.path.display()
                            ),
                        ));
                    }
                    let now = git(&checkout.path, &["rev-parse", "HEAD"]).unwrap_or_default();
                    if now != expected_base {
                        return Err(Failure::new(
                            FailureKind::BaseMoved,
                            format!("`{}` moved during the land; run land again", s.base),
                        ));
                    }
                    logged_git(log, &checkout.path, &["merge", "--ff-only", &head], None).map_err(
                        |e| {
                            Failure::new(
                                FailureKind::NotFastForward,
                                format!("fast-forward in {} failed ({e})", checkout.path.display()),
                            )
                        },
                    )?;
                }
                None => {
                    let reference = format!("refs/heads/{}", s.base);
                    logged_git(
                        log,
                        &job.repo,
                        &[
                            "update-ref",
                            "-m",
                            &format!("aiw land {}", job.branch),
                            &reference,
                            &head,
                            &expected_base,
                        ],
                        None,
                    )
                    .map_err(|_| {
                        Failure::new(
                            FailureKind::BaseMoved,
                            format!("`{}` moved during the land; run land again", s.base),
                        )
                    })?;
                }
            }
            outcome.merged = Some(head.clone());
        }
        Finish::Pr => {
            log.line(&format!("--- push `{}` to {}", job.branch, s.remote));
            logged_git(
                log,
                &job.workdir,
                &["push", "-u", &s.remote, &job.branch],
                Some(NETWORK_TIMEOUT),
            )
            .map_err(|e| {
                Failure::new(
                    FailureKind::Push,
                    format!(
                        "push failed ({e}); if the branch was pushed before the rebase, push it yourself (aiw never force-pushes)"
                    ),
                )
            })?;
            log.line("--- gh pr create");
            let mut args: Vec<String> = vec![
                "pr".into(),
                "create".into(),
                "--base".into(),
                s.base.clone(),
                "--head".into(),
                job.branch.clone(),
            ];
            match &s.pr_title {
                Some(title) => {
                    args.push("--title".into());
                    args.push(render_template(
                        title,
                        &job.feature,
                        &job.branch,
                        &s.base,
                        &job.summary,
                    ));
                    args.push("--body".into());
                    args.push(render_template(
                        s.pr_body.as_deref().unwrap_or(""),
                        &job.feature,
                        &job.branch,
                        &s.base,
                        &job.summary,
                    ));
                }
                None => args.push("--fill".into()),
            }
            if s.pr_draft {
                args.push("--draft".into());
            }
            log.line(&format!("$ gh {}", args.join(" ")));
            let (success, stdout, stderr) = run_gh(&job.gh, &job.workdir, &args)?;
            log.line(stdout.trim_end());
            if !stderr.trim().is_empty() {
                log.line(stderr.trim_end());
            }
            match parse_gh_create(success, &stdout, &stderr) {
                GhCreate::Created(url) => {
                    outcome.pr_url = Some(url);
                }
                GhCreate::Existing(url) => {
                    log.line(&format!("    a pull request is already open: {url}"));
                    outcome.pr_url = Some(url);
                    outcome.pr_existed = true;
                }
                GhCreate::Failed(reason) => {
                    return Err(Failure::new(
                        FailureKind::Gh,
                        format!("gh pr create failed: {reason}"),
                    ));
                }
            }
        }
    }
    Ok(())
}

fn run_gh(gh: &str, dir: &Path, args: &[String]) -> Result<(bool, String, String), Failure> {
    let mut cmd = Command::new(gh);
    cmd.args(args)
        .current_dir(dir)
        .env("GH_PROMPT_DISABLED", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = cmd
        .spawn()
        .map_err(|e| Failure::new(FailureKind::Gh, format!("could not run gh: {e}")))?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    match rx.recv_timeout(NETWORK_TIMEOUT) {
        Ok(Ok(out)) => Ok((
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )),
        Ok(Err(e)) => Err(Failure::new(FailureKind::Gh, format!("gh failed: {e}"))),
        Err(_) => Err(Failure::new(
            FailureKind::Gh,
            format!("gh timed out after {}s", NETWORK_TIMEOUT.as_secs()),
        )),
    }
}

/// The script the `land` terminal runs: print the log from the start, follow
/// it, and return once the end marker is written (`tail --pid` is not
/// portable, so a watcher stops the tail).
pub fn tail_script(log: &Path) -> String {
    format!(
        "#!/bin/sh\n\
LOG={log}\n\
( while ! grep -q '^{LOG_END_MARKER}' \"$LOG\" 2>/dev/null; do sleep 0.2; done; sleep 0.3 ) &\n\
w=$!\n\
tail -n +1 -F \"$LOG\" 2>/dev/null &\n\
t=$!\n\
wait $w\n\
kill $t 2>/dev/null\n\
wait $t 2>/dev/null\n",
        log = shell_quote(&log.to_string_lossy())
    )
}

/// The last `n` lines of a land log (for the dialog and automation).
pub fn log_tail(path: &Path, n: usize) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].iter().map(|l| l.to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.invalid")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.invalid")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn commit(dir: &Path, file: &str, text: &str, msg: &str) {
        std::fs::write(dir.join(file), text).unwrap();
        run_git(dir, &["add", file]);
        run_git(dir, &["commit", "-q", "-m", msg]);
    }

    /// A repo on `base` with one commit, and a feature worktree `feat`
    /// with one commit of its own.
    struct Fixture {
        _dir: tempfile::TempDir,
        repo: PathBuf,
        wt: PathBuf,
        state: PathBuf,
    }

    fn fixture(base: &str) -> Fixture {
        let dir = tempfile::TempDir::new().unwrap();
        let repo = dir.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        run_git(&repo, &["init", "-q", "-b", base]);
        run_git(&repo, &["config", "commit.gpgsign", "false"]);
        commit(&repo, "a.txt", "one\n", "init");
        let wt = dir.path().join("wt");
        run_git(
            &repo,
            &["worktree", "add", "-q", "-b", "feat", wt.to_str().unwrap()],
        );
        run_git(&wt, &["config", "commit.gpgsign", "false"]);
        commit(&wt, "b.txt", "feature\n", "feature work");
        let state = dir.path().join("state");
        Fixture {
            _dir: dir,
            repo,
            wt,
            state,
        }
    }

    fn input(f: &Fixture, config: LandConfig) -> PreflightInput {
        PreflightInput {
            feature: "feat".into(),
            project: "p".into(),
            repo: f.repo.clone(),
            workdir: f.wt.clone(),
            is_worktree: true,
            branch: "feat".into(),
            config,
            probe_gh: false,
            extra_blockers: vec![],
            extra_warnings: vec![],
        }
    }

    fn job(f: &Fixture, pre: &Preflight) -> Job {
        Job {
            feature_id: "fid".into(),
            feature: "feat".into(),
            repo: f.repo.clone(),
            workdir: f.wt.clone(),
            branch: pre.branch.clone(),
            strategy: pre.strategy.clone(),
            has_remote: pre.facts_has_remote,
            state_dir: f.state.clone(),
            log: log_path(&f.state, "fid"),
            env: vec![("AIW_PORT".into(), "4321".into())],
            pre_land: None,
            post_land: None,
            summary: "Add the thing".into(),
            gh: "gh".into(),
            before_finish: None,
        }
    }

    fn run_job(job: Job) -> Outcome {
        let (tx, _rx) = std::sync::mpsc::channel();
        run(job, tx)
    }

    #[test]
    fn config_merges_project_over_global_field_by_field() {
        let global = LandConfig {
            update: Some(UpdateStrategy::Merge),
            check_command: Some("make test".into()),
            remote: Some("upstream".into()),
            ..Default::default()
        };
        let project = LandConfig {
            check_command: Some("cargo test".into()),
            finish: Some(Finish::Pr),
            ..Default::default()
        };
        let merged = project.merged_over(&global);
        assert_eq!(merged.update, Some(UpdateStrategy::Merge));
        assert_eq!(merged.check_command.as_deref(), Some("cargo test"));
        assert_eq!(merged.finish, Some(Finish::Pr));
        assert_eq!(merged.remote.as_deref(), Some("upstream"));
    }

    #[test]
    fn config_parses_from_json() {
        let config: LandConfig = serde_json::from_str(
            r#"{"update":"merge","finish":"local_merge","check_command":"npm test","delete_branch":false}"#,
        )
        .unwrap();
        assert_eq!(config.update, Some(UpdateStrategy::Merge));
        assert_eq!(config.finish, Some(Finish::LocalMerge));
        assert_eq!(config.delete_branch, Some(false));
        assert!(serde_json::from_str::<LandConfig>(r#"{"finish":"squash"}"#).is_err());
    }

    #[test]
    fn defaults_pick_pr_only_for_a_signed_in_github_remote() {
        let config = LandConfig::default();
        let github = RemoteFacts {
            has_remote: true,
            github: true,
            gh_authed: true,
        };
        let s = resolve_strategy(&config, "main", &github);
        assert_eq!(s.finish, Finish::Pr);
        assert!(!s.delete_branch && !s.remove_worktree);
        assert!(s.mark_done);
        assert_eq!(s.update, UpdateStrategy::Rebase);
        assert_eq!(s.remote, "origin");

        let unsigned = RemoteFacts {
            gh_authed: false,
            ..github.clone()
        };
        let s = resolve_strategy(&config, "main", &unsigned);
        assert_eq!(s.finish, Finish::LocalMerge);
        assert!(s.finish_reason.contains("not signed in"));
        assert!(s.delete_branch && s.remove_worktree);

        let none = resolve_strategy(&config, "main", &RemoteFacts::default());
        assert_eq!(none.finish, Finish::LocalMerge);
        assert!(none.finish_reason.contains("no remote"));

        let configured = LandConfig {
            finish: Some(Finish::None),
            remove_worktree: Some(true),
            ..Default::default()
        };
        let s = resolve_strategy(&configured, "main", &github);
        assert_eq!(s.finish, Finish::None);
        assert!(s.remove_worktree && !s.delete_branch);
    }

    #[test]
    fn gh_output_is_read_for_created_existing_and_failed() {
        assert_eq!(
            parse_gh_create(
                true,
                "https://github.com/o/r/pull/7\n",
                "Creating pull request…"
            ),
            GhCreate::Created("https://github.com/o/r/pull/7".into())
        );
        assert_eq!(
            parse_gh_create(
                false,
                "",
                "a pull request for branch \"feat\" into branch \"main\" already exists:\nhttps://github.com/o/r/pull/3\n"
            ),
            GhCreate::Existing("https://github.com/o/r/pull/3".into())
        );
        assert_eq!(
            parse_gh_create(
                false,
                "",
                "pull request create failed: GraphQL: No commits\n"
            ),
            GhCreate::Failed("pull request create failed: GraphQL: No commits".into())
        );
    }

    #[test]
    fn worktree_porcelain_finds_where_a_branch_is_checked_out() {
        let porcelain = "worktree /r\nHEAD aaa\nbranch refs/heads/main\n\nworktree /w\nHEAD bbb\nbranch refs/heads/feat\n\nworktree /d\nHEAD ccc\ndetached\n";
        assert_eq!(checkout_of(porcelain, "main"), Some(PathBuf::from("/r")));
        assert_eq!(checkout_of(porcelain, "feat"), Some(PathBuf::from("/w")));
        assert_eq!(checkout_of(porcelain, "other"), None);
        assert_eq!(checkout_of(porcelain, "mai"), None);
    }

    #[test]
    fn templates_and_summaries() {
        assert_eq!(
            render_template(
                "{feature}: {summary} ({branch} → {base})",
                "auth",
                "auth-fix",
                "main",
                "Fix login"
            ),
            "auth: Fix login (auth-fix → main)"
        );
        assert_eq!(
            summary_line("\n\n  Fix the login bug  \nmore"),
            "Fix the login bug"
        );
        let long = "x".repeat(100);
        assert_eq!(summary_line(&long).chars().count(), 72);
        assert_eq!(shell_quote("npm test"), "'npm test'");
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
        assert_eq!(shell_quote("make"), "make");
    }

    #[test]
    fn preflight_finds_base_ahead_behind_and_plan() {
        let f = fixture("trunk");
        commit(&f.repo, "c.txt", "base moved\n", "base work");
        let pre = compute_preflight(&input(&f, LandConfig::default()));
        assert_eq!(pre.base, "trunk");
        assert!(pre.base_source.contains("trunk"), "{}", pre.base_source);
        assert_eq!((pre.ahead, pre.behind), (1, 1));
        assert_eq!(pre.strategy.finish, Finish::LocalMerge);
        assert!(pre.landable(), "{:?}", pre.blockers);
        assert_eq!(
            pre.base_checkout.as_ref().map(|c| c.path.clone()),
            Some(f.repo.clone())
        );
        assert!(
            pre.plan.iter().any(|c| c == "git rebase trunk"),
            "{:?}",
            pre.plan
        );
        assert!(
            pre.plan.iter().any(|c| c.contains("merge --ff-only feat")),
            "{:?}",
            pre.plan
        );
        assert!(
            pre.plan.iter().any(|c| c == "git branch -d feat"),
            "{:?}",
            pre.plan
        );
        assert!(
            pre.warnings.iter().any(|w| w.contains("1 commit(s) ahead")),
            "{:?}",
            pre.warnings
        );
    }

    #[test]
    fn preflight_blocks_dirty_repo_dir_base_branch_and_nothing_to_land() {
        let f = fixture("main");
        std::fs::write(f.wt.join("b.txt"), "changed\n").unwrap();
        std::fs::write(f.wt.join("new.txt"), "x\n").unwrap();
        let pre = compute_preflight(&input(&f, LandConfig::default()));
        assert!(
            pre.blockers
                .iter()
                .any(|b| b.contains("1 changed, 1 untracked")),
            "{:?}",
            pre.blockers
        );

        let mut repo_dir = input(&f, LandConfig::default());
        repo_dir.is_worktree = false;
        repo_dir.workdir = f.repo.clone();
        let pre = compute_preflight(&repo_dir);
        assert!(
            pre.blockers
                .iter()
                .any(|b| b.contains("repository directory"))
        );

        let base = LandConfig {
            base: Some("feat".into()),
            ..Default::default()
        };
        run_git(&f.wt, &["checkout", "-q", "--", "b.txt"]);
        std::fs::remove_file(f.wt.join("new.txt")).unwrap();
        let pre = compute_preflight(&input(&f, base));
        assert!(
            pre.blockers
                .iter()
                .any(|b| b.contains("base branch itself")),
            "{:?}",
            pre.blockers
        );

        let g = fixture("main");
        run_git(&g.wt, &["reset", "-q", "--hard", "main"]);
        let pre = compute_preflight(&input(&g, LandConfig::default()));
        assert!(
            pre.blockers.iter().any(|b| b.contains("nothing to land")),
            "{:?}",
            pre.blockers
        );
    }

    #[test]
    fn preflight_blocks_a_dirty_base_checkout_for_local_merge() {
        let f = fixture("main");
        std::fs::write(f.repo.join("a.txt"), "edited in main\n").unwrap();
        let pre = compute_preflight(&input(&f, LandConfig::default()));
        assert!(
            pre.blockers.iter().any(|b| b.contains("will not touch it")),
            "{:?}",
            pre.blockers
        );
        // An untracked file in the main checkout does not count.
        let g = fixture("main");
        std::fs::write(g.repo.join("scratch.txt"), "x").unwrap();
        assert!(compute_preflight(&input(&g, LandConfig::default())).landable());
    }

    #[test]
    fn local_merge_with_rebase_and_a_passing_check_fast_forwards_the_checked_out_base() {
        let f = fixture("main");
        commit(&f.repo, "c.txt", "base moved\n", "base work");
        let config = LandConfig {
            check_command: Some(
                "test \"$AIW_PORT\" = 4321 && test -f b.txt && test -f c.txt".into(),
            ),
            ..Default::default()
        };
        let pre = compute_preflight(&input(&f, config));
        assert!(pre.landable(), "{:?}", pre.blockers);
        let out = run_job(job(&f, &pre));
        assert!(out.ok, "{:?} {:?}", out.failure, log_tail(&out.log, 30));
        let main = run_git(&f.repo, &["rev-parse", "main"]);
        let feat = run_git(&f.wt, &["rev-parse", "HEAD"]);
        assert_eq!(main, feat);
        assert_eq!(out.merged.as_deref(), Some(feat.as_str()));
        // The main checkout's files follow (ff-merge, not update-ref).
        assert!(f.repo.join("b.txt").exists());
        assert_eq!(run_git(&f.repo, &["status", "--porcelain"]), "");
        let tail = log_tail(&out.log, 3).join("\n");
        assert!(tail.contains(LOG_END_MARKER), "{tail}");
    }

    #[test]
    fn local_merge_updates_a_base_not_checked_out_anywhere_with_the_expected_old_value() {
        let f = fixture("main");
        // Move the main checkout off `main`, so main is checked out nowhere.
        run_git(&f.repo, &["checkout", "-q", "--detach"]);
        let pre = compute_preflight(&input(&f, LandConfig::default()));
        assert!(pre.base_checkout.is_none());
        assert!(
            pre.plan
                .iter()
                .any(|c| c.starts_with("git update-ref refs/heads/main")),
            "{:?}",
            pre.plan
        );
        let out = run_job(job(&f, &pre));
        assert!(out.ok, "{:?}", out.failure);
        assert_eq!(
            run_git(&f.repo, &["rev-parse", "main"]),
            run_git(&f.wt, &["rev-parse", "HEAD"])
        );
    }

    fn move_base(job: &Job) {
        // A commit lands on the base while the land is between its check and
        // its finish.
        let _ = Command::new("git")
            .args(["commit", "-q", "--allow-empty", "-m", "race"])
            .current_dir(&job.repo)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.invalid")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.invalid")
            .status();
    }

    fn move_detached_base(job: &Job) {
        let tree = git(&job.repo, &["rev-parse", "main^{tree}"]).unwrap();
        let parent = git(&job.repo, &["rev-parse", "main"]).unwrap();
        let out = Command::new("git")
            .args(["commit-tree", &tree, "-p", &parent, "-m", "race"])
            .current_dir(&job.repo)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@example.invalid")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@example.invalid")
            .output()
            .unwrap();
        let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
        git(&job.repo, &["update-ref", "refs/heads/main", &sha]).unwrap();
    }

    #[test]
    fn a_base_that_moves_during_the_land_stops_it_without_merging() {
        let f = fixture("main");
        let pre = compute_preflight(&input(&f, LandConfig::default()));
        let mut j = job(&f, &pre);
        j.before_finish = Some(move_base);
        let before_feat = run_git(&f.wt, &["rev-parse", "HEAD"]);
        let out = run_job(j);
        assert!(!out.ok);
        assert_eq!(out.failure.as_ref().unwrap().kind, FailureKind::BaseMoved);
        assert_ne!(run_git(&f.repo, &["rev-parse", "main"]), before_feat);

        // The same through update-ref when the base is checked out nowhere.
        let g = fixture("main");
        run_git(&g.repo, &["checkout", "-q", "--detach"]);
        let pre = compute_preflight(&input(&g, LandConfig::default()));
        let mut j = job(&g, &pre);
        j.before_finish = Some(move_detached_base);
        let out = run_job(j);
        assert_eq!(
            out.failure.as_ref().map(|f| f.kind.clone()),
            Some(FailureKind::BaseMoved)
        );
    }

    #[test]
    fn a_failing_check_stops_before_anything_is_merged() {
        let f = fixture("main");
        let main_before = run_git(&f.repo, &["rev-parse", "main"]);
        let config = LandConfig {
            check_command: Some("echo checking; exit 3".into()),
            ..Default::default()
        };
        let pre = compute_preflight(&input(&f, config));
        let out = run_job(job(&f, &pre));
        let failure = out.failure.unwrap();
        assert_eq!(failure.kind, FailureKind::Check);
        assert_eq!(failure.reason, "check failed (exit 3)");
        assert_eq!(run_git(&f.repo, &["rev-parse", "main"]), main_before);
        let log = log_tail(&out.log, 20).join("\n");
        assert!(log.contains("checking"), "{log}");
    }

    #[test]
    fn a_check_that_dirties_the_worktree_stops_the_land() {
        let f = fixture("main");
        let config = LandConfig {
            check_command: Some("echo formatted > b.txt".into()),
            ..Default::default()
        };
        let pre = compute_preflight(&input(&f, config));
        let out = run_job(job(&f, &pre));
        assert_eq!(out.failure.unwrap().kind, FailureKind::DirtyAfterCheck);
    }

    #[test]
    fn a_rebase_conflict_is_left_in_progress_and_can_be_aborted() {
        let f = fixture("main");
        commit(&f.repo, "b.txt", "base version\n", "conflicting base");
        let pre = compute_preflight(&input(&f, LandConfig::default()));
        let out = run_job(job(&f, &pre));
        let failure = out.failure.unwrap();
        assert_eq!(failure.kind, FailureKind::Conflict);
        assert_eq!(failure.in_progress, Some(InProgress::Rebase));
        assert_eq!(in_progress(&f.wt), Some(InProgress::Rebase));
        let blocked = compute_preflight(&input(&f, LandConfig::default()));
        assert!(
            blocked
                .blockers
                .iter()
                .any(|b| b.contains("rebase is in progress"))
        );
        git(&f.wt, &InProgress::Rebase.abort_args()).unwrap();
        assert_eq!(in_progress(&f.wt), None);
    }

    #[test]
    fn a_merge_update_conflict_is_reported_as_a_merge() {
        let f = fixture("main");
        commit(&f.repo, "b.txt", "base version\n", "conflicting base");
        let config = LandConfig {
            update: Some(UpdateStrategy::Merge),
            ..Default::default()
        };
        let pre = compute_preflight(&input(&f, config));
        let out = run_job(job(&f, &pre));
        assert_eq!(out.failure.unwrap().in_progress, Some(InProgress::Merge));
        git(&f.wt, &InProgress::Merge.abort_args()).unwrap();
    }

    #[test]
    fn a_second_land_in_the_same_repo_is_refused_while_the_lock_is_held() {
        let f = fixture("main");
        let held = RepoLock::try_acquire(&f.state, &f.repo).unwrap();
        assert!(RepoLock::try_acquire(&f.state, &f.repo).is_err());
        let pre = compute_preflight(&input(&f, LandConfig::default()));
        let out = run_job(job(&f, &pre));
        assert_eq!(out.failure.unwrap().kind, FailureKind::Locked);
        drop(held);
        // Another test thread's fork can hold a copy of the descriptor for
        // the instant before its exec closes it; the lock frees right after.
        let freed = (0..100).any(|_| {
            let ok = RepoLock::try_acquire(&f.state, &f.repo).is_ok();
            if !ok {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            ok
        });
        assert!(freed);
    }

    #[test]
    fn hooks_run_with_the_land_env_and_a_failing_pre_land_stops() {
        let f = fixture("main");
        let marker = f.state.join("post.txt");
        std::fs::create_dir_all(&f.state).unwrap();
        let pre = compute_preflight(&input(&f, LandConfig::default()));
        let mut j = job(&f, &pre);
        j.pre_land = Some(HookRun {
            name: "pre_land",
            script: "test \"$AIW_LAND_BRANCH\" = feat && test \"$AIW_LAND_BASE\" = main".into(),
        });
        j.post_land = Some(HookRun {
            name: "post_land",
            script: format!(
                "echo \"$AIW_LAND_RESULT $AIW_LAND_FINISH $AIW_LAND_STRATEGY\" > {}",
                marker.display()
            ),
        });
        let out = run_job(j.clone());
        assert!(out.ok, "{:?}", out.failure);
        assert_eq!(
            std::fs::read_to_string(&marker).unwrap().trim(),
            "success local_merge rebase"
        );

        let g = fixture("main");
        let pre = compute_preflight(&input(&g, LandConfig::default()));
        let mut j = job(&g, &pre);
        let marker = g.state.join("post.txt");
        std::fs::create_dir_all(&g.state).unwrap();
        j.pre_land = Some(HookRun {
            name: "pre_land",
            script: "exit 5".into(),
        });
        j.post_land = Some(HookRun {
            name: "post_land",
            script: format!("echo \"$AIW_LAND_RESULT\" > {}", marker.display()),
        });
        let main_before = run_git(&g.repo, &["rev-parse", "main"]);
        let out = run_job(j);
        assert_eq!(out.failure.unwrap().reason, "pre_land hook failed (exit 5)");
        assert_eq!(run_git(&g.repo, &["rev-parse", "main"]), main_before);
        assert_eq!(std::fs::read_to_string(&marker).unwrap().trim(), "failed");
    }

    #[test]
    fn finish_none_updates_and_checks_only() {
        let f = fixture("main");
        commit(&f.repo, "c.txt", "base\n", "base work");
        let config = LandConfig {
            finish: Some(Finish::None),
            ..Default::default()
        };
        let pre = compute_preflight(&input(&f, config));
        let main_before = run_git(&f.repo, &["rev-parse", "main"]);
        let out = run_job(job(&f, &pre));
        assert!(out.ok);
        assert_eq!(run_git(&f.repo, &["rev-parse", "main"]), main_before);
        // rebased onto the moved base
        assert!(f.wt.join("c.txt").exists());
    }

    #[test]
    fn pr_without_a_remote_is_blocked() {
        let f = fixture("main");
        let config = LandConfig {
            finish: Some(Finish::Pr),
            ..Default::default()
        };
        let pre = compute_preflight(&input(&f, config));
        assert!(
            pre.blockers
                .iter()
                .any(|b| b.contains("no `origin` remote")),
            "{:?}",
            pre.blockers
        );
    }

    #[test]
    fn pr_finish_pushes_and_opens_or_reports_the_pr_with_a_fake_gh() {
        let f = fixture("main");
        let bare = f.state.parent().unwrap().join("remote.git");
        run_git(
            f.state.parent().unwrap(),
            &["init", "-q", "--bare", bare.to_str().unwrap()],
        );
        run_git(
            &f.repo,
            &["remote", "add", "origin", bare.to_str().unwrap()],
        );
        run_git(&f.repo, &["push", "-q", "origin", "main"]);
        run_git(&f.repo, &["fetch", "-q", "origin"]);
        let bin = f.state.parent().unwrap().join("fake-gh");
        let calls = f.state.parent().unwrap().join("gh.calls");
        std::fs::write(
            &bin,
            format!(
                "#!/bin/sh\necho \"$@\" >> {calls}\nif [ -f {calls}.exists ]; then echo 'a pull request for branch \"feat\" into branch \"main\" already exists:' >&2; echo https://github.com/o/r/pull/9 >&2; exit 1; fi\necho https://github.com/o/r/pull/8\n",
                calls = calls.display()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let config = LandConfig {
            finish: Some(Finish::Pr),
            pr_title: Some("{feature}: {summary}".into()),
            pr_body: Some("into {base}".into()),
            ..Default::default()
        };
        let pre = compute_preflight(&input(&f, config.clone()));
        assert!(pre.landable(), "{:?}", pre.blockers);
        assert_eq!(pre.upstream, "origin/main");
        let mut j = job(&f, &pre);
        j.gh = bin.to_string_lossy().into_owned();
        let out = run_job(j.clone());
        assert!(out.ok, "{:?} {:?}", out.failure, log_tail(&out.log, 30));
        assert_eq!(out.pr_url.as_deref(), Some("https://github.com/o/r/pull/8"));
        assert!(!out.pr_existed);
        // The branch reached the remote.
        assert_eq!(
            run_git(&bare, &["rev-parse", "refs/heads/feat"]),
            run_git(&f.wt, &["rev-parse", "HEAD"])
        );
        let call = std::fs::read_to_string(&calls).unwrap();
        assert!(
            call.contains(
                "pr create --base main --head feat --title feat: Add the thing --body into main"
            ),
            "{call}"
        );
        // main itself is untouched by a PR finish.
        assert_ne!(
            run_git(&f.repo, &["rev-parse", "main"]),
            run_git(&f.wt, &["rev-parse", "HEAD"])
        );

        // An open PR is reported, not duplicated.
        std::fs::write(format!("{}.exists", calls.display()), "").unwrap();
        let out = run_job(j);
        assert!(out.ok, "{:?}", out.failure);
        assert!(out.pr_existed);
        assert_eq!(out.pr_url.as_deref(), Some("https://github.com/o/r/pull/9"));
    }

    #[test]
    fn base_resolution_prefers_config_then_origin_head_then_known_names() {
        let f = fixture("develop");
        assert_eq!(
            resolve_base(&f.repo, "feat", Some("release"), "origin").map(|b| b.0),
            Some("release".into())
        );
        assert_eq!(
            resolve_base(&f.repo, "feat", None, "origin").map(|b| b.0),
            Some("develop".into())
        );
        run_git(&f.repo, &["branch", "main"]);
        assert_eq!(
            resolve_base(&f.repo, "feat", None, "origin").map(|b| b.0),
            Some("main".into())
        );
        let g = fixture("weird");
        assert_eq!(
            resolve_base(&g.repo, "feat", None, "origin"),
            Some(("weird".into(), "the main checkout's branch".into()))
        );
    }

    #[test]
    fn the_tail_script_prints_the_log_and_returns_at_the_marker() {
        let dir = tempfile::TempDir::new().unwrap();
        let log = dir.path().join("x.log");
        std::fs::write(&log, "step one\n").unwrap();
        let script = dir.path().join("x.tail.sh");
        std::fs::write(&script, tail_script(&log)).unwrap();
        let child = Command::new("sh")
            .arg(&script)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        std::thread::sleep(Duration::from_millis(400));
        let mut f = OpenOptions::new().append(true).open(&log).unwrap();
        writeln!(f, "step two").unwrap();
        writeln!(f, "{LOG_END_MARKER}: ok").unwrap();
        drop(f);
        let out = child.wait_with_output().unwrap();
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            text.contains("step one") && text.contains("step two"),
            "{text}"
        );
        assert!(text.contains(LOG_END_MARKER), "{text}");
    }

    #[test]
    fn log_paths_and_lock_paths_are_safe_and_stable() {
        let state = Path::new("/s");
        assert_eq!(
            log_path(state, "ab/c d"),
            PathBuf::from("/s/land/ab_c_d.log")
        );
        assert_eq!(
            lock_path(state, Path::new("/r")),
            lock_path(state, Path::new("/r"))
        );
        assert_ne!(
            lock_path(state, Path::new("/r")),
            lock_path(state, Path::new("/q"))
        );
    }
}
