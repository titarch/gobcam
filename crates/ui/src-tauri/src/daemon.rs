//! Daemon-as-child supervision for the UI process. Either attach to a
//! running daemon or spawn `gobcam-pipeline` and own the child for its
//! lifetime; on drop, send EOF and wait, then SIGKILL after a grace period.

use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use tracing::{info, warn};

const SOCKET_WAIT: Duration = Duration::from_secs(10);
const SOCKET_POLL: Duration = Duration::from_millis(50);
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);
/// Window after socket-bind during which the daemon may still fail
/// preroll (e.g. v4l2 rejecting the requested format).
const POST_BIND_HEALTH_CHECK: Duration = Duration::from_millis(800);
const POST_BIND_POLL: Duration = Duration::from_millis(50);

#[derive(Debug, Clone)]
pub(crate) struct DaemonArgs {
    pub input: PathBuf,
    pub output: PathBuf,
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub preview: bool,
    /// Daemon respawn required to change (pads pre-allocated at build).
    pub slot_count: usize,
    /// Daemon respawn required to change.
    pub slot_dim: u32,
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
            preview: false,
            slot_count: 48,
            slot_dim: 256,
        }
    }
}

#[derive(Debug)]
pub(crate) struct DaemonGuard {
    child: Child,
}

impl Drop for DaemonGuard {
    fn drop(&mut self) {
        let pid = self.child.id();
        // The UI keeps a pipe to the daemon's stdin so kernel-closes-on-UI-death
        // (or this explicit drop) trips the daemon's EOF watchdog without us
        // needing signal handling.
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

/// Attach to a live daemon at `socket` (returns `Ok(None)`) or spawn
/// `gobcam-pipeline` and wait for its socket to come up.
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

    if socket.exists() {
        let _ = std::fs::remove_file(socket);
    }

    let mut cmd = Command::new(&bin);
    cmd.arg("--socket")
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
        .arg("--slot-count")
        .arg(args.slot_count.to_string())
        .arg("--slot-dim")
        .arg(args.slot_dim.to_string())
        // Stdin is the shutdown channel (see `DaemonGuard::drop`).
        .arg("--exit-on-stdin-eof");
    if args.preview {
        cmd.arg("--preview");
    }
    let child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("spawning {}", bin.display()))?;

    let mut guard = DaemonGuard { child };

    // Block until the socket is up so the first invoke from JS doesn't race startup.
    let deadline = Instant::now() + SOCKET_WAIT;
    loop {
        if probe_socket(socket) {
            break;
        }
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

    // Socket binds before pipeline transitions to PLAYING; preroll
    // failures surface here.
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

fn probe_socket(socket: &Path) -> bool {
    if !socket.exists() {
        return false;
    }
    UnixStream::connect(socket).is_ok()
}

/// Resolve the daemon binary: first next to the running UI executable
/// (dev mode + bundled release), then fall back to `PATH`.
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
