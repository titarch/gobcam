//! Daemon-as-child supervision for the UI process.
//!
//! On startup we either *attach* to a daemon someone else launched
//! (helpful during `--profile-log` debugging where you want the
//! daemon under your shell) or *spawn* the bundled `gobcam-pipeline`
//! ourselves. The supervisor holds the [`Child`] for the daemon's
//! lifetime; on drop it sends `SIGINT`, waits up to
//! [`SHUTDOWN_GRACE`], then `SIGKILL` if still alive.

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use tracing::{info, warn};

const SOCKET_WAIT: Duration = Duration::from_secs(10);
const SOCKET_POLL: Duration = Duration::from_millis(50);
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);
/// How long to keep watching the daemon for early-exit after it has
/// bound its socket. Pipeline preroll (PLAYING transition) finishes
/// in well under a second on a healthy daemon; this catches the
/// failure case where v4l2 refuses the requested format.
const POST_BIND_HEALTH_CHECK: Duration = Duration::from_millis(800);
const POST_BIND_POLL: Duration = Duration::from_millis(50);

/// Args the UI passes to the spawned daemon. Mutable so the user can
/// switch input + mode at runtime via `apply_settings` (which
/// restarts the daemon with the new values).
#[derive(Debug, Clone)]
pub(crate) struct DaemonArgs {
    pub input: PathBuf,
    pub output: PathBuf,
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
}

impl Default for DaemonArgs {
    fn default() -> Self {
        Self {
            input: PathBuf::from("/dev/video0"),
            output: PathBuf::from("/dev/video10"),
            width: 1280,
            height: 720,
            fps_num: 30,
            fps_den: 1,
        }
    }
}

/// Owns the spawned daemon process. Sends SIGINT on drop and waits up
/// to `SHUTDOWN_GRACE` for clean exit before SIGKILL.
#[derive(Debug)]
pub(crate) struct DaemonGuard {
    child: Child,
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let pid = self.child.id();
        // Closing stdin triggers the daemon's `--exit-on-stdin-eof`
        // watchdog, which calls `process::exit(0)` cleanly. This same
        // mechanism handles non-graceful UI death (SIGKILL/crash):
        // the kernel closes the pipe automatically.
        info!(pid, "closing daemon stdin (watchdog → clean shutdown)");
        let _ = self.child.stdin.take();

        let deadline = Instant::now() + SHUTDOWN_GRACE;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => {
                    info!(pid, ?status, "daemon exited");
                    return;
                }
                Ok(None) => {
                    if Instant::now() >= deadline {
                        warn!(pid, "daemon didn't exit on stdin EOF; sending SIGKILL");
                        let _ = self.child.kill();
                        let _ = self.child.wait();
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => {
                    warn!(pid, error = %e, "wait on daemon failed");
                    return;
                }
            }
        }
    }
}

/// If `socket` is already a live IPC endpoint, attach (no spawn) and
/// return `Ok(None)`. Otherwise spawn `gobcam-pipeline --socket
/// <socket> --input <…> --output <…>`, wait for the socket to appear,
/// return the guard.
pub(crate) fn spawn_or_attach(socket: &Path, args: &DaemonArgs) -> Result<Option<DaemonGuard>> {
    if probe_socket(socket) {
        info!(path = %socket.display(), "attaching to running daemon (skip spawn)");
        return Ok(None);
    }

    let bin = locate_daemon_binary().context("locating gobcam-pipeline binary")?;
    info!(
        bin = %bin.display(),
        socket = %socket.display(),
        input = %args.input.display(),
        "spawning daemon"
    );

    // Best-effort: clean up any stale socket file from a previous run.
    if socket.exists() {
        let _ = std::fs::remove_file(socket);
    }

    let child = Command::new(&bin)
        .arg("--socket")
        .arg(socket)
        .arg("--input")
        .arg(&args.input)
        .arg("--output")
        .arg(&args.output)
        .arg("--width")
        .arg(args.width.to_string())
        .arg("--height")
        .arg(args.height.to_string())
        .arg("--fps-num")
        .arg(args.fps_num.to_string())
        .arg("--fps-den")
        .arg(args.fps_den.to_string())
        // Open stdin pipe + ask the daemon to exit on EOF. We never
        // write to it; closing it on UI exit (or kernel-closing on
        // UI crash) is the shutdown signal.
        .arg("--exit-on-stdin-eof")
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("spawning {}", bin.display()))?;

    let mut guard = DaemonGuard { child };

    // Block until the daemon has bound its socket — otherwise the
    // first invoke from JS would race the daemon's startup.
    let deadline = Instant::now() + SOCKET_WAIT;
    loop {
        if probe_socket(socket) {
            break;
        }
        // Catch daemons that fail before binding (e.g. invalid args).
        if let Some(status) = guard.child.try_wait().ok().flatten() {
            bail!(
                "daemon exited early with {status} before binding socket {}",
                socket.display()
            );
        }
        if Instant::now() >= deadline {
            bail!(
                "daemon did not bind {} within {SOCKET_WAIT:?}",
                socket.display()
            );
        }
        std::thread::sleep(SOCKET_POLL);
    }

    // The daemon binds its socket *before* transitioning the GStreamer
    // pipeline to PLAYING. Failures during preroll (e.g. v4l2sink
    // refusing the requested format) happen after socket binding, so
    // briefly watch the child for early exit before declaring the
    // daemon healthy.
    let post_bind_deadline = Instant::now() + POST_BIND_HEALTH_CHECK;
    while Instant::now() < post_bind_deadline {
        if let Some(status) = guard.child.try_wait().ok().flatten() {
            bail!(
                "daemon exited with {status} during pipeline startup. \
                 Common cause: the requested camera mode is unsupported, or \
                 the loopback (`/dev/video10`) is currently locked by an \
                 active consumer (close `view-loopback` / Teams and try again, \
                 or run `just reset-loopback`). See the daemon's stderr above \
                 for the exact GStreamer error."
            );
        }
        std::thread::sleep(POST_BIND_POLL);
    }

    Ok(Some(guard))
}

/// Try to connect to `socket`; if it accepts a connection without
/// errors, treat it as a live daemon.
fn probe_socket(socket: &Path) -> bool {
    if !socket.exists() {
        return false;
    }
    UnixStream::connect(socket).is_ok()
}

/// Resolve the daemon binary. First look next to the running UI
/// executable (works for `cargo run -p gobcam-ui` dev mode, for
/// `pnpm tauri dev` (same target dir), and for a bundled release
/// where Tauri places the sidecar adjacent to the main binary).
/// Fall back to `PATH`.
fn locate_daemon_binary() -> Result<PathBuf> {
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        let candidate = dir.join("gobcam-pipeline");
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    if let Ok(found) = which("gobcam-pipeline") {
        return Ok(found);
    }
    bail!(
        "could not find `gobcam-pipeline` next to the UI binary or on PATH; \
         build it (`cargo build -p gobcam-pipeline`) before launching the UI"
    )
}

/// Tiny standalone PATH search; avoids pulling in the `which` crate
/// for a one-call use.
fn which(name: &str) -> Result<PathBuf> {
    let path = std::env::var_os("PATH").context("PATH not set")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    bail!("`{name}` not on PATH");
}
