//! FORK: cost of keeping live agent panes on screen (DESIGN.md §12.3).
//!
//! Runs the real view snapshot workers against a real tmux server on the
//! test-isolated socket (`TmuxManager::isolated_test_socket`, under
//! `$TMPDIR`) and counts the tmux processes they start and the frames they
//! deliver over a few seconds:
//!
//! ```bash
//! TMPDIR=$(mktemp -d) cargo +1.99.0 test --locked view_refresh_bench -- --ignored --nocapture --test-threads=1
//! ```
//!
//! The server is killed at the end; nothing outside that socket is touched.

use std::os::fd::{FromRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Condvar as StdCondvar, Mutex as StdMutex};
use std::time::{Duration, Instant};

use super::{App, SnapshotSender, VIEW_SNAPSHOT_REFRESH_NORMAL};
use crate::tmux::TmuxManager;

const SESSION: &str = "aiw-view-bench";
const COLS: u16 = 100;
const ROWS: u16 = 30;
const RUN_FOR: Duration = Duration::from_secs(4);

/// Output roughly like a working agent's spinner: a short line rewritten
/// every 100 ms.
const BUSY: &str = "i=0; while :; do i=$((i+1)); printf '\\r working %s ' $i; sleep 0.1; done";
const IDLE: &str = "printf 'waiting for you\\n'; sleep 100000";

fn tmux(args: &[&str]) {
    let status = TmuxManager::command()
        .args(args)
        .status()
        .expect("tmux runs");
    assert!(status.success(), "tmux {args:?} failed");
}

fn start_server() {
    let _ = TmuxManager::command().arg("kill-server").output();
    tmux(&[
        "new-session",
        "-d",
        "-s",
        SESSION,
        "-n",
        "idle",
        "-x",
        "100",
        "-y",
        "30",
        IDLE,
    ]);
    for i in 0..4 {
        tmux(&[
            "new-window",
            "-d",
            "-t",
            SESSION,
            "-n",
            &format!("busy{i}"),
            BUSY,
        ]);
    }
    std::thread::sleep(Duration::from_millis(300));
}

fn stop_server() {
    let _ = TmuxManager::command().arg("kill-server").output();
}

fn sender() -> (SnapshotSender, Arc<StdMutex<Option<super::ViewSnapshot>>>) {
    let mut fds = [0i32; 2];
    // SAFETY: plain pipe(2) into a local array; both ends are owned below.
    let (read, write) = unsafe {
        libc::pipe(fds.as_mut_ptr());
        libc::fcntl(fds[0], libc::F_SETFL, libc::O_NONBLOCK);
        libc::fcntl(fds[1], libc::F_SETFL, libc::O_NONBLOCK);
        (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1]))
    };
    // Keep the read end alive (and drained) for the run.
    std::mem::forget(read);
    let slot = Arc::new(StdMutex::new(None));
    (
        SnapshotSender {
            slot: slot.clone(),
            wakeup_fd: Arc::new(write),
        },
        slot,
    )
}

struct Handles {
    stop: Arc<AtomicBool>,
    condvar: Arc<(StdMutex<()>, StdCondvar)>,
    slot: Arc<StdMutex<Option<super::ViewSnapshot>>>,
    thread: std::thread::JoinHandle<()>,
}

#[derive(Clone, Copy)]
enum Worker {
    /// Direct mode (macOS default) and split-view tiles.
    PipePane { background: bool },
    /// Linux single view.
    ControlMode,
}

fn spawn(window: &str, worker: Worker) -> Handles {
    let stop = Arc::new(AtomicBool::new(false));
    let refresh = Arc::new(AtomicU8::new(VIEW_SNAPSHOT_REFRESH_NORMAL));
    let condvar = Arc::new((StdMutex::new(()), StdCondvar::new()));
    let (tx, slot) = sender();
    let (stop2, condvar2) = (stop.clone(), condvar.clone());
    let window = window.to_string();
    let thread = std::thread::spawn(move || match worker {
        Worker::PipePane { background } => {
            let throttle = AtomicBool::new(background);
            let _ = App::run_pipe_pane_worker(
                SESSION, &window, COLS, ROWS, &stop2, &refresh, &throttle, &condvar2, &tx,
            );
        }
        Worker::ControlMode => {
            let include = AtomicBool::new(true);
            let _ = App::run_control_mode_view_worker(
                SESSION, &window, COLS, ROWS, &stop2, &refresh, &include, true, &condvar2, &tx,
            );
        }
    });
    Handles {
        stop,
        condvar,
        slot,
        thread,
    }
}

/// Run `workers` for [`RUN_FOR`], taking frames like the main loop does, and
/// return (tmux processes per second, frames per second).
fn measure(name: &str, workers: &[(&str, Worker)]) -> (f64, f64) {
    let before = TmuxManager::processes_started();
    let handles: Vec<Handles> = workers.iter().map(|(w, k)| spawn(w, *k)).collect();
    let started = Instant::now();
    let mut frames = 0u64;
    while started.elapsed() < RUN_FOR {
        for handle in &handles {
            if handle.slot.lock().unwrap().take().is_some() {
                frames += 1;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    for handle in &handles {
        handle.stop.store(true, Ordering::Relaxed);
        handle.condvar.1.notify_all();
    }
    let elapsed = started.elapsed().as_secs_f64();
    for handle in handles {
        let _ = handle.thread.join();
    }
    let processes = (TmuxManager::processes_started() - before) as f64 / elapsed;
    let frames = frames as f64 / elapsed;
    println!("{name:<48} {processes:>6.1} tmux processes/s {frames:>6.1} frames/s");
    (processes, frames)
}

#[test]
#[ignore = "needs a real tmux; run with --ignored --nocapture --test-threads=1"]
fn view_refresh_bench() {
    start_server();
    let pipe = Worker::PipePane { background: false };
    let pipe_bg = Worker::PipePane { background: true };
    measure(
        "single view, idle pane, pipe-pane (macOS)",
        &[("idle", pipe)],
    );
    measure(
        "single view, busy pane, pipe-pane (macOS)",
        &[("busy0", pipe)],
    );
    measure(
        "single view, idle pane, control mode (Linux)",
        &[("idle", Worker::ControlMode)],
    );
    measure(
        "single view, busy pane, control mode (Linux)",
        &[("busy0", Worker::ControlMode)],
    );
    measure(
        "split view, 4 busy tiles (1 focused)",
        &[
            ("busy0", pipe),
            ("busy1", pipe_bg),
            ("busy2", pipe_bg),
            ("busy3", pipe_bg),
        ],
    );
    stop_server();
}

#[test]
#[ignore = "needs a real tmux; run with --ignored --test-threads=1"]
fn combined_capture_matches_the_two_separate_calls() {
    start_server();
    tmux(&["send-keys", "-t", &format!("{SESSION}:idle"), "hello", ""]);
    std::thread::sleep(Duration::from_millis(200));
    let (content, cursor) =
        TmuxManager::capture_pane_ansi_with_cursor(SESSION, "idle").expect("tmux runs");
    let separate = TmuxManager::capture_pane_ansi(SESSION, "idle").expect("tmux runs");
    let separate_cursor = TmuxManager::cursor_position(SESSION, "idle").expect("tmux runs");
    stop_server();
    assert_eq!(content, separate);
    assert_eq!(cursor, separate_cursor);
    assert!(content.contains("waiting for you"), "{content:?}");
}
