//! FORK: the cockpit (DESIGN.md §18) — tmux draws the dashboard.
//!
//! `aiw` with no subcommand is a thin launcher. It starts (or finds) a
//! throw-away tmux server on its own socket (`cockpit.sock`, next to AIW's
//! private socket, `-f /dev/null`) holding one session, `aiw`, with one
//! window and two panes:
//!
//! * the left pane runs `aiw sidebar` — the whole dashboard app (instance
//!   lock, IPC socket, database, hooks), drawn at the pane's width;
//! * the right pane shows the selected agent's *real* tmux window through a
//!   grouped view session on AIW's own socket, exactly as `aiw grid` does
//!   (`new-session -t <feature session> -s aiw-view-… ; destroy-unattached
//!   on`), so typing there is native — the same as `tmux attach` — with no
//!   key relay and no capture-and-render loop. With nothing to show it runs
//!   `aiw sidebar --placeholder`.
//!
//! The launcher then attaches this terminal to the cockpit server and exits
//! when the client returns. Closing the terminal only hangs up the client:
//! the sidebar and the agents keep running, and the next `aiw` re-attaches.
//! `q` in the sidebar tears the server down.
//!
//! Everything the sidebar asks of the cockpit server (show this agent, zoom
//! for a dialog, focus the agent pane, tear down) goes through a
//! [`Link`]: a channel to one worker thread that runs the tmux commands, so
//! nothing here runs on the drawing path and the frame budget is untouched.

use std::ffi::OsString;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result, bail};

use super::grid::shell_quote;

/// The session inside the cockpit server.
pub const SESSION: &str = "aiw";
/// Its one window.
pub const WINDOW: &str = "cockpit";
/// The sidebar pane (left).
pub const SIDEBAR_PANE: &str = "aiw:0.0";
/// The agent pane (right).
pub const AGENT_PANE: &str = "aiw:0.1";
/// Root-table key that flips the focus sidebar ⇄ agent (a no-op while a
/// dialog has the sidebar zoomed).
pub const FOCUS_KEY: &str = "C-q";
/// `sidebar_width` default and floor (`config.json`).
pub const DEFAULT_SIDEBAR_WIDTH: u16 = 36;
pub const MIN_SIDEBAR_WIDTH: u16 = 28;
/// Title of the sidebar pane's border.
pub const SIDEBAR_TITLE: &str = "AIW";
/// Title of the agent pane while nothing is shown.
pub const PLACEHOLDER_TITLE: &str = "select a session";
/// Prefix of the grouped view sessions the cockpit creates on AIW's socket
/// (`aiw-view-ck<sidebar pid>-<n>`), for cleanup.
pub const VIEW_SESSION_PREFIX: &str = "aiw-view-ck";

/// The sidebar width from `config.json`, clamped to the floor.
pub fn sidebar_width(configured: u16) -> u16 {
    configured.max(MIN_SIDEBAR_WIDTH)
}

/// Socket of the cockpit server: next to AIW's own tmux socket (falling back
/// to the temp dir when that path would be too long for a unix socket).
pub fn socket_path() -> PathBuf {
    let [private, _] = crate::tmux::TmuxManager::default_socket_paths();
    socket_path_beside(&private)
}

fn socket_path_beside(private: &Path) -> PathBuf {
    let name = "cockpit.sock";
    let preferred = private
        .parent()
        .map(|dir| dir.join(name))
        .unwrap_or_else(|| std::env::temp_dir().join(name));
    if preferred.as_os_str().len() < 100 {
        preferred
    } else {
        std::env::temp_dir().join(format!("aiw-{}-{name}", std::process::id()))
    }
}

/// `#rrggbb` for tmux from a theme colour.
pub fn tmux_colour((r, g, b): (u8, u8, u8)) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// The one tmux command (after `-S <socket> -f /dev/null`) that starts the
/// cockpit server and builds the session: options, the focus key, the
/// resize hook, the two panes and their titles.
///
/// `remain-on-exit` is on, so a pane whose command ends (a crashed sidebar,
/// an agent window that closed) stays as a dead pane the launcher respawns
/// at the next `aiw`, instead of vanishing and leaving a one-pane window.
pub fn build_args(
    sidebar_command: &str,
    placeholder_command: &str,
    size: (u16, u16),
    sidebar_width: u16,
    truecolor: bool,
) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    let mut push = |cmd: &[&str]| {
        if !args.is_empty() {
            args.push(";".to_string());
        }
        args.extend(cmd.iter().map(|s| s.to_string()));
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
        ("set-titles-string", "AI Workstation"),
        ("window-size", "latest"),
        ("remain-on-exit", "on"),
    ] {
        push(&["set-option", "-g", option, value]);
    }
    if truecolor {
        push(&["set-option", "-ga", "terminal-features", ",*:RGB"]);
    }
    push(&["bind-key", "-n", FOCUS_KEY, &focus_key_command()]);
    push(&[
        "set-hook",
        "-g",
        "client-resized",
        &resize_hook(sidebar_width),
    ]);
    let cols = size.0.max(40).to_string();
    let rows = size.1.max(10).to_string();
    push(&[
        "new-session",
        "-d",
        "-s",
        SESSION,
        "-n",
        WINDOW,
        "-x",
        &cols,
        "-y",
        &rows,
        sidebar_command,
    ]);
    push(&[
        "set-option",
        "-p",
        "-t",
        SIDEBAR_PANE,
        "@aiw_title",
        SIDEBAR_TITLE,
    ]);
    push(&[
        "split-window",
        "-h",
        "-t",
        SIDEBAR_PANE,
        placeholder_command,
    ]);
    push(&[
        "set-option",
        "-p",
        "-t",
        AGENT_PANE,
        "@aiw_title",
        PLACEHOLDER_TITLE,
    ]);
    push(&[
        "resize-pane",
        "-t",
        SIDEBAR_PANE,
        "-x",
        &sidebar_width.to_string(),
    ]);
    push(&["select-pane", "-t", SIDEBAR_PANE]);
    args
}

/// `C-q`: flip panes, unless a dialog has the sidebar zoomed.
fn focus_key_command() -> String {
    "if-shell -F '#{window_zoomed_flag}' '' 'last-pane'".to_string()
}

/// Keep the sidebar at its width when the terminal is resized (tmux scales
/// panes proportionally otherwise); never while zoomed, since `resize-pane`
/// would un-zoom the dialog.
fn resize_hook(sidebar_width: u16) -> String {
    format!(
        "if-shell -F '#{{window_zoomed_flag}}' '' 'resize-pane -t {SIDEBAR_PANE} -x {sidebar_width}'"
    )
}

/// `env -u TMUX <aiw> sidebar`: the dashboard, with the cockpit's own
/// `$TMUX` stripped so AIW's tmux clients never think they are nested.
pub fn sidebar_command(aiw: &str) -> String {
    format!("env -u TMUX {} sidebar", shell_quote(aiw))
}

/// `exec env -u TMUX <aiw> sidebar --placeholder`.
pub fn placeholder_command(aiw: &str) -> String {
    format!(
        "exec env -u TMUX {} sidebar --placeholder",
        shell_quote(aiw)
    )
}

/// The tmux argv (without the binary) that shows one agent window through
/// a grouped view session on AIW's socket, as `aiw grid` builds it but by
/// window *name* (the sidebar knows names, not indexes).
pub fn view_client_args(
    aiw_socket: Option<&Path>,
    tmux_session: &str,
    window: &str,
    view_session: &str,
) -> Vec<String> {
    let mut args = Vec::new();
    if let Some(socket) = aiw_socket {
        args.push("-S".to_string());
        args.push(socket.to_string_lossy().into_owned());
    }
    let target = format!(":={window}");
    args.extend(
        [
            "new-session",
            "-t",
            &format!("={tmux_session}"),
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
            "set-window-option",
            "-t",
            &target,
            "window-size",
            "latest",
            ";",
            "select-window",
            "-t",
            &target,
        ]
        .map(String::from),
    );
    args
}

/// The shell command that retitles the agent pane from inside it (the pane
/// command's own `$TMUX` points at the cockpit server; the socket is given
/// explicitly all the same).
pub fn title_command(tmux_bin: &str, cockpit_socket: &Path, title: &str) -> String {
    format!(
        "{} -S {} set-option -p -t {AGENT_PANE} @aiw_title {}",
        shell_quote(tmux_bin),
        shell_quote(&cockpit_socket.to_string_lossy()),
        shell_quote(title)
    )
}

/// What the agent pane runs for one agent window: the view client, then —
/// when it ends (the window closed, the feature stopped) — the placeholder,
/// retitling the pane on the way.
pub fn agent_pane_command(
    tmux_bin: &str,
    aiw_socket: Option<&Path>,
    cockpit_socket: &Path,
    tmux_session: &str,
    window: &str,
    view_session: &str,
    aiw: &str,
) -> String {
    let mut parts = vec![
        "env".to_string(),
        "-u".to_string(),
        "TMUX".to_string(),
        shell_quote(tmux_bin),
    ];
    parts.extend(
        view_client_args(aiw_socket, tmux_session, window, view_session)
            .iter()
            .map(|arg| shell_quote(arg)),
    );
    format!(
        "{}; {}; {}",
        parts.join(" "),
        title_command(tmux_bin, cockpit_socket, PLACEHOLDER_TITLE),
        placeholder_command(aiw)
    )
}

/// What the agent pane runs for `v`: `aiw grid <name>` (native tiles; Alt+q
/// closes them), then `after` — the selected agent's command, under its own
/// title again — which ends in the placeholder.
pub fn grid_pane_command(
    aiw: &str,
    grid_name: &str,
    project: bool,
    tmux_bin: &str,
    cockpit_socket: &Path,
    after_title: &str,
    after: &str,
) -> String {
    let flag = if project { " --project" } else { "" };
    format!(
        "env -u TMUX {} grid{flag} {}; {}; {after}",
        shell_quote(aiw),
        shell_quote(grid_name),
        title_command(tmux_bin, cockpit_socket, after_title)
    )
}

/// One thing the sidebar asks of the cockpit server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Run `command` in the agent pane, titled `title`.
    Respawn {
        command: String,
        title: String,
    },
    /// Zoom the sidebar pane (a dialog is open) or restore the two panes.
    Zoom(bool),
    FocusAgent,
    FocusSidebar,
    /// Pin the sidebar's width (also updates the resize hook).
    SidebarWidth(u16),
    /// `pane-active-border-style` from the theme (`#rrggbb` or a tmux name).
    BorderColour(String),
}

enum Transport {
    Worker(Sender<Request>),
    Recorder(Arc<Mutex<Vec<Request>>>),
}

/// The sidebar's handle on the cockpit server.
pub struct Link {
    transport: Transport,
}

impl Link {
    /// A link that runs requests on a worker thread against `socket`.
    pub fn spawn(socket: PathBuf) -> Self {
        let (tx, rx) = channel();
        std::thread::Builder::new()
            .name("aiw-cockpit".into())
            .spawn(move || worker(socket, rx))
            .expect("failed to start the cockpit worker");
        Self {
            transport: Transport::Worker(tx),
        }
    }

    /// A link that only records what was asked (tests).
    pub fn recording() -> Self {
        Self {
            transport: Transport::Recorder(Arc::new(Mutex::new(Vec::new()))),
        }
    }

    pub fn send(&self, request: Request) {
        match &self.transport {
            Transport::Worker(tx) => {
                let _ = tx.send(request);
            }
            Transport::Recorder(log) => log.lock().unwrap().push(request),
        }
    }

    /// Everything recorded so far (empty for a worker link).
    pub fn recorded(&self) -> Vec<Request> {
        match &self.transport {
            Transport::Worker(_) => Vec::new(),
            Transport::Recorder(log) => log.lock().unwrap().clone(),
        }
    }

    /// Drop the recorded requests (tests).
    pub fn clear_recorded(&self) {
        if let Transport::Recorder(log) = &self.transport {
            log.lock().unwrap().clear();
        }
    }
}

fn tmux_bin() -> OsString {
    crate::tmux::TmuxManager::runtime_binary_and_socket().0
}

fn cockpit(bin: &OsString, socket: &Path) -> Command {
    let mut command = Command::new(bin);
    command.arg("-S").arg(socket);
    command.env_remove("TMUX").env_remove("TMUX_PANE");
    command
}

fn run_quiet(command: &mut Command) -> bool {
    command
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn zoomed(bin: &OsString, socket: &Path) -> bool {
    cockpit(bin, socket)
        .args([
            "display-message",
            "-p",
            "-t",
            SIDEBAR_PANE,
            "#{window_zoomed_flag}",
        ])
        .output()
        .ok()
        .is_some_and(|out| String::from_utf8_lossy(&out.stdout).trim() == "1")
}

/// The worker: one tmux process per request, the newest `Respawn` winning
/// when several queued up while a previous one ran.
fn worker(socket: PathBuf, rx: Receiver<Request>) {
    let bin = tmux_bin();
    let mut width = DEFAULT_SIDEBAR_WIDTH;
    while let Ok(first) = rx.recv() {
        let mut batch = vec![first];
        while let Ok(next) = rx.try_recv() {
            batch.push(next);
        }
        // Only the last respawn matters: the earlier ones would be killed
        // by it at once.
        let last_respawn = batch
            .iter()
            .rposition(|r| matches!(r, Request::Respawn { .. }));
        for (i, request) in batch.into_iter().enumerate() {
            match request {
                Request::Respawn { command, title } => {
                    if last_respawn != Some(i) {
                        continue;
                    }
                    let ok = run_quiet(cockpit(&bin, &socket).args([
                        "respawn-pane",
                        "-k",
                        "-t",
                        AGENT_PANE,
                        &command,
                    ]));
                    if ok {
                        run_quiet(cockpit(&bin, &socket).args([
                            "set-option",
                            "-p",
                            "-t",
                            AGENT_PANE,
                            "@aiw_title",
                            &title,
                        ]));
                    } else {
                        crate::debug::log_to_file(
                            crate::debug::LogLevel::Warn,
                            "cockpit",
                            &format!("respawn-pane failed for {title}"),
                        );
                    }
                }
                Request::Zoom(want) => {
                    if zoomed(&bin, &socket) != want {
                        run_quiet(cockpit(&bin, &socket).args([
                            "resize-pane",
                            "-Z",
                            "-t",
                            SIDEBAR_PANE,
                        ]));
                    }
                    if !want {
                        // The terminal may have been resized while zoomed
                        // (the hook stays quiet then); pin the width again.
                        run_quiet(cockpit(&bin, &socket).args([
                            "resize-pane",
                            "-t",
                            SIDEBAR_PANE,
                            "-x",
                            &width.to_string(),
                        ]));
                    }
                }
                Request::FocusAgent => {
                    run_quiet(cockpit(&bin, &socket).args(["select-pane", "-t", AGENT_PANE]));
                }
                Request::FocusSidebar => {
                    run_quiet(cockpit(&bin, &socket).args(["select-pane", "-t", SIDEBAR_PANE]));
                }
                Request::SidebarWidth(w) => {
                    width = sidebar_width(w);
                    run_quiet(cockpit(&bin, &socket).args([
                        "set-hook",
                        "-g",
                        "client-resized",
                        &resize_hook(width),
                    ]));
                    if !zoomed(&bin, &socket) {
                        run_quiet(cockpit(&bin, &socket).args([
                            "resize-pane",
                            "-t",
                            SIDEBAR_PANE,
                            "-x",
                            &width.to_string(),
                        ]));
                    }
                }
                Request::BorderColour(colour) => {
                    run_quiet(cockpit(&bin, &socket).args([
                        "set-option",
                        "-g",
                        "pane-active-border-style",
                        &format!("fg={colour},bold"),
                    ]));
                }
            }
        }
    }
}

/// Is a cockpit server with the `aiw` session up on `socket`?
pub fn alive(socket: &Path) -> bool {
    socket.exists()
        && run_quiet(cockpit(&tmux_bin(), socket).args([
            "has-session",
            "-t",
            &format!("={SESSION}"),
        ]))
}

/// `(pane index, dead)` for the cockpit window.
fn panes(bin: &OsString, socket: &Path) -> Vec<(u32, bool)> {
    cockpit(bin, socket)
        .args([
            "list-panes",
            "-t",
            &format!("{SESSION}:0"),
            "-F",
            "#{pane_index}\t#{pane_dead}",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter_map(|line| {
                    let (index, dead) = line.split_once('\t')?;
                    Some((index.trim().parse().ok()?, dead.trim() == "1"))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Kill the cockpit server and the view sessions this sidebar made on
/// AIW's socket. Called by the sidebar on `q` (its own pane dies with the
/// server, so SIGHUP is ignored first) and by the launcher when it rebuilds.
pub fn teardown(socket: &Path, view_prefix: &str) {
    let bin = tmux_bin();
    run_quiet(cockpit(&bin, socket).arg("kill-server"));
    let _ = std::fs::remove_file(socket);
    kill_view_sessions(view_prefix);
}

/// Kill the grouped view sessions named `<prefix>…` on AIW's socket (they
/// are `destroy-unattached`, so this only matters when a client lingered).
pub fn kill_view_sessions(prefix: &str) {
    let mut command = crate::tmux::TmuxManager::command();
    let Ok(output) = command
        .args(["list-sessions", "-F", "#{session_name}"])
        .output()
    else {
        return;
    };
    for name in String::from_utf8_lossy(&output.stdout).lines() {
        if name.starts_with(prefix) {
            let _ = crate::tmux::TmuxManager::command()
                .args(["kill-session", "-t", &format!("={name}")])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
    }
}

/// The path of this executable, for the pane commands.
fn aiw_exe() -> Result<String> {
    let exe = std::env::current_exe().context("could not find the aiw binary")?;
    Ok(exe.to_string_lossy().into_owned())
}

/// Entry point of `aiw` with no subcommand: build the cockpit if there is
/// none, repair dead panes if there is, then attach this terminal.
pub fn launch(configured_width: u16) -> Result<i32> {
    if !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
        bail!("aiw needs an interactive terminal (run it in a terminal window)");
    }
    let bin = tmux_bin();
    let socket = socket_path();
    if let Some(dir) = socket.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let aiw = aiw_exe()?;
    let width = sidebar_width(configured_width);

    if alive(&socket) {
        let panes = panes(&bin, &socket);
        if panes.len() != 2 {
            // A one-pane window cannot be repaired in place: rebuild.
            teardown(&socket, VIEW_SESSION_PREFIX);
        } else {
            for (index, dead) in panes {
                if !dead {
                    continue;
                }
                let (target, command) = if index == 0 {
                    (SIDEBAR_PANE, sidebar_command(&aiw))
                } else {
                    (AGENT_PANE, placeholder_command(&aiw))
                };
                run_quiet(cockpit(&bin, &socket).args([
                    "respawn-pane",
                    "-k",
                    "-t",
                    target,
                    &command,
                ]));
            }
            eprintln!("aiw: attaching to the running dashboard");
        }
    }

    if !alive(&socket) {
        let size = crossterm::terminal::size().unwrap_or((200, 50));
        let truecolor = std::env::var("COLORTERM")
            .is_ok_and(|value| matches!(value.as_str(), "truecolor" | "24bit"));
        let args = build_args(
            &sidebar_command(&aiw),
            &placeholder_command(&aiw),
            size,
            width,
            truecolor,
        );
        let output = cockpit(&bin, &socket)
            .args(["-f", "/dev/null"])
            .args(&args)
            .output()
            .context("could not start tmux for the dashboard")?;
        if !output.status.success() {
            teardown(&socket, VIEW_SESSION_PREFIX);
            bail!(
                "could not build the dashboard: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
    }

    let status = cockpit(&bin, &socket)
        .args(["attach-session", "-t", &format!("={SESSION}")])
        .status()
        .context("could not attach to the dashboard")?;
    // `q` kills the server under the client, which tmux reports as a
    // non-zero exit; that is the normal way out.
    Ok(if alive(&socket) {
        status.code().unwrap_or(0)
    } else {
        0
    })
}

/// `aiw sidebar --placeholder`: what the agent pane shows with nothing
/// selected. Prints a hint and waits to be killed (`respawn-pane -k`).
pub fn run_placeholder() -> ! {
    use std::io::Write;
    let mut out = std::io::stdout();
    let _ = write!(
        out,
        "\x1b[2J\x1b[H\r\n\r\n   \x1b[2m{PLACEHOLDER_TITLE} — j/k in the sidebar picks one, Enter focuses it here, Ctrl+Q flips back\x1b[0m\r\n"
    );
    let _ = out.flush();
    loop {
        std::thread::park();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commands(args: &[String]) -> Vec<Vec<&str>> {
        args.split(|a| a == ";")
            .map(|cmd| cmd.iter().map(String::as_str).collect())
            .collect()
    }

    #[test]
    fn the_plan_builds_two_titled_panes_with_the_focus_key_and_resize_hook() {
        let args = build_args(
            "env -u TMUX aiw sidebar",
            "exec aiw sidebar --placeholder",
            (200, 60),
            36,
            true,
        );
        let cmds = commands(&args);
        assert_eq!(cmds[0], vec!["start-server"]);
        let has = |needle: &[&str]| cmds.iter().any(|c| c.as_slice() == needle);
        assert!(has(&["set-option", "-g", "status", "off"]));
        assert!(has(&["set-option", "-g", "prefix", "None"]));
        assert!(has(&["set-option", "-g", "mouse", "on"]));
        assert!(has(&["set-option", "-g", "pane-border-status", "top"]));
        assert!(has(&["set-option", "-g", "window-size", "latest"]));
        assert!(has(&["set-option", "-g", "remain-on-exit", "on"]));
        assert!(has(&["set-option", "-ga", "terminal-features", ",*:RGB"]));
        let bind = cmds.iter().find(|c| c[0] == "bind-key").unwrap();
        assert_eq!(&bind[1..3], ["-n", "C-q"]);
        assert!(bind[3].contains("last-pane") && bind[3].contains("window_zoomed_flag"));
        let hook = cmds.iter().find(|c| c[0] == "set-hook").unwrap();
        assert_eq!(hook[2], "client-resized");
        assert!(hook[3].contains("resize-pane -t aiw:0.0 -x 36"));
        let new = cmds.iter().find(|c| c[0] == "new-session").unwrap();
        assert_eq!(new.last().unwrap(), &"env -u TMUX aiw sidebar");
        assert!(new.contains(&"-x") && new.contains(&"200"));
        let split = cmds.iter().find(|c| c[0] == "split-window").unwrap();
        assert_eq!(split.last().unwrap(), &"exec aiw sidebar --placeholder");
        assert!(has(&[
            "set-option",
            "-p",
            "-t",
            "aiw:0.0",
            "@aiw_title",
            "AIW"
        ]));
        assert!(has(&[
            "set-option",
            "-p",
            "-t",
            "aiw:0.1",
            "@aiw_title",
            "select a session"
        ]));
        assert!(has(&["resize-pane", "-t", "aiw:0.0", "-x", "36"]));
        assert_eq!(cmds.last().unwrap(), &vec!["select-pane", "-t", "aiw:0.0"]);
        let plain = build_args("a", "b", (200, 60), 36, false);
        assert!(!plain.iter().any(|a| a.contains("RGB")));
    }

    #[test]
    fn the_agent_pane_runs_a_grouped_view_client_then_the_placeholder() {
        let command = agent_pane_command(
            "/usr/bin/tmux",
            Some(Path::new("/s/tmux.sock")),
            Path::new("/s/cockpit.sock"),
            "aiw-hx-12",
            "claude",
            "aiw-view-ck1-3",
            "/bin/aiw",
        );
        assert!(command.starts_with(
            "env -u TMUX /usr/bin/tmux -S /s/tmux.sock new-session -t =aiw-hx-12 -s aiw-view-ck1-3 "
        ));
        assert!(command.contains(" ';' set-option destroy-unattached on "));
        assert!(command.contains(
            " select-window -t :=claude; /usr/bin/tmux -S /s/cockpit.sock set-option -p -t aiw:0.1 @aiw_title 'select a session'; "
        ));
        assert!(command.ends_with("exec env -u TMUX /bin/aiw sidebar --placeholder"));
        assert!(!command.contains("select-window -t aiw-hx-12"));
    }

    #[test]
    fn the_grid_pane_returns_to_the_agent_after_alt_q() {
        let after = "env -u TMUX tmux …; exec env -u TMUX aiw sidebar --placeholder";
        let command = grid_pane_command(
            "/bin/aiw",
            "RS/hx-12",
            false,
            "tmux",
            Path::new("/s/cockpit.sock"),
            "RS › hx-12 › Claude 1",
            after,
        );
        assert_eq!(
            command,
            format!(
                "env -u TMUX /bin/aiw grid RS/hx-12; tmux -S /s/cockpit.sock set-option -p -t aiw:0.1 @aiw_title 'RS › hx-12 › Claude 1'; {after}"
            )
        );
        let project = grid_pane_command(
            "/bin/aiw",
            "RS",
            true,
            "tmux",
            Path::new("/s/cockpit.sock"),
            "t",
            after,
        );
        assert!(project.starts_with("env -u TMUX /bin/aiw grid --project RS; "));
    }

    #[test]
    fn widths_and_colours() {
        assert_eq!(sidebar_width(10), MIN_SIDEBAR_WIDTH);
        assert_eq!(sidebar_width(50), 50);
        assert_eq!(tmux_colour((255, 176, 0)), "#ffb000");
        assert_eq!(
            socket_path_beside(Path::new("/s/state/tmux.sock")),
            PathBuf::from("/s/state/cockpit.sock")
        );
        let long = format!("/{}/tmux.sock", "x".repeat(120));
        assert!(socket_path_beside(Path::new(&long)).starts_with(std::env::temp_dir()));
    }

    #[test]
    fn a_recording_link_keeps_requests_in_order() {
        let link = Link::recording();
        link.send(Request::Zoom(true));
        link.send(Request::FocusAgent);
        assert_eq!(
            link.recorded(),
            vec![Request::Zoom(true), Request::FocusAgent]
        );
        link.clear_recorded();
        assert!(link.recorded().is_empty());
    }
}
