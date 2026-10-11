//! `aiw grid <feature|project>` — a feature's agents (or the first agent of
//! every feature in a project) as live, tiled tmux panes in the current
//! terminal. DESIGN.md §4.2.
//!
//! How it works, and why it is safe for the user's live sessions:
//!
//! * Each agent window is shown through a *grouped view session* on AI Workstation's own
//!   tmux socket (`new-session -t <feature session> -s aiw-view-…`). A group
//!   shares windows but each member has its own current window, so selecting
//!   the agent's window in the view never moves AIW's own session, and
//!   `destroy-unattached on` removes the view the moment its client goes away.
//! * The tiles live in a separate, throw-away *grid server* (own socket
//!   `aiw-grid-<pid>.sock` next to AI Workstation's, `-f /dev/null`): status line off, no prefix key, mouse on (click focuses a
//!   pane), pane borders titled with the agent's name. Each grid pane runs one
//!   `env -u TMUX tmux -S <AIW socket> new-session -t …` client.
//! * The grid session is `destroy-unattached` too, so closing the terminal
//!   tears down the grid server, which hangs up the view clients, which
//!   destroys the view sessions. `aiw grid` ignores the hang-up itself, so it
//!   still cleans up explicitly (and removes the grid socket) when the attach
//!   returns.

use std::ffi::OsString;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::project::{ProjectStatus, ProjectStore, SessionKind};

/// Name of the session inside the grid server.
pub const GRID_SESSION: &str = "aiw-grid";
/// Root-table key that closes the grid (there is no prefix key).
pub const GRID_QUIT_KEY: &str = "M-q";

/// Parsed `aiw grid` arguments.
#[derive(Debug, Clone, Default)]
pub struct GridOptions {
    /// Feature name (or `project/feature`) or project name.
    pub name: String,
    /// Print what would be shown and exit.
    pub list: bool,
    /// Include every tmux-backed session (terminals, editors, custom), not
    /// only agents.
    pub all: bool,
    /// Treat `name` as a project name even if a feature has the same name.
    pub project: bool,
}

/// A session of the store that could be shown in the grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub project: String,
    pub feature: String,
    pub label: String,
    pub tmux_session: String,
    pub window: String,
}

impl Candidate {
    pub fn title(&self) -> String {
        format!("{} › {} › {}", self.project, self.feature, self.label)
    }
}

/// What a name resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Feature { project: String, feature: String },
    Project { project: String },
}

impl Scope {
    pub fn describe(&self) -> String {
        match self {
            Scope::Feature { project, feature } => format!("feature {project}/{feature}"),
            Scope::Project { project } => format!("project {project}"),
        }
    }
}

/// The candidates a name resolved to, before checking what is running.
///
/// For a feature every live candidate is shown; for a project, the first live
/// candidate of each feature (`groups` holds one group per feature).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub scope: Scope,
    pub groups: Vec<Vec<Candidate>>,
}

/// One live window that will become one grid pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridTarget {
    pub candidate: Candidate,
    /// Index of the window in the feature's tmux session (shared by every
    /// session in its group).
    pub window_index: u32,
}

/// A live tmux window: `(session, window name, window index)`.
pub type LiveWindow = (String, String, u32);

fn wanted(kind: &SessionKind, all: bool) -> bool {
    if all {
        kind.is_tmux_backed()
    } else {
        kind.is_agent_harness()
    }
}

fn feature_candidates(
    project: &crate::project::Project,
    feature: &crate::project::Feature,
    all: bool,
) -> Vec<Candidate> {
    let feature_name = feature
        .nickname
        .clone()
        .unwrap_or_else(|| feature.name.clone());
    let mut sessions: Vec<&crate::project::FeatureSession> = feature
        .sessions
        .iter()
        .filter(|s| wanted(&s.kind, all) && !s.stopped)
        .collect();
    // Agents first, keeping store order within each group.
    sessions.sort_by_key(|s| !s.kind.is_agent_harness());
    sessions
        .into_iter()
        .map(|s| Candidate {
            project: project.name.clone(),
            feature: feature_name.clone(),
            label: s.label.clone(),
            tmux_session: feature.tmux_session.clone(),
            window: s.tmux_window.clone(),
        })
        .collect()
}

fn feature_matches(feature: &crate::project::Feature, query: &str, exact: bool) -> bool {
    let eq = |value: &str| {
        if exact {
            value == query
        } else {
            value.eq_ignore_ascii_case(query)
        }
    };
    eq(&feature.name)
        || eq(&feature.branch)
        || eq(&feature.tmux_session)
        || feature.nickname.as_deref().is_some_and(eq)
}

/// Resolve `query` against the store.
///
/// Order: `project/feature`, then an exact feature match (an ambiguous one is
/// an error listing the choices), then an exact project match, then the same
/// two case-insensitively. `force_project` skips the feature lookups.
pub fn select(
    store: &ProjectStore,
    query: &str,
    force_project: bool,
    all: bool,
) -> Result<Selection, String> {
    let query = query.trim();
    if query.is_empty() {
        return Err("give a feature or project name".to_string());
    }

    for exact in [true, false] {
        let name_eq = |value: &str| {
            if exact {
                value == query
            } else {
                value.eq_ignore_ascii_case(query)
            }
        };

        if !force_project {
            // `project/feature` — branch names may contain `/` too, so only
            // split where the prefix is a real project.
            for (idx, _) in query.match_indices('/') {
                let (project_part, feature_part) = (&query[..idx], &query[idx + 1..]);
                for project in &store.projects {
                    let project_hit = if exact {
                        project.name == project_part
                    } else {
                        project.name.eq_ignore_ascii_case(project_part)
                    };
                    if !project_hit {
                        continue;
                    }
                    if let Some(feature) = project
                        .features
                        .iter()
                        .find(|f| feature_matches(f, feature_part, exact))
                    {
                        return Ok(Selection {
                            scope: Scope::Feature {
                                project: project.name.clone(),
                                feature: feature.name.clone(),
                            },
                            groups: vec![feature_candidates(project, feature, all)],
                        });
                    }
                }
            }

            let hits: Vec<(&crate::project::Project, &crate::project::Feature)> = store
                .projects
                .iter()
                .flat_map(|p| p.features.iter().map(move |f| (p, f)))
                .filter(|(_, f)| feature_matches(f, query, exact))
                .collect();
            match hits.as_slice() {
                [(project, feature)] => {
                    return Ok(Selection {
                        scope: Scope::Feature {
                            project: project.name.clone(),
                            feature: feature.name.clone(),
                        },
                        groups: vec![feature_candidates(project, feature, all)],
                    });
                }
                [] => {}
                many => {
                    let choices: Vec<String> = many
                        .iter()
                        .map(|(p, f)| format!("  aiw grid {}/{}", p.name, f.name))
                        .collect();
                    return Err(format!(
                        "\"{query}\" matches features in several projects; pick one:\n{}",
                        choices.join("\n")
                    ));
                }
            }
        }

        if let Some(project) = store.projects.iter().find(|p| name_eq(&p.name)) {
            return Ok(Selection {
                scope: Scope::Project {
                    project: project.name.clone(),
                },
                groups: project
                    .features
                    .iter()
                    .filter(|f| f.status != ProjectStatus::Stopped)
                    .map(|f| feature_candidates(project, f, all))
                    .filter(|group| !group.is_empty())
                    .collect(),
            });
        }
    }

    let mut known: Vec<String> = Vec::new();
    for project in &store.projects {
        known.push(format!("  {} (project)", project.name));
        for feature in &project.features {
            known.push(format!("  {}/{}", project.name, feature.name));
        }
    }
    if known.is_empty() {
        Err(format!(
            "no feature or project named \"{query}\" (AI Workstation has no projects yet)"
        ))
    } else {
        known.truncate(40);
        Err(format!(
            "no feature or project named \"{query}\". Known names:\n{}",
            known.join("\n")
        ))
    }
}

/// Keep the candidates whose window is running: every one for a feature, the
/// first one per feature for a project.
pub fn live_targets(selection: &Selection, live: &[LiveWindow]) -> Vec<GridTarget> {
    let take_all = matches!(selection.scope, Scope::Feature { .. });
    let mut targets = Vec::new();
    for group in &selection.groups {
        let mut live_in_group = group.iter().filter_map(|candidate| {
            live.iter()
                .find(|(session, window, _)| {
                    *session == candidate.tmux_session && *window == candidate.window
                })
                .map(|(_, _, index)| GridTarget {
                    candidate: candidate.clone(),
                    window_index: *index,
                })
        });
        if take_all {
            targets.extend(live_in_group);
        } else if let Some(first) = live_in_group.next() {
            targets.push(first);
        }
    }
    targets
}

/// Parse `list-windows -a -F "#{session_name}\t#{window_name}\t#{window_index}"`.
pub fn parse_live_windows(output: &str) -> Vec<LiveWindow> {
    output
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let session = parts.next()?;
            let window = parts.next()?;
            let index = parts.next()?.trim().parse().ok()?;
            Some((session.to_string(), window.to_string(), index))
        })
        .collect()
}

/// POSIX shell single-quoting.
pub fn shell_quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:@%+,".contains(c))
    {
        return value.to_string();
    }
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// The tmux argv (without the binary) a grid pane runs to show one agent
/// window through a grouped view session on AI Workstation's socket.
pub fn view_client_args(
    aiw_socket: Option<&Path>,
    target: &GridTarget,
    view_session: &str,
) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(socket) = aiw_socket {
        args.push("-S".to_string());
        args.push(socket.to_string_lossy().into_owned());
    }
    let window = format!(":{}", target.window_index);
    args.extend(
        [
            "new-session",
            "-t",
            &format!("={}", target.candidate.tmux_session),
            "-s",
            view_session,
            ";",
            "set-option",
            "destroy-unattached",
            "on",
            ";",
            "set-option",
            "status",
            "off",
            ";",
            // The window follows whichever client used it last, so the tile
            // gets a window its own size instead of AIW's last manual size.
            "set-window-option",
            "-t",
            &window,
            "window-size",
            "latest",
            ";",
            "select-window",
            "-t",
            &window,
        ]
        .map(String::from),
    );
    args
}

/// The shell command a grid pane runs (the grid server runs pane commands
/// through `default-shell -c`).
pub fn view_client_command(
    tmux_bin: &str,
    aiw_socket: Option<&Path>,
    target: &GridTarget,
    view_session: &str,
) -> String {
    let mut parts = vec![
        "exec".to_string(),
        "env".to_string(),
        "-u".to_string(),
        "TMUX".to_string(),
        shell_quote(tmux_bin),
    ];
    parts.extend(
        view_client_args(aiw_socket, target, view_session)
            .iter()
            .map(|arg| shell_quote(arg)),
    );
    parts.join(" ")
}

/// Everything needed to build and attach the grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridPlan {
    /// Socket of the throw-away grid server.
    pub grid_socket: PathBuf,
    /// One chained tmux invocation (arguments after `-S <grid socket> -f
    /// /dev/null`) that starts the grid server and builds every pane.
    pub build: Vec<String>,
    /// Arguments (after `-S <grid socket>`) that attach this terminal.
    pub attach: Vec<String>,
    /// View sessions created on AI Workstation's socket, for cleanup.
    pub view_sessions: Vec<String>,
}

/// Build the grid: server options, one pane per target, tiled layout.
///
/// Pane ids of a fresh tmux server are allocated `%0, %1, …` in creation
/// order, which lets the whole grid be built in one atomic invocation.
pub fn build_plan(
    targets: &[GridTarget],
    tmux_bin: &str,
    aiw_socket: Option<&Path>,
    grid_socket: &Path,
    instance: &str,
    size: (u16, u16),
    truecolor: bool,
) -> GridPlan {
    let window = format!("{GRID_SESSION}:grid");
    let mut build: Vec<String> = Vec::new();
    let mut push = |args: &[&str]| {
        if !build.is_empty() {
            build.push(";".to_string());
        }
        build.extend(args.iter().map(|arg| arg.to_string()));
    };

    push(&["start-server"]);
    for (option, value) in [
        ("status", "off"),
        ("prefix", "None"),
        ("prefix2", "None"),
        ("mouse", "on"),
        ("escape-time", "0"),
        ("default-terminal", "tmux-256color"),
        ("pane-border-status", "top"),
        (
            "pane-border-format",
            " #{?pane_active,#[bold],}#{@aiw_title}#[default] ",
        ),
        ("pane-active-border-style", "fg=yellow,bold"),
        ("pane-border-style", "fg=brightblack"),
        ("set-titles", "on"),
        ("set-titles-string", "aiw grid"),
        ("remain-on-exit", "off"),
    ] {
        push(&["set-option", "-g", option, value]);
    }
    if truecolor {
        push(&["set-option", "-ga", "terminal-features", ",*:RGB"]);
    }
    push(&["bind-key", "-n", GRID_QUIT_KEY, "kill-server"]);

    let cols = size.0.max(20).to_string();
    let rows = size.1.max(5).to_string();
    let mut view_sessions = Vec::new();
    for (i, target) in targets.iter().enumerate() {
        let view = format!("aiw-view-{instance}-{i}");
        let command = view_client_command(tmux_bin, aiw_socket, target, &view);
        view_sessions.push(view);
        if i == 0 {
            push(&[
                "new-session",
                "-d",
                "-s",
                GRID_SESSION,
                "-n",
                "grid",
                "-x",
                &cols,
                "-y",
                &rows,
                &command,
            ]);
        } else {
            push(&["split-window", "-t", &window, &command]);
            // Re-tile after every split so the next one has room.
            push(&["select-layout", "-t", &window, "tiled"]);
        }
        let pane = format!("%{i}");
        push(&[
            "set-option",
            "-p",
            "-t",
            &pane,
            "@aiw_title",
            &target.candidate.title(),
        ]);
    }
    push(&["select-layout", "-t", &window, "tiled"]);
    push(&["select-pane", "-t", "%0"]);

    let attach = [
        "attach-session",
        "-t",
        GRID_SESSION,
        ";",
        // Set only once a client is attached: closing the terminal then
        // destroys the grid session, which ends the grid server.
        "set-option",
        "-t",
        GRID_SESSION,
        "destroy-unattached",
        "on",
    ]
    .map(String::from)
    .to_vec();

    GridPlan {
        grid_socket: grid_socket.to_path_buf(),
        build,
        attach,
        view_sessions,
    }
}

fn tmux_on(bin: &OsString, socket: Option<&Path>) -> Command {
    let mut command = Command::new(bin);
    if let Some(socket) = socket {
        command.arg("-S").arg(socket);
    }
    command
}

fn list_live_windows(bin: &OsString, socket: Option<&Path>) -> Vec<LiveWindow> {
    if socket.is_some_and(|s| !s.exists()) {
        return Vec::new();
    }
    tmux_on(bin, socket)
        .args([
            "list-windows",
            "-a",
            "-F",
            "#{session_name}\t#{window_name}\t#{window_index}",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| parse_live_windows(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

/// Sockets to look for the agents on: the one AIW would use from this
/// environment first, then AIW's default private and dedicated sockets (so
/// `aiw grid` run inside some unrelated tmux still finds them).
fn candidate_sockets() -> Vec<Option<PathBuf>> {
    let (_, runtime_socket) = crate::tmux::TmuxManager::runtime_binary_and_socket();
    let mut sockets = vec![runtime_socket];
    for path in crate::tmux::TmuxManager::default_socket_paths() {
        if !sockets.iter().any(|s| s.as_deref() == Some(path.as_path())) {
            sockets.push(Some(path));
        }
    }
    sockets
}

fn load_store() -> Result<ProjectStore> {
    let db_path = crate::project::db_path();
    let db = crate::db::AiwDb::open_read_only(&db_path).with_context(|| {
        format!(
            "could not open the AI Workstation database at {}",
            db_path.display()
        )
    })?;
    db.load_store().context(
        "could not read projects from the AI Workstation database (run `aiw` once to migrate it)",
    )
}

/// Entry point of `aiw grid`.
pub fn run(options: GridOptions) -> Result<()> {
    let store = load_store()?;
    let selection = match select(&store, &options.name, options.project, options.all) {
        Ok(selection) => selection,
        Err(message) => bail!("{message}"),
    };

    let (tmux_bin, _) = crate::tmux::TmuxManager::runtime_binary_and_socket();
    let mut chosen: Option<(Option<PathBuf>, Vec<GridTarget>)> = None;
    for socket in candidate_sockets() {
        let live = list_live_windows(&tmux_bin, socket.as_deref());
        let targets = live_targets(&selection, &live);
        if !targets.is_empty() {
            chosen = Some((socket, targets));
            break;
        }
    }

    let what = selection.scope.describe();
    let Some((aiw_socket, targets)) = chosen else {
        let hint = if options.all {
            ""
        } else {
            " (only agent sessions are shown; --all includes terminals and other sessions)"
        };
        bail!(
            "{what} has no running sessions to show{hint}. Start it in AI Workstation first (`c` on the feature)."
        );
    };

    let socket_label = aiw_socket
        .as_deref()
        .map(|s| s.display().to_string())
        .unwrap_or_else(|| "default tmux server".to_string());
    if options.list {
        println!(
            "aiw grid: {what} — {} pane(s) from {socket_label}",
            targets.len()
        );
        for target in &targets {
            println!(
                "  {:<50} {}:{} (window {})",
                target.candidate.title(),
                target.candidate.tmux_session,
                target.candidate.window,
                target.window_index
            );
        }
        return Ok(());
    }

    if !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
        bail!("aiw grid needs an interactive terminal (use --list to print what it would show)");
    }

    let size = crossterm::terminal::size().unwrap_or((200, 50));
    let truecolor = std::env::var("COLORTERM")
        .is_ok_and(|value| matches!(value.as_str(), "truecolor" | "24bit"));
    let instance = std::process::id().to_string();
    let grid_socket = grid_socket_path(&instance);
    if let Some(dir) = grid_socket.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let plan = build_plan(
        &targets,
        &tmux_bin.to_string_lossy(),
        aiw_socket.as_deref(),
        &grid_socket,
        &instance,
        size,
        truecolor,
    );

    let grid = |args: &[String]| {
        let mut command = Command::new(&tmux_bin);
        command.arg("-S").arg(&plan.grid_socket);
        command.args(args);
        command.env_remove("TMUX");
        command
    };
    let cleanup = || {
        let _ = grid(&["kill-server".to_string()]).output();
        for view in &plan.view_sessions {
            let _ = tmux_on(&tmux_bin, aiw_socket.as_deref())
                .args(["kill-session", "-t", &format!("={view}")])
                .output();
        }
        let _ = std::fs::remove_file(&plan.grid_socket);
    };

    let mut build = grid(&["-f".to_string(), "/dev/null".to_string()]);
    build.args(&plan.build);
    let output = build
        .output()
        .context("could not start tmux for the grid")?;
    if !output.status.success() {
        cleanup();
        bail!(
            "could not build the grid: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    eprintln!(
        "aiw grid: {what} — {} pane(s). Click a pane to focus it; Alt+q closes the grid.",
        targets.len()
    );
    // Closing the terminal hangs up this process too. Ignore that so the
    // cleanup below still runs once the tmux client (which handles its own
    // hang-up by detaching) returns. Nothing is printed after this point:
    // the terminal may be gone.
    // SAFETY: setting a signal disposition to SIG_IGN has no preconditions.
    unsafe {
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
    }
    let status = grid(&plan.attach).status();
    cleanup();
    status.context("could not attach to the grid")?;
    Ok(())
}

/// Socket of the grid server: next to AI Workstation's own tmux socket (falling back to
/// the temp dir when that path would be too long for a unix socket).
fn grid_socket_path(instance: &str) -> PathBuf {
    let name = format!("aiw-grid-{instance}.sock");
    let [private, _] = crate::tmux::TmuxManager::default_socket_paths();
    let preferred = private
        .parent()
        .map(|dir| dir.join(&name))
        .unwrap_or_else(|| std::env::temp_dir().join(&name));
    if preferred.as_os_str().len() < 100 {
        preferred
    } else {
        std::env::temp_dir().join(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{AgentKind, Feature, Project, ProjectStore, VibeMode};
    use std::collections::HashMap;

    fn feature(name: &str, kinds: &[SessionKind]) -> Feature {
        let mut feature = Feature::new(
            name.to_string(),
            name.to_string(),
            PathBuf::from("/tmp"),
            false,
            VibeMode::Vibeless,
            false,
            false,
            AgentKind::Claude,
            false,
        );
        feature.status = ProjectStatus::Active;
        for kind in kinds {
            feature.add_session(kind.clone());
        }
        feature
    }

    fn store() -> ProjectStore {
        let mut rs = Project::new(
            "RS".into(),
            PathBuf::from("/tmp/rs"),
            true,
            AgentKind::Claude,
        );
        rs.features.push(feature(
            "hx-12",
            &[
                SessionKind::Claude,
                SessionKind::Terminal,
                SessionKind::Codex,
            ],
        ));
        rs.features.push(feature("hx-13", &[SessionKind::Codex]));
        let mut ba = Project::new(
            "BA".into(),
            PathBuf::from("/tmp/ba"),
            true,
            AgentKind::Claude,
        );
        ba.features.push(feature("ba-7", &[SessionKind::Claude]));
        ba.features.push(feature("hx-13", &[SessionKind::Claude]));
        ProjectStore {
            version: 5,
            projects: vec![rs, ba],
            session_bookmarks: vec![],
            available_harnesses: vec![],
            prompt_templates: Vec::new(),
            extra: HashMap::new(),
        }
    }

    fn live_for(selection: &Selection) -> Vec<LiveWindow> {
        selection
            .groups
            .iter()
            .flatten()
            .enumerate()
            .map(|(i, c)| (c.tmux_session.clone(), c.window.clone(), i as u32))
            .collect()
    }

    #[test]
    fn feature_name_selects_its_agent_windows_only() {
        let selection = select(&store(), "hx-12", false, false).unwrap();
        assert_eq!(
            selection.scope,
            Scope::Feature {
                project: "RS".into(),
                feature: "hx-12".into()
            }
        );
        let labels: Vec<_> = selection.groups[0]
            .iter()
            .map(|c| c.label.clone())
            .collect();
        assert_eq!(labels.len(), 2, "terminal is not an agent: {labels:?}");
        assert!(
            selection.groups[0]
                .iter()
                .all(|c| c.tmux_session == "aiw-hx-12")
        );
    }

    #[test]
    fn all_flag_includes_terminals_after_agents() {
        let selection = select(&store(), "hx-12", false, true).unwrap();
        assert_eq!(selection.groups[0].len(), 3);
        assert_eq!(selection.groups[0][2].window, "terminal");
    }

    #[test]
    fn ambiguous_feature_lists_qualified_names() {
        let err = select(&store(), "hx-13", false, false).unwrap_err();
        assert!(err.contains("aiw grid RS/hx-13"), "{err}");
        assert!(err.contains("aiw grid BA/hx-13"), "{err}");
        let selection = select(&store(), "BA/hx-13", false, false).unwrap();
        assert_eq!(
            selection.scope,
            Scope::Feature {
                project: "BA".into(),
                feature: "hx-13".into()
            }
        );
    }

    #[test]
    fn project_name_takes_first_live_agent_per_feature() {
        let selection = select(&store(), "rs", false, false).unwrap();
        assert_eq!(
            selection.scope,
            Scope::Project {
                project: "RS".into()
            }
        );
        let live = live_for(&selection);
        let targets = live_targets(&selection, &live);
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].candidate.feature, "hx-12");
        assert_eq!(targets[1].candidate.feature, "hx-13");
    }

    #[test]
    fn project_flag_wins_over_feature_names() {
        let mut store = store();
        store.projects[1].name = "hx-12".into();
        let selection = select(&store, "hx-12", true, false).unwrap();
        assert_eq!(
            selection.scope,
            Scope::Project {
                project: "hx-12".into()
            }
        );
    }

    #[test]
    fn unknown_name_lists_known_names() {
        let err = select(&store(), "nope", false, false).unwrap_err();
        assert!(err.contains("RS/hx-12"), "{err}");
        assert!(err.contains("BA (project)"), "{err}");
    }

    #[test]
    fn only_running_windows_become_targets() {
        let selection = select(&store(), "hx-12", false, false).unwrap();
        let live = vec![
            ("aiw-hx-12".to_string(), "codex".to_string(), 4),
            ("aiw-other".to_string(), "claude".to_string(), 0),
        ];
        let targets = live_targets(&selection, &live);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].window_index, 4);
        assert!(live_targets(&selection, &[]).is_empty());
    }

    #[test]
    fn parses_list_windows_output() {
        let parsed = parse_live_windows("aiw-a\tclaude\t0\naiw-a\tcodex\t2\nbroken\n");
        assert_eq!(
            parsed,
            vec![
                ("aiw-a".to_string(), "claude".to_string(), 0),
                ("aiw-a".to_string(), "codex".to_string(), 2),
            ]
        );
    }

    fn target(session: &str, window: &str, index: u32) -> GridTarget {
        GridTarget {
            candidate: Candidate {
                project: "RS".into(),
                feature: "hx-12".into(),
                label: window.into(),
                tmux_session: session.into(),
                window: window.into(),
            },
            window_index: index,
        }
    }

    #[test]
    fn view_client_uses_a_grouped_self_destroying_session() {
        let args = view_client_args(
            Some(Path::new("/s/tmux.sock")),
            &target("aiw-hx-12", "claude", 3),
            "aiw-view-1-0",
        );
        assert_eq!(&args[..2], ["-S", "/s/tmux.sock"]);
        let joined = args.join(" ");
        assert!(joined.starts_with("-S /s/tmux.sock new-session -t =aiw-hx-12 -s aiw-view-1-0 ;"));
        assert!(joined.contains("; set-option destroy-unattached on ;"));
        assert!(joined.contains("; set-option status off ;"));
        assert!(joined.ends_with("; select-window -t :3"));
        // Never targets AIW's own session for window selection.
        assert!(!joined.contains("select-window -t aiw-hx-12"));
    }

    #[test]
    fn view_client_command_is_shell_quoted_and_unsets_tmux() {
        let command = view_client_command(
            "/usr/bin/tmux",
            Some(Path::new("/home/me/my state/tmux.sock")),
            &target("aiw-x", "claude", 0),
            "aiw-view-1-0",
        );
        assert!(
            command.starts_with("exec env -u TMUX /usr/bin/tmux -S '/home/me/my state/tmux.sock' ")
        );
        assert!(command.contains(" ';' "));
        assert!(shell_quote("it's").eq("'it'\\''s'"));
    }

    #[test]
    fn grid_plan_builds_one_pane_per_target_and_tiles_them() {
        let targets = vec![
            target("aiw-a", "claude", 0),
            target("aiw-a", "codex", 1),
            target("aiw-a", "pi", 2),
        ];
        let grid = Path::new("/state/aiw-grid-42.sock");
        let plan = build_plan(
            &targets,
            "tmux",
            Some(Path::new("/s")),
            grid,
            "42",
            (120, 40),
            false,
        );
        assert_eq!(plan.grid_socket, grid);
        assert_eq!(
            plan.view_sessions,
            vec!["aiw-view-42-0", "aiw-view-42-1", "aiw-view-42-2"]
        );
        let commands: Vec<&[String]> = plan.build.split(|arg| arg == ";").collect();
        assert_eq!(commands[0], ["start-server"]);
        let has = |needle: &[&str]| {
            commands
                .iter()
                .any(|c| c.iter().map(String::as_str).eq(needle.iter().copied()))
        };
        assert!(has(&["set-option", "-g", "status", "off"]));
        assert!(has(&["set-option", "-g", "prefix", "None"]));
        assert!(has(&["set-option", "-g", "mouse", "on"]));
        assert!(has(&["set-option", "-g", "pane-border-status", "top"]));
        assert!(has(&["bind-key", "-n", "M-q", "kill-server"]));
        let new_sessions = commands.iter().filter(|c| c[0] == "new-session").count();
        let splits = commands.iter().filter(|c| c[0] == "split-window").count();
        assert_eq!((new_sessions, splits), (1, 2));
        let titles: Vec<_> = commands
            .iter()
            .filter(|c| c.len() == 6 && c[4] == "@aiw_title")
            .map(|c| c[3].clone())
            .collect();
        assert_eq!(titles, vec!["%0", "%1", "%2"]);
        assert_eq!(commands.last().unwrap(), &["select-pane", "-t", "%0"]);
        assert!(plan.attach.join(" ").contains("destroy-unattached on"));
        assert!(!plan.build.iter().any(|a| a.contains("RGB")));
        let truecolor = build_plan(&targets, "tmux", None, grid, "42", (120, 40), true);
        assert!(truecolor.build.iter().any(|a| a.contains("RGB")));
    }
}
