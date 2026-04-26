//! Lazy, single-connection client for the daemon's Unix-socket IPC
//! surface (`crates/pipeline/src/ipc.rs`). One outstanding request at a
//! time per `IpcClient` — fine for human-driven UI gestures and avoids
//! interleaving line-delimited frames.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{Context, Result};
use gobcam_protocol::{Command, Response};

pub(crate) struct IpcClient {
    socket: PathBuf,
    state: Mutex<Option<Connection>>,
}

struct Connection {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl IpcClient {
    pub(crate) const fn new(socket: PathBuf) -> Self {
        Self {
            socket,
            state: Mutex::new(None),
        }
    }

    /// Send a command, await its response. On any I/O error the
    /// underlying stream is dropped so the next call reconnects —
    /// transparently surviving daemon restarts.
    pub(crate) fn send(&self, cmd: &Command) -> Result<Response, String> {
        let mut guard = self.state.lock().expect("ipc state poisoned");
        if guard.is_none() {
            let new = Connection::open(&self.socket).map_err(|e| format!("{e:#}"))?;
            *guard = Some(new);
        }
        let conn = guard.as_mut().expect("connection just established");
        let result = conn.exchange(cmd);
        if result.is_err() {
            *guard = None;
        }
        drop(guard);
        result.map_err(|e| format!("{e:#}"))
    }

    /// Drop any cached connection so the next `send` reconnects. Used
    /// when the UI deliberately respawns the daemon (settings change).
    pub(crate) fn reset(&self) {
        *self.state.lock().expect("ipc state poisoned") = None;
    }
}

impl Connection {
    fn open(socket: &Path) -> Result<Self> {
        let stream = UnixStream::connect(socket)
            .with_context(|| format!("connecting to {}", socket.display()))?;
        let reader = BufReader::new(stream.try_clone().context("cloning unix stream")?);
        Ok(Self {
            reader,
            writer: stream,
        })
    }

    fn exchange(&mut self, cmd: &Command) -> Result<Response> {
        let mut line = serde_json::to_vec(cmd).context("encoding command")?;
        line.push(b'\n');
        self.writer.write_all(&line).context("writing command")?;
        let mut response_line = String::new();
        let n = self
            .reader
            .read_line(&mut response_line)
            .context("reading response")?;
        if n == 0 {
            anyhow::bail!("daemon closed the connection");
        }
        serde_json::from_str(response_line.trim_end()).context("parsing response")
    }
}
