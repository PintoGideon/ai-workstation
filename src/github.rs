//! GitHub access via the `gh` CLI.
//!
//! A reusable tool-manager (in the spirit of [`TmuxManager`], [`WorktreeManager`],
//! and [`ClaudeLauncher`]) wrapping the GitHub CLI: preconditions (gh installed,
//! authenticated) plus typed PR queries. All work happens in Rust here — no agent
//! tokens are spent. Keep this layer feature-agnostic; feature-specific logic
//! (e.g. PR comment review) should live in its own module and call into this one.
//!
//! [`TmuxManager`]: crate::tmux::TmuxManager
//! [`WorktreeManager`]: crate::worktree::WorktreeManager
//! [`ClaudeLauncher`]: crate::claude::ClaudeLauncher

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::path::Path;
use std::process::Command;

/// Static-method wrapper around the `gh` CLI. Add general GitHub operations
/// here (PRs, issues, reviews, repo metadata) so they can be reused across
/// features rather than coupled to any one of them.
pub struct GhCli;

/// Shared GitHub transport boundary for app workflows.
///
/// The production adapter remains the existing `gh` CLI wrapper. Keeping the
/// boundary as a small trait lets focused tests supply a deterministic
/// transport for the issue workflows.
pub trait GithubTransport {
    fn check_available(&self) -> Result<()>;
    fn check_auth(&self) -> Result<()>;
    fn list_issues(
        &self,
        workdir: &Path,
        repository: &GithubRepository,
        page: u32,
        per_page: u32,
    ) -> Result<GithubIssuePage>;
    fn issue_comment_bodies(
        &self,
        workdir: &Path,
        repository: &GithubRepository,
        number: u32,
    ) -> Result<Vec<String>>;
    fn post_issue_comment(
        &self,
        workdir: &Path,
        repository: &GithubRepository,
        number: u32,
        body: &str,
    ) -> Result<()>;
}

/// A lightweight open issue snapshot used by the issue-fixer list and prompt.
/// Pull requests are represented by the same GitHub REST endpoint and are
/// filtered before this type reaches the app workflow.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GithubIssue {
    pub number: u32,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default, rename = "html_url")]
    pub url: String,
    #[serde(default)]
    pub labels: Vec<GithubIssueLabel>,
    #[serde(default, rename = "updated_at")]
    pub updated_at: String,
    #[serde(default, rename = "pull_request")]
    pub(crate) pull_request: Option<serde_json::Value>,
}

impl GithubIssue {
    pub fn is_pull_request(&self) -> bool {
        self.pull_request.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct GithubIssueLabel {
    #[serde(default)]
    pub name: String,
}

/// One explicit page from GitHub's open-issue endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GithubIssuePage {
    pub issues: Vec<GithubIssue>,
    pub page: u32,
    pub per_page: u32,
    pub has_next_page: bool,
}

impl GhCli {
    /// Check that a working `gh` binary is on PATH. Mirrors
    /// `TmuxManager::check_available` / `ClaudeLauncher::check_available`.
    pub fn check_available() -> Result<()> {
        let output = Command::new("gh").arg("--version").output().context(
            "GitHub CLI (`gh`) not found. Install it from https://cli.github.com to use GitHub features.",
        )?;
        if !output.status.success() {
            bail!("GitHub CLI (`gh`) is not working correctly. Try `gh --version`.");
        }
        Ok(())
    }

    /// Check that `gh` is authenticated. We trust the exit code rather than
    /// parsing the human-readable text, which changes between versions.
    pub fn check_auth() -> Result<()> {
        let output = Command::new("gh")
            .args(["auth", "status"])
            .output()
            .context("Failed to run `gh auth status`.")?;
        if !output.status.success() {
            bail!(
                "`gh` is not authenticated. Run `! gh auth login` to sign in (login is interactive)."
            );
        }
        Ok(())
    }

    /// Fetch one explicit page of open issues for a canonical repository.
    /// GitHub's issues endpoint also returns pull requests, so those rows are
    /// removed here before the page reaches the issue workflow. The response
    /// is intentionally parsed strictly: a malformed payload is a recoverable
    /// error, never an empty issue list that looks authoritative.
    pub fn list_issues(
        workdir: &Path,
        repository: &GithubRepository,
        page: u32,
        per_page: u32,
    ) -> Result<GithubIssuePage> {
        let page = page.max(1);
        let per_page = per_page.clamp(1, 100);
        let endpoint = format!(
            "repos/{}/{}/issues?state=open&sort=updated&direction=desc&page={page}&per_page={per_page}",
            repository.owner, repository.name
        );
        let mut cmd = Command::new("gh");
        cmd.args(["api", &endpoint]);
        if repository.host != "github.com" {
            cmd.args(["--hostname", &repository.host]);
        }
        let output = cmd
            .current_dir(workdir)
            .output()
            .context("Failed to run `gh api` while loading GitHub issues.")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if is_rate_limited(&stderr) {
                bail!("GitHub rate limit exceeded while loading issues; retry later");
            }
            bail!("`gh api {endpoint}` failed: {}", stderr.trim());
        }

        parse_issue_page(&output.stdout, page, per_page)
    }

    pub fn issue_comment_bodies(
        workdir: &Path,
        repository: &GithubRepository,
        number: u32,
    ) -> Result<Vec<String>> {
        #[derive(Deserialize)]
        struct BodyOnly {
            #[serde(default)]
            body: String,
        }

        let endpoint = format!(
            "repos/{}/{}/issues/{number}/comments?per_page=100",
            repository.owner, repository.name
        );
        let mut cmd = Command::new("gh");
        cmd.args(["api", "--paginate", "--slurp"]);
        if repository.host != "github.com" {
            cmd.args(["--hostname", &repository.host]);
        }
        let output = cmd
            .arg(endpoint)
            .current_dir(workdir)
            .output()
            .context("Failed to run `gh api` while reconciling issue comments.")?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        if is_rate_limited(&stdout) || is_rate_limited(&stderr) {
            bail!("GitHub rate limit exceeded while checking issue comments; retry later");
        }
        if !output.status.success() {
            bail!(
                "`gh api` failed while checking issue comments: {}",
                stderr.trim()
            );
        }
        let pages: Vec<Vec<BodyOnly>> = serde_json::from_slice(&output.stdout)
            .context("GitHub returned malformed issue comments")?;
        Ok(pages
            .into_iter()
            .flatten()
            .map(|comment| comment.body)
            .collect())
    }

    pub fn post_feature_issue_comment(
        workdir: &Path,
        repository: &GithubRepository,
        number: u32,
        body: &str,
    ) -> Result<()> {
        let endpoint = format!(
            "repos/{}/{}/issues/{number}/comments",
            repository.owner, repository.name
        );
        let mut cmd = Command::new("gh");
        cmd.args(["api", "--method", "POST"]);
        if repository.host != "github.com" {
            cmd.args(["--hostname", &repository.host]);
        }
        cmd.arg(endpoint).args(["-f", &format!("body={body}")]);
        cmd.current_dir(workdir);
        run_write(cmd, "post issue feature comment")
    }

    /// Every open PR in one repository, with each one's unresolved
    /// review-thread count, in a single query.
    ///
    /// This replaces a `gh pr view` *per feature* plus a thread query per PR.
    /// Cost now scales with the number of **repositories** on the dashboard
    /// rather than the number of features, which is what made the badge sweep
    /// able to exhaust an account's hourly GraphQL budget: 34 features cost ~68
    /// points a sweep where the repositories behind them cost a handful.
    ///
    /// Callers match a feature's branch against [`OpenPr::head_ref`].
    ///
    /// **Fork limitation.** This asks the repository `origin` points at. When a
    /// worktree's `origin` is a fork and the PR lives on the upstream
    /// repository, the PR is not in this result and the feature shows no badge.
    /// `gh pr view` resolved that case because it knows the base repo. Covering
    /// it means either pulling every open PR of the upstream (unbounded on a
    /// busy project) or going back to a query per branch, which is the cost
    /// this replaced — so it is deliberately left out rather than paid for on
    /// every sweep.
    pub fn open_prs(
        workdir: &Path,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<OpenPr>, GhGraphqlError> {
        let mut all = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut cmd = Command::new("gh");
            cmd.args(["api", "graphql", "-f", &format!("query={OPEN_PRS_QUERY}")])
                .args(["-F", &format!("owner={owner}")])
                .args(["-F", &format!("repo={repo}")])
                .current_dir(workdir);
            if let Some(c) = &cursor {
                cmd.args(["-F", &format!("cursor={c}")]);
            }
            let output = cmd.output().map_err(|e| {
                GhGraphqlError::Failed(format!("failed to run `gh api graphql`: {e}"))
            })?;

            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            if is_rate_limited(&stdout) || is_rate_limited(&stderr) {
                return Err(GhGraphqlError::RateLimited);
            }
            if !output.status.success() {
                return Err(GhGraphqlError::Failed(format!(
                    "`gh api graphql` (open PRs) failed: {}",
                    stderr.trim()
                )));
            }

            let (mut page, next) = parse_open_prs_page(&output.stdout)
                .map_err(|e| GhGraphqlError::Failed(e.to_string()))?;
            all.append(&mut page);
            match next {
                Some(c) => cursor = Some(c),
                None => break,
            }
        }
        Ok(all)
    }

    /// Resolve terminal (merged/closed) PR state for a batch of branches in
    /// one repository. Meant to be called with the branches [`open_prs`]
    /// reported as absent — a branch already in the open set can't also be
    /// terminal — so this is a *second*, smaller call on top of that one, not
    /// a replacement for it.
    ///
    /// One GraphQL call per chunk of [`TERMINAL_PRS_CHUNK`] branches, each
    /// aliased (`a0`, `a1`, …) to a `pullRequests(headRefName:…)` sub-query
    /// asking only for the single most-recently-updated merged-or-closed PR.
    /// Branch names never enter the query text — they travel as `-F`
    /// variables (`$b0`, `$b1`, …) — so this stays batched-per-repository
    /// (matching [`open_prs`]'s cost model) without branch text ever needing
    /// escaping.
    pub fn terminal_prs(
        workdir: &Path,
        owner: &str,
        repo: &str,
        branches: &[String],
    ) -> Result<HashMap<String, TerminalPr>, GhGraphqlError> {
        let mut out = HashMap::new();
        for chunk in branches.chunks(TERMINAL_PRS_CHUNK) {
            let query = build_terminal_prs_query(chunk);
            let mut cmd = Command::new("gh");
            cmd.args(["api", "graphql", "-f", &format!("query={query}")])
                .args(["-F", &format!("owner={owner}")])
                .args(["-F", &format!("repo={repo}")])
                .current_dir(workdir);
            for (i, branch) in chunk.iter().enumerate() {
                cmd.args(["-F", &format!("b{i}={branch}")]);
            }
            let output = cmd.output().map_err(|e| {
                GhGraphqlError::Failed(format!("failed to run `gh api graphql`: {e}"))
            })?;

            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            if is_rate_limited(&stdout) || is_rate_limited(&stderr) {
                return Err(GhGraphqlError::RateLimited);
            }
            if !output.status.success() {
                return Err(GhGraphqlError::Failed(format!(
                    "`gh api graphql` (terminal PRs) failed: {}",
                    stderr.trim()
                )));
            }

            let page = parse_terminal_prs_page(&output.stdout, chunk)
                .map_err(|e| GhGraphqlError::Failed(e.to_string()))?;
            out.extend(page);
        }
        Ok(out)
    }
}

impl GithubTransport for GhCli {
    fn check_available(&self) -> Result<()> {
        Self::check_available()
    }

    fn check_auth(&self) -> Result<()> {
        Self::check_auth()
    }

    fn list_issues(
        &self,
        workdir: &Path,
        repository: &GithubRepository,
        page: u32,
        per_page: u32,
    ) -> Result<GithubIssuePage> {
        Self::list_issues(workdir, repository, page, per_page)
    }

    fn issue_comment_bodies(
        &self,
        workdir: &Path,
        repository: &GithubRepository,
        number: u32,
    ) -> Result<Vec<String>> {
        Self::issue_comment_bodies(workdir, repository, number)
    }

    fn post_issue_comment(
        &self,
        workdir: &Path,
        repository: &GithubRepository,
        number: u32,
        body: &str,
    ) -> Result<()> {
        Self::post_feature_issue_comment(workdir, repository, number, body)
    }
}

/// Run a `gh` write command, mapping a missing-`repo`-scope failure (the common
/// first-write 403) to an actionable message instead of a raw HTTP error.
fn run_write(mut cmd: Command, what: &str) -> Result<()> {
    let output = cmd
        .output()
        .with_context(|| format!("Failed to run `gh api` ({what})."))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if is_missing_write_scope(&stderr) {
        bail!(
            "Posting to GitHub needs the `repo` scope. Run `! gh auth refresh -s repo` and try again."
        );
    }
    bail!("`gh api` ({what}) failed: {}", stderr.trim());
}

/// Whether a `gh` failure looks like a missing-write-scope / 403, so we can
/// point the user at `gh auth refresh -s repo` rather than show a raw error.
fn is_missing_write_scope(stderr: &str) -> bool {
    let s = stderr.to_lowercase();
    s.contains("http 403") || (s.contains("403") && s.contains("scope")) || s.contains("must have")
}

/// The dashboard badge's per-repository query.
///
/// Written as a real multi-line string, not with `\` continuations. A
/// continuation eats the newline *and* the following indentation, so a line
/// ending in one field name and the next beginning with another silently fuses
/// them — which is exactly how this query first shipped, asking GitHub for
/// `headRefOidreviewThreads` and failing every call. GraphQL is
/// whitespace-insensitive, so real newlines cost nothing.
///
/// `first:50` rather than 100: cost scales with nodes actually returned, and a
/// repository with 50 open PRs already returns their threads too. Pagination
/// covers the rest.
const OPEN_PRS_QUERY: &str = "
            query($owner:String!,$repo:String!,$cursor:String){
              repository(owner:$owner,name:$repo){
                pullRequests(states:OPEN,first:50,after:$cursor){
                  pageInfo{hasNextPage endCursor}
                  nodes{
                    number
                    headRefName
                    headRefOid
                    reviewThreads(first:100){nodes{isResolved}}
                  }
                }
              }
            }";

/// One open pull request, as the dashboard badge needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenPr {
    pub number: u32,
    /// GitHub's `headRefName` — the branch the PR is *from*, which is what a
    /// feature's branch is matched against.
    pub head_ref: String,
    pub head_sha: String,
    pub unresolved_threads: usize,
}

/// A pull request's *terminal* state — the two outcomes a PR never leaves.
///
/// GitHub's own `state` enum spells both as `CLOSED` at the REST layer and
/// distinguishes them with a separate `merged` flag; GraphQL's
/// `PullRequestState` is the honest one (`OPEN`/`CLOSED`/`MERGED`) and is what
/// this reads. The distinction is the whole point of the badge: a merged
/// branch is finished work, a closed one is abandoned work, and before this
/// existed both rendered identically to a branch that never had a PR at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalPrState {
    Merged,
    /// Closed *without* being merged.
    Closed,
}

impl TerminalPrState {
    /// Parse GitHub's `PullRequestState` string. `OPEN` is deliberately not a
    /// terminal state and maps to `None` rather than to a third variant — an
    /// open PR is already the open sweep's business.
    pub fn from_gh(state: &str) -> Option<Self> {
        match state {
            "MERGED" => Some(Self::Merged),
            "CLOSED" => Some(Self::Closed),
            _ => None,
        }
    }

    /// The word shown in the dashboard badge.
    pub fn label(self) -> &'static str {
        match self {
            Self::Merged => "merged",
            Self::Closed => "closed",
        }
    }
}

/// One pull request that has reached a terminal state, as the dashboard badge
/// needs it. Deliberately smaller than [`OpenPr`]: a finished PR has no
/// unresolved-thread count worth chasing and no head SHA worth pinning to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalPr {
    pub number: u32,
    pub state: TerminalPrState,
    /// `mergedAt` for a merged PR, `closedAt` for a closed one — an ISO-8601
    /// timestamp as GitHub returns it, stored verbatim rather than parsed. It
    /// is only ever persisted and shown, never compared.
    pub at: String,
}

/// Canonical identity of the GitHub repository associated with a project.
/// Host is retained so GitHub Enterprise repositories never collide with
/// github.com repositories that happen to use the same owner/name.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GithubRepository {
    pub host: String,
    pub owner: String,
    pub name: String,
}

impl GithubRepository {
    /// Parse a GitHub remote in HTTPS, `ssh://`, or scp-style SSH form.
    ///
    /// Unknown hosts are accepted as GitHub Enterprise candidates; well-known
    /// non-GitHub hosts are rejected. The later `gh` request remains the
    /// authority for whether an Enterprise host is configured and reachable.
    pub fn from_remote_url(url: &str) -> Option<Self> {
        let url = url.trim();
        if url.is_empty() {
            return None;
        }

        let (host, path) = if let Some((scheme, rest)) = url.split_once("://") {
            if !matches!(
                scheme.to_ascii_lowercase().as_str(),
                "http" | "https" | "ssh" | "git"
            ) {
                return None;
            }
            let (authority, path) = rest.split_once('/')?;
            (authority.rsplit('@').next()?.split(':').next()?, path)
        } else {
            let (authority, path) = url.split_once(':')?;
            (authority.rsplit('@').next()?, path)
        };

        let host = host.trim().trim_end_matches('/').to_ascii_lowercase();
        if host.is_empty() || is_known_non_github_host(&host) {
            return None;
        }
        let path = path.trim_matches('/');
        let path = path.strip_suffix(".git").unwrap_or(path);
        let mut segments = path.split('/').filter(|segment| !segment.is_empty());
        let owner = segments.next()?.trim();
        let name = segments.next()?.trim();
        if owner.is_empty() || name.is_empty() || segments.next().is_some() {
            return None;
        }

        Some(Self {
            host,
            owner: owner.to_ascii_lowercase(),
            name: name.to_ascii_lowercase(),
        })
    }

    /// Stable key used for duplicate lookup and persistence.
    pub fn canonical(&self) -> String {
        format!("{}/{}/{}", self.host, self.owner, self.name)
    }

    pub fn issue_url(&self, number: u32) -> String {
        format!(
            "https://{}/{}/{}/issues/{number}",
            self.host, self.owner, self.name
        )
    }
}

/// Why a selected project cannot be associated with one GitHub repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GithubRepositoryError {
    NotGitRepository,
    NoRemote,
    NoGithubRemote,
    Ambiguous { repositories: Vec<GithubRepository> },
}

impl fmt::Display for GithubRepositoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotGitRepository => write!(f, "Selected project is not a git repository"),
            Self::NoRemote => write!(f, "Selected git project has no remotes"),
            Self::NoGithubRemote => write!(f, "Selected project has no GitHub remote"),
            Self::Ambiguous { repositories } => write!(
                f,
                "Selected project has multiple GitHub repositories: {}",
                repositories
                    .iter()
                    .map(GithubRepository::canonical)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

impl std::error::Error for GithubRepositoryError {}

fn is_known_non_github_host(host: &str) -> bool {
    matches!(
        host,
        "gitlab.com" | "bitbucket.org" | "codeberg.org" | "gitee.com" | "sourceforge.net"
    )
}

/// Resolve one canonical GitHub repository from the selected project's git
/// remotes. Equivalent remotes are accepted once; distinct GitHub targets are
/// reported as ambiguous rather than choosing an arbitrary remote.
pub fn resolve_github_repository(
    project_repo: &Path,
) -> std::result::Result<GithubRepository, GithubRepositoryError> {
    let remotes = Command::new("git")
        .args(["remote"])
        .current_dir(project_repo)
        .output()
        .map_err(|_| GithubRepositoryError::NotGitRepository)?;
    if !remotes.status.success() {
        return Err(GithubRepositoryError::NotGitRepository);
    }
    let remote_names: Vec<String> = String::from_utf8_lossy(&remotes.stdout)
        .lines()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect();
    if remote_names.is_empty() {
        return Err(GithubRepositoryError::NoRemote);
    }

    let mut repositories = Vec::new();
    for remote in remote_names {
        let urls = Command::new("git")
            .args(["remote", "get-url", "--all", &remote])
            .current_dir(project_repo)
            .output()
            .map_err(|_| GithubRepositoryError::NoGithubRemote)?;
        if !urls.status.success() {
            continue;
        }
        for url in String::from_utf8_lossy(&urls.stdout).lines() {
            if let Some(repository) = GithubRepository::from_remote_url(url)
                && !repositories.contains(&repository)
            {
                repositories.push(repository);
            }
        }
    }

    match repositories.len() {
        0 => Err(GithubRepositoryError::NoGithubRemote),
        1 => Ok(repositories.remove(0)),
        _ => Err(GithubRepositoryError::Ambiguous { repositories }),
    }
}

/// Resolve `owner/repo` from the repository's `origin` remote.
///
/// Deliberately local: this is `git remote get-url`, not an API call. The
/// previous route to owner/repo was parsing it back out of a PR url returned by
/// `gh pr view`, which meant spending an API call *per feature* just to learn
/// something every worktree of a repo already agrees on.
///
/// Handles both remote spellings: `https://host/owner/repo(.git)` and
/// `git@host:owner/repo(.git)`.
pub fn owner_repo_from_remote(workdir: &Path) -> Option<(String, String)> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(workdir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_remote_owner_repo(String::from_utf8_lossy(&output.stdout).trim())
}

/// Pull `owner/repo` out of a git remote url, in either spelling.
fn parse_remote_owner_repo(url: &str) -> Option<(String, String)> {
    let url = url.trim();
    if url.is_empty() {
        return None;
    }
    // The presence of a scheme is what separates the two spellings. Testing
    // for `://` rather than for `:` matters: `ssh://git@host/owner/repo` has
    // both, and treating it as scp-style yields the host as the owner.
    let path = match url.split_once("://") {
        // Any scheme (https, ssh, git). Whatever precedes the first `/` is the
        // host, with userinfo already attached to it.
        Some((_scheme, rest)) => rest.split_once('/')?.1.to_string(),
        // scp-style: git@host:owner/repo(.git)
        None => url.split_once(':')?.1.to_string(),
    };

    let path = path.trim_start_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let mut segments = path.split('/').filter(|s| !s.is_empty());
    let owner = segments.next()?.to_string();
    let repo = segments.next()?.to_string();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner, repo))
}

/// Sum unresolved threads on one PR node, tolerating a missing connection.
fn unresolved_in_node(node: &serde_json::Value) -> usize {
    node["reviewThreads"]["nodes"]
        .as_array()
        .map(|threads| {
            threads
                .iter()
                .filter(|t| !t["isResolved"].as_bool().unwrap_or(false))
                .count()
        })
        .unwrap_or(0)
}

/// Parse one page of the repository-wide open-PR query.
fn parse_open_prs_page(stdout: &[u8]) -> Result<(Vec<OpenPr>, Option<String>)> {
    let v: serde_json::Value =
        serde_json::from_slice(stdout).context("Failed to parse open-PRs GraphQL JSON.")?;
    let prs = &v["data"]["repository"]["pullRequests"];

    let open = prs["nodes"]
        .as_array()
        .map(|nodes| {
            nodes
                .iter()
                .filter_map(|n| {
                    Some(OpenPr {
                        number: u32::try_from(n["number"].as_u64()?).ok()?,
                        head_ref: n["headRefName"].as_str()?.to_string(),
                        head_sha: n["headRefOid"].as_str().unwrap_or_default().to_string(),
                        unresolved_threads: unresolved_in_node(n),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let next = if prs["pageInfo"]["hasNextPage"].as_bool().unwrap_or(false) {
        prs["pageInfo"]["endCursor"].as_str().map(|s| s.to_string())
    } else {
        None
    };
    Ok((open, next))
}

/// Branches per [`GhCli::terminal_prs`] call. Each branch costs one GraphQL
/// alias plus its own `-F` variable; kept well under GitHub's per-query node
/// and variable ceilings and in line with [`OPEN_PRS_QUERY`]'s `first:50`.
const TERMINAL_PRS_CHUNK: usize = 50;

/// Build the alias-batch query for [`GhCli::terminal_prs`]. `branches.len()`
/// aliased sub-queries (`a0`, `a1`, …), each taking its branch name through
/// its own declared variable (`$b0`, `$b1`, …) — never interpolated into the
/// query text, since a branch name is not a valid GraphQL identifier and may
/// contain characters that would break the document if it were.
fn build_terminal_prs_query(branches: &[String]) -> String {
    let mut query = String::from("query($owner:String!,$repo:String!");
    for i in 0..branches.len() {
        query.push_str(&format!(",$b{i}:String!"));
    }
    query.push_str("){repository(owner:$owner,name:$repo){");
    for i in 0..branches.len() {
        query.push_str(&format!(
            "a{i}:pullRequests(headRefName:$b{i},states:[MERGED,CLOSED],first:1,\
             orderBy:{{field:UPDATED_AT,direction:DESC}}){{nodes{{number state mergedAt closedAt}}}}"
        ));
    }
    query.push_str("}}");
    query
}

/// Parse the response of [`build_terminal_prs_query`], mapping each alias
/// back to the branch it was built for. A branch with no merged-or-closed PR
/// (an empty `nodes` array, or the alias missing because a chunk answered
/// short) is simply absent from the result — that's `NoPr`, not a parse
/// failure, and the caller reads a missing entry that way.
fn parse_terminal_prs_page(
    stdout: &[u8],
    branches: &[String],
) -> Result<HashMap<String, TerminalPr>> {
    let v: serde_json::Value =
        serde_json::from_slice(stdout).context("Failed to parse terminal-PRs GraphQL JSON.")?;
    let repo = &v["data"]["repository"];

    let mut out = HashMap::new();
    for (i, branch) in branches.iter().enumerate() {
        let node = &repo[format!("a{i}")]["nodes"][0];
        let Some(state) = node["state"].as_str().and_then(TerminalPrState::from_gh) else {
            continue;
        };
        let Some(number) = node["number"].as_u64().and_then(|n| u32::try_from(n).ok()) else {
            continue;
        };
        let at = match state {
            TerminalPrState::Merged => node["mergedAt"].as_str(),
            TerminalPrState::Closed => node["closedAt"].as_str(),
        }
        .unwrap_or_default()
        .to_string();
        out.insert(branch.clone(), TerminalPr { number, state, at });
    }
    Ok(out)
}

/// Why a GraphQL call failed, distinguishing the one cause worth reacting to.
///
/// A depleted point budget is not a transient error: every subsequent call in
/// the same window fails the same way, so a caller on a timer has to stop
/// rather than retry. Everything else stays an opaque string, since the
/// callers only report it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GhGraphqlError {
    /// GitHub's hourly GraphQL point budget is exhausted.
    RateLimited,
    Failed(String),
}

impl std::fmt::Display for GhGraphqlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GhGraphqlError::RateLimited => {
                write!(f, "GitHub's hourly GraphQL rate limit is exhausted")
            }
            GhGraphqlError::Failed(detail) => write!(f, "{detail}"),
        }
    }
}

/// Whether a `gh` response reports an exhausted rate limit.
///
/// Matched on text rather than a parsed shape because GitHub reports this
/// several different ways, and an exhausted budget must be recognised through
/// all of them. Observed from a real depleted account:
///
/// ```text
/// {"errors":[{"type":"RATE_LIMIT","code":"graphql_rate_limit",
///   "message":"API rate limit already exceeded for user ID 1."}]}
/// ```
///
/// Note `RATE_LIMIT`, not the `RATE_LIMITED` the schema's enum suggests, and
/// note it arrives with HTTP 200 — so neither the exit status nor a guess at
/// the type name is enough on its own. `already exceeded` also differs from the
/// REST wording (`API rate limit exceeded`), which is why the last clause
/// matches the two words separately rather than a fixed phrase.
fn is_rate_limited(text: &str) -> bool {
    let lowered = text.to_ascii_lowercase();
    lowered.contains("graphql_rate_limit")
        || lowered.contains("rate_limited")
        || lowered.contains("\"rate_limit\"")
        || lowered.contains("secondary rate limit")
        || (lowered.contains("rate limit") && lowered.contains("exceeded"))
}

fn parse_issue_page(stdout: &[u8], page: u32, per_page: u32) -> Result<GithubIssuePage> {
    let raw: Vec<GithubIssue> =
        serde_json::from_slice(stdout).context("Failed to parse GitHub issues JSON response.")?;
    let has_next_page = raw.len() == per_page as usize;
    let issues = raw
        .into_iter()
        .filter(|issue| !issue.is_pull_request())
        .collect();
    Ok(GithubIssuePage {
        issues,
        page,
        per_page,
        has_next_page,
    })
}

#[cfg(test)]
mod tests {
    /// Every identifier in a GraphQL document, in order.
    fn graphql_identifiers(query: &str) -> Vec<String> {
        query
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .filter(|token| !token.is_empty() && !token.starts_with(|c: char| c.is_ascii_digit()))
            .map(str::to_string)
            .collect()
    }

    /// The query has to be *checked*, not just its parser.
    ///
    /// This module's tests exercised `parse_open_prs_page` against handcrafted
    /// JSON and passed while the query itself was malformed: a `\` line
    /// continuation had fused two field names into `headRefOidreviewThreads`,
    /// so GitHub rejected every call and the badge silently reported nothing.
    /// Nothing that ran offline could see it, because nothing looked at the
    /// query string.
    ///
    /// An unknown identifier is the signature of exactly that failure — two
    /// legal names concatenated make one illegal one — so the assertion is
    /// that the document contains no identifier this query has no business
    /// asking for.
    #[test]
    fn the_open_prs_query_names_every_field_as_its_own_token() {
        const EXPECTED: &[&str] = &[
            // operation + variables
            "query",
            "owner",
            "String",
            "repo",
            "cursor",
            // selection
            "repository",
            "name",
            "pullRequests",
            "states",
            "OPEN",
            "first",
            "after",
            "pageInfo",
            "hasNextPage",
            "endCursor",
            "nodes",
            "number",
            "headRefName",
            "headRefOid",
            "reviewThreads",
            "isResolved",
        ];

        let identifiers = graphql_identifiers(OPEN_PRS_QUERY);

        for field in [
            "number",
            "headRefName",
            "headRefOid",
            "reviewThreads",
            "isResolved",
            "hasNextPage",
            "endCursor",
        ] {
            assert!(
                identifiers.iter().any(|token| token == field),
                "`{field}` is missing or fused with a neighbour; query was:\n{OPEN_PRS_QUERY}"
            );
        }

        for token in &identifiers {
            assert!(
                EXPECTED.contains(&token.as_str()),
                "unexpected identifier `{token}` — two field names have probably \
                 run together; query was:\n{OPEN_PRS_QUERY}"
            );
        }
    }

    /// One query answers a whole repository: each PR carries the branch it is
    /// from and its own unresolved count, so a feature is matched locally
    /// instead of costing an API call of its own.
    #[test]
    fn the_open_prs_parser_reads_branch_sha_and_unresolved_count_per_pr() {
        let json = br#"{"data":{"repository":{"pullRequests":{
            "pageInfo":{"hasNextPage":false,"endCursor":null},
            "nodes":[
              {"number":546,"headRefName":"todo-plan","headRefOid":"abc1234",
               "reviewThreads":{"nodes":[{"isResolved":false},{"isResolved":true}]}},
              {"number":343,"headRefName":"fixture","headRefOid":"def5678",
               "reviewThreads":{"nodes":[]}}]}}}}"#;
        let (prs, next) = parse_open_prs_page(json).unwrap();
        assert_eq!(next, None);
        assert_eq!(prs.len(), 2);
        assert_eq!(prs[0].number, 546);
        assert_eq!(prs[0].head_ref, "todo-plan");
        assert_eq!(prs[0].head_sha, "abc1234");
        assert_eq!(prs[0].unresolved_threads, 1);
        assert_eq!(prs[1].unresolved_threads, 0);
    }

    /// A thread with no `isResolved` counts as unresolved, matching
    /// `parse_review_threads_page`'s `unwrap_or(false)`. Under-reporting work
    /// still to do is the worse failure.
    #[test]
    fn a_thread_missing_its_resolution_counts_as_unresolved() {
        let json = br#"{"data":{"repository":{"pullRequests":{"pageInfo":{"hasNextPage":false},
            "nodes":[{"number":1,"headRefName":"b","headRefOid":"s",
              "reviewThreads":{"nodes":[{},{"isResolved":true}]}}]}}}}"#;
        let (prs, _) = parse_open_prs_page(json).unwrap();
        assert_eq!(prs[0].unresolved_threads, 1);
    }

    /// Pagination carries the cursor, so a repository with more than 50 open
    /// PRs is read in full rather than truncated at the first page.
    #[test]
    fn the_open_prs_parser_reports_the_next_cursor() {
        let json = br#"{"data":{"repository":{"pullRequests":{
            "pageInfo":{"hasNextPage":true,"endCursor":"Y3Vyc29yOjUw"},
            "nodes":[]}}}}"#;
        let (prs, next) = parse_open_prs_page(json).unwrap();
        assert!(prs.is_empty());
        assert_eq!(next.as_deref(), Some("Y3Vyc29yOjUw"));
    }

    /// An empty or shape-drifted response is "no open PRs", not an error: the
    /// sweep degrades to clearing badges rather than failing the repository.
    #[test]
    fn an_unexpected_shape_reads_as_no_open_prs() {
        let (prs, next) = parse_open_prs_page(b"{}").unwrap();
        assert!(prs.is_empty());
        assert_eq!(next, None);
    }

    /// A PR missing `headRefName` cannot be matched to a feature branch, so it
    /// is dropped rather than matched against every feature with an empty
    /// branch name.
    #[test]
    fn a_pr_without_a_head_branch_is_skipped() {
        let json = br#"{"data":{"repository":{"pullRequests":{"pageInfo":{"hasNextPage":false},
            "nodes":[{"number":7,"headRefOid":"s","reviewThreads":{"nodes":[]}}]}}}}"#;
        let (prs, _) = parse_open_prs_page(json).unwrap();
        assert!(prs.is_empty());
    }

    /// Same discipline as `the_open_prs_query_names_every_field_as_its_own_token`:
    /// the query is built by hand (aliases, variable names), so it has to be
    /// checked as text, not trusted because its parser passes on handcrafted
    /// JSON that a broken query would never actually produce.
    #[test]
    fn the_terminal_prs_query_names_every_field_as_its_own_token_and_one_alias_per_branch() {
        let branches = vec!["feat-a".to_string(), "feat-b".to_string()];
        let query = build_terminal_prs_query(&branches);

        const EXPECTED: &[&str] = &[
            "query",
            "owner",
            "String",
            "repo",
            "b0",
            "b1",
            "repository",
            "name",
            "a0",
            "a1",
            "pullRequests",
            "headRefName",
            "states",
            "MERGED",
            "CLOSED",
            "first",
            "orderBy",
            "field",
            "UPDATED_AT",
            "direction",
            "DESC",
            "nodes",
            "number",
            "state",
            "mergedAt",
            "closedAt",
        ];

        let identifiers = graphql_identifiers(&query);
        for field in [
            "a0", "a1", "b0", "b1", "number", "state", "mergedAt", "closedAt",
        ] {
            assert!(
                identifiers.iter().any(|token| token == field),
                "`{field}` is missing or fused with a neighbour; query was:\n{query}"
            );
        }
        for token in &identifiers {
            assert!(
                EXPECTED.contains(&token.as_str()),
                "unexpected identifier `{token}` — two field names have probably \
                 run together; query was:\n{query}"
            );
        }
    }

    /// One branch merged, one closed-without-merge, one never had a terminal
    /// PR at all (an empty `nodes` array) — the three outcomes the sweep has
    /// to tell apart.
    #[test]
    fn the_terminal_prs_parser_maps_each_alias_back_to_its_branch() {
        let branches = vec![
            "feat-merged".to_string(),
            "feat-closed".to_string(),
            "feat-none".to_string(),
        ];
        let json = br#"{"data":{"repository":{
            "a0":{"nodes":[{"number":11,"state":"MERGED","mergedAt":"2026-01-01T00:00:00Z","closedAt":"2026-01-01T00:00:00Z"}]},
            "a1":{"nodes":[{"number":12,"state":"CLOSED","mergedAt":null,"closedAt":"2026-02-02T00:00:00Z"}]},
            "a2":{"nodes":[]}
        }}}"#;

        let result = parse_terminal_prs_page(json, &branches).unwrap();

        let merged = result.get("feat-merged").unwrap();
        assert_eq!(merged.number, 11);
        assert_eq!(merged.state, TerminalPrState::Merged);
        assert_eq!(merged.at, "2026-01-01T00:00:00Z");

        let closed = result.get("feat-closed").unwrap();
        assert_eq!(closed.number, 12);
        assert_eq!(closed.state, TerminalPrState::Closed);
        assert_eq!(closed.at, "2026-02-02T00:00:00Z");

        assert!(!result.contains_key("feat-none"));
    }

    /// An open PR (or any other non-terminal state) must never surface here —
    /// `open_prs` already owns that state, and double-reporting it would let
    /// the two sweeps disagree about which badge to show.
    #[test]
    fn an_open_pr_returned_by_the_terminal_query_is_not_reported() {
        let branches = vec!["feat-open".to_string()];
        let json = br#"{"data":{"repository":{
            "a0":{"nodes":[{"number":9,"state":"OPEN","mergedAt":null,"closedAt":null}]}
        }}}"#;
        let result = parse_terminal_prs_page(json, &branches).unwrap();
        assert!(result.is_empty());
    }

    /// An unexpected/empty shape reads as "nothing terminal found" rather than
    /// an error, matching `an_unexpected_shape_reads_as_no_open_prs`.
    #[test]
    fn an_unexpected_terminal_shape_reads_as_no_terminal_prs() {
        let branches = vec!["feat-a".to_string()];
        let result = parse_terminal_prs_page(b"{}", &branches).unwrap();
        assert!(result.is_empty());
    }

    /// Owner/repo comes from the git remote, in either spelling, so learning it
    /// costs no API call at all.
    #[test]
    fn owner_repo_is_read_from_either_remote_spelling() {
        for url in [
            "https://github.com/example-org/ai-workstation.git",
            "https://github.com/example-org/ai-workstation",
            "git@github.com:example-org/ai-workstation.git",
            "ssh://git@github.com/example-org/ai-workstation.git",
            "git@git.example.com:example-org/ai-workstation",
        ] {
            assert_eq!(
                parse_remote_owner_repo(url),
                Some(("example-org".into(), "ai-workstation".into())),
                "failed on {url}"
            );
        }
    }

    #[test]
    fn canonical_github_identity_normalizes_equivalent_remote_spellings() {
        let https =
            GithubRepository::from_remote_url("https://GitHub.com/Example-Org/AI-Workstation.git")
                .unwrap();
        let ssh =
            GithubRepository::from_remote_url("git@github.com:Example-Org/AI-Workstation").unwrap();
        assert_eq!(https, ssh);
        assert_eq!(https.canonical(), "github.com/example-org/ai-workstation");

        let enterprise = GithubRepository::from_remote_url(
            "ssh://git@github.acme.example/platform/ai-workstation.git",
        )
        .unwrap();
        assert_eq!(
            enterprise.canonical(),
            "github.acme.example/platform/ai-workstation"
        );
    }

    #[test]
    fn canonical_identity_rejects_known_non_github_hosts() {
        assert!(GithubRepository::from_remote_url("git@gitlab.com:team/project.git").is_none());
        assert!(GithubRepository::from_remote_url("https://bitbucket.org/team/project").is_none());
    }

    #[test]
    fn issue_page_parsing_keeps_issue_metadata_filters_prs_and_marks_pages() {
        let json = r#"[
            {"number":7,"title":"Unicode 🚀","body":"line one\nline two",
             "html_url":"https://github.com/acme/widget/issues/7",
             "labels":[{"name":"bug"},{"name":"needs-help"}],
             "updated_at":"2026-09-15T12:00:00Z"},
            {"number":8,"title":"A pull request","body":null,
             "html_url":"https://github.com/acme/widget/pull/8",
             "labels":[],"updated_at":"2026-09-15T11:00:00Z",
             "pull_request":{"url":"https://api.github.com/repos/acme/widget/pulls/8"}}
        ]"#;
        let page = parse_issue_page(json.as_bytes(), 2, 2).unwrap();
        assert_eq!(page.page, 2);
        assert_eq!(page.per_page, 2);
        assert!(page.has_next_page);
        assert_eq!(page.issues.len(), 1);
        assert_eq!(page.issues[0].number, 7);
        assert_eq!(page.issues[0].body.as_deref(), Some("line one\nline two"));
        assert_eq!(page.issues[0].labels[1].name, "needs-help");
    }

    #[test]
    fn malformed_issue_page_is_a_recoverable_parse_error() {
        let error = parse_issue_page(br#"[{\"number\":1}"#, 1, 50)
            .unwrap_err()
            .to_string();
        assert!(error.contains("GitHub issues JSON"));
    }

    #[test]
    fn repository_resolution_reports_unavailable_and_ambiguous_states() {
        let no_git = tempfile::tempdir().unwrap();
        assert_eq!(
            resolve_github_repository(no_git.path()),
            Err(GithubRepositoryError::NotGitRepository)
        );

        let no_remote = tempfile::tempdir().unwrap();
        assert!(
            Command::new("git")
                .args(["init", "--quiet"])
                .current_dir(no_remote.path())
                .status()
                .unwrap()
                .success()
        );
        assert_eq!(
            resolve_github_repository(no_remote.path()),
            Err(GithubRepositoryError::NoRemote)
        );

        let ambiguous = tempfile::tempdir().unwrap();
        assert!(
            Command::new("git")
                .args(["init", "--quiet"])
                .current_dir(ambiguous.path())
                .status()
                .unwrap()
                .success()
        );
        for (name, url) in [
            ("origin", "git@github.com:one/project.git"),
            ("upstream", "https://github.com/two/project.git"),
        ] {
            assert!(
                Command::new("git")
                    .args(["remote", "add", name, url])
                    .current_dir(ambiguous.path())
                    .status()
                    .unwrap()
                    .success()
            );
        }
        assert!(matches!(
            resolve_github_repository(ambiguous.path()),
            Err(GithubRepositoryError::Ambiguous { .. })
        ));
    }

    /// A remote that names no repository yields nothing, so the sweep reports
    /// "no PR" instead of querying a repo it cannot name.
    #[test]
    fn an_unusable_remote_yields_no_owner_repo() {
        for url in ["", "   ", "https://github.com/onlyowner", "not a url"] {
            assert_eq!(parse_remote_owner_repo(url), None, "failed on {url:?}");
        }
    }

    /// GraphQL reports a depleted budget with HTTP 200 and an `errors[].type`,
    /// so the exit status alone never catches it. REST's phrasing and the
    /// secondary limit have to match too, because `gh pr view` spends the same
    /// budget and fails its own way.
    #[test]
    fn rate_limit_detection_covers_how_gh_actually_reports_it() {
        // Captured verbatim from a real depleted account. Note `RATE_LIMIT`
        // (not the `RATE_LIMITED` the enum name suggests) and "already
        // exceeded" (not REST's "exceeded") — this case is why the matcher
        // does not key on one phrase.
        assert!(is_rate_limited(
            r#"{"errors":[{"type":"RATE_LIMIT","code":"graphql_rate_limit","message":"API rate limit already exceeded for user ID 72774132."}]}"#
        ));
        assert!(is_rate_limited(
            r#"{"errors":[{"type":"RATE_LIMITED","message":"API rate limit exceeded"}]}"#
        ));
        assert!(is_rate_limited("API rate limit exceeded for user ID 1."));
        assert!(is_rate_limited("You have exceeded a secondary rate limit."));
        assert!(is_rate_limited("HTTP 403: rate limit exceeded"));
    }

    /// An ordinary failure must not be mistaken for a rate limit: that would
    /// pause PR badges for fifteen minutes over a typo'd repo name.
    #[test]
    fn ordinary_failures_are_not_read_as_rate_limits() {
        assert!(!is_rate_limited("Could not resolve to a Repository."));
        assert!(!is_rate_limited("no pull requests found for branch"));
        assert!(!is_rate_limited(""));
        // "rate limit" alone, without exhaustion, is documentation not failure.
        assert!(!is_rate_limited(
            "See the rate limit documentation for details."
        ));
    }
    use super::*;

    #[test]
    fn detects_missing_write_scope() {
        assert!(is_missing_write_scope(
            "gh: HTTP 403: Resource not accessible by integration"
        ));
        assert!(is_missing_write_scope(
            "403 your token has not been granted the required scopes"
        ));
        assert!(!is_missing_write_scope("HTTP 422: Validation Failed"));
        assert!(!is_missing_write_scope("could not resolve host github.com"));
    }
}
