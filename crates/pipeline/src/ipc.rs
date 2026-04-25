//! Unix-socket IPC surface: line-delimited JSON commands dispatched to
//! the [`Reactor`].
//!
//! One `Command` per line in, one `Response` per line out, in order.
//! Connection-per-thread; EOF closes the conversation. The bound socket
//! file is unlinked when the returned [`SocketGuard`] drops at daemon
//! shutdown.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use anyhow::{Context, Result};
use gobcam_protocol::{Command, Response};
use tracing::{debug, info, warn};

use crate::assets::bootstrap::SyncProgress;
use crate::reactions::{DEFAULT_REACTION_DURATION, Reactor};

/// Bag of long-lived handles the dispatch loop needs.
#[derive(Clone)]
pub(crate) struct DispatchCtx {
    pub reactor: Arc<Reactor>,
    pub progress: Arc<SyncProgress>,
}

/// Holds the bound socket path for the daemon's lifetime; on drop, the
/// path is unlinked so a subsequent run can rebind cleanly.
#[derive(Debug)]
pub(crate) struct SocketGuard {
    path: PathBuf,
}

impl Drop for SocketGuard {
    fn drop(&mut self) {
        match std::fs::remove_file(&self.path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                warn!(path = %self.path.display(), error = %e, "removing socket failed");
            }
        }
    }
}

/// Bind `socket_path`, detach the accept loop on a worker thread, and
/// return a guard that owns the file's lifetime.
pub(crate) fn serve(ctx: DispatchCtx, socket_path: PathBuf) -> Result<SocketGuard> {
    // A leftover file from a previous run blocks bind. We don't try to
    // distinguish "stale socket" from "live daemon" — running two
    // daemons against the same loopback would already be broken, so
    // taking over the path is fine.
    if socket_path.exists() {
        std::fs::remove_file(&socket_path)
            .with_context(|| format!("removing stale socket at {}", socket_path.display()))?;
    }
    let listener = UnixListener::bind(&socket_path)
        .with_context(|| format!("binding socket at {}", socket_path.display()))?;
    info!(path = %socket_path.display(), "ipc socket bound");

    let guard_path = socket_path;
    thread::Builder::new()
        .name("ipc-accept".into())
        .spawn(move || accept_loop(&listener, &ctx))
        .context("spawning ipc accept thread")?;

    Ok(SocketGuard { path: guard_path })
}

fn accept_loop(listener: &UnixListener, ctx: &DispatchCtx) {
    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                let ctx = ctx.clone();
                if let Err(e) = thread::Builder::new()
                    .name("ipc-conn".into())
                    .spawn(move || handle_connection(stream, &ctx))
                {
                    warn!(error = %e, "spawning ipc connection thread");
                }
            }
            Err(e) => {
                warn!(error = %e, "ipc accept failed; listener exiting");
                return;
            }
        }
    }
}

fn handle_connection(stream: UnixStream, ctx: &DispatchCtx) {
    debug!("ipc connection opened");
    let Ok(read_half) = stream.try_clone() else {
        warn!("ipc stream clone failed");
        return;
    };
    let reader = BufReader::new(read_half);
    let mut writer = stream;
    for line in reader.lines() {
        let Ok(line) = line else {
            break;
        };
        if line.trim().is_empty() {
            continue;
        }
        let response = dispatch(&line, ctx);
        if write_response(&mut writer, &response).is_err() {
            break;
        }
    }
    debug!("ipc connection closed");
}

fn dispatch(line: &str, ctx: &DispatchCtx) -> Response {
    let cmd: Command = match serde_json::from_str(line) {
        Ok(c) => c,
        Err(e) => {
            return Response::Error {
                message: format!("malformed command: {e}"),
            };
        }
    };
    match cmd {
        Command::Trigger { emoji_id } => {
            match ctx
                .reactor
                .activate(&emoji_id, Some(DEFAULT_REACTION_DURATION))
            {
                Ok(()) => Response::Ok,
                Err(e) => Response::Error {
                    message: e.to_string(),
                },
            }
        }
        Command::ListEmoji => Response::EmojiList {
            items: ctx.reactor.library().list(),
        },
        Command::SyncStatus => {
            let (fetched, total, complete) = ctx.progress.snapshot();
            Response::SyncStatus {
                fetched,
                total,
                complete,
            }
        }
    }
}

fn write_response(writer: &mut UnixStream, response: &Response) -> std::io::Result<()> {
    let mut buf = serde_json::to_vec(response).map_err(std::io::Error::other)?;
    buf.push(b'\n');
    writer.write_all(&buf)
}
