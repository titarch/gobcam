//! Gobcam Tauri shell. Resolves the daemon's Unix socket once at
//! startup, hands a lazily-connected [`IpcClient`] to the webview, and
//! launches a single always-on-top panel window. The actual panel is
//! the Svelte frontend in `../src/`.
//!
//! On startup we also supervise the `GStreamer` daemon
//! (`gobcam-pipeline`): if no daemon is bound to the configured
//! socket, spawn one ourselves and shut it down when the UI exits.
//! See `daemon::spawn_or_attach`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod daemon;
mod ipc;

use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::{Context, Result};
use clap::Parser;

use crate::daemon::DaemonArgs;
use crate::ipc::IpcClient;

/// Bag of state the `switch_input` Tauri command needs to respawn
/// the daemon. Wrapped in a `Mutex` and managed by Tauri.
pub(crate) struct DaemonSupervisor {
    pub socket: PathBuf,
    pub args: DaemonArgs,
    pub guard: Option<daemon::DaemonGuard>,
}

#[derive(Parser, Debug, Clone)]
#[command(version, about)]
struct Cli {
    /// Path to the daemon's Unix socket. Defaults to
    /// `$XDG_RUNTIME_DIR/gobcam.sock`.
    #[arg(long, env = "GOBCAM_SOCKET")]
    socket: Option<PathBuf>,
    /// Skip auto-launch of `gobcam-pipeline`; only attach to a
    /// daemon already bound to `--socket`. Useful when running a
    /// manual daemon under `RUST_LOG=…` or `--profile-log`.
    #[arg(long)]
    no_spawn_daemon: bool,
}

fn main() -> Result<()> {
    init_tracing();
    let cli = Cli::parse();
    let socket = resolve_socket(cli.socket)
        .context("could not resolve a default socket path; pass --socket or set XDG_RUNTIME_DIR")?;
    tracing::info!(path = %socket.display(), "gobcam-ui starting");

    let args = DaemonArgs::default();
    let daemon_guard = if cli.no_spawn_daemon {
        None
    } else {
        daemon::spawn_or_attach(&socket, &args).context("starting daemon")?
    };
    let supervisor = DaemonSupervisor {
        socket: socket.clone(),
        args,
        guard: daemon_guard,
    };
    let client = IpcClient::new(socket);

    tauri::Builder::default()
        .manage(client)
        .manage(Mutex::new(supervisor))
        .invoke_handler(tauri::generate_handler![
            commands::trigger,
            commands::list_emoji,
            commands::sync_status,
            commands::list_inputs,
            commands::apply_settings,
            commands::preview_path,
        ])
        .run(tauri::generate_context!())
        .context("running tauri")
}

fn resolve_socket(explicit: Option<PathBuf>) -> Option<PathBuf> {
    explicit.or_else(|| {
        std::env::var_os("XDG_RUNTIME_DIR").map(|d| PathBuf::from(d).join("gobcam.sock"))
    })
}

fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = fmt().with_env_filter(filter).try_init();
}
