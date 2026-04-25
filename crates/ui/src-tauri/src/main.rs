//! Gobcam Tauri shell. Resolves the daemon's Unix socket once at
//! startup, hands a lazily-connected [`IpcClient`] to the webview, and
//! launches a single always-on-top panel window. The actual panel is
//! the Svelte frontend in `../src/`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod ipc;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;

use crate::ipc::IpcClient;

#[derive(Parser, Debug, Clone)]
#[command(version, about)]
struct Cli {
    /// Path to the daemon's Unix socket. Defaults to
    /// `$XDG_RUNTIME_DIR/gobcam.sock`.
    #[arg(long, env = "GOBCAM_SOCKET")]
    socket: Option<PathBuf>,
}

fn main() -> Result<()> {
    init_tracing();
    let cli = Cli::parse();
    let socket = resolve_socket(cli.socket)
        .context("could not resolve a default socket path; pass --socket or set XDG_RUNTIME_DIR")?;
    tracing::info!(path = %socket.display(), "gobcam-ui starting");
    let client = IpcClient::new(socket);

    tauri::Builder::default()
        .manage(client)
        .invoke_handler(tauri::generate_handler![commands::trigger])
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
