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
mod config;
mod daemon;
mod hotkeys;
mod ipc;
mod loopback;
mod prefs;
mod setup;
mod tray;

use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::{Context, Result};
use clap::Parser;

use crate::daemon::DaemonArgs;
use crate::ipc::IpcClient;
use crate::prefs::UiPrefs;

/// Bag of state the `switch_input` Tauri command needs to respawn
/// the daemon. Wrapped in a `Mutex` and managed by Tauri.
pub(crate) struct DaemonSupervisor {
    pub socket: PathBuf,
    pub args: DaemonArgs,
    pub guard: Option<daemon::DaemonGuard>,
    /// True when startup found the loopback device missing — the UI
    /// surfaces a first-run setup pane that calls `run_setup` instead
    /// of trying to use the daemon. Cleared after a successful
    /// `run_setup` respawn.
    pub setup_required: bool,
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

    let stored = config::load();
    tracing::info!(?stored, "loaded persisted settings");
    let initial_prefs = UiPrefs::from_stored(&stored);
    let mut args = DaemonArgs::from(stored);
    // First-run / post-uninstall: the loopback device hasn't been
    // created yet. Don't even try to spawn the daemon — the
    // `firewall::install` probe would fail with "Element failed to
    // change its state" and the UI would just exit. Surface a
    // setup pane instead and let the user invoke `gobcam-setup`
    // from there (works inside the AppImage too).
    let setup_required = !cli.no_spawn_daemon && !args.output.exists();
    let daemon_guard = if cli.no_spawn_daemon || setup_required {
        if setup_required {
            tracing::warn!(
                output = %args.output.display(),
                "loopback device missing; UI will prompt for setup"
            );
        }
        None
    } else {
        match daemon::spawn_or_attach(&socket, &args) {
            Ok(g) => g,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "spawn with persisted settings failed; retrying with defaults"
                );
                args = DaemonArgs::default();
                daemon::spawn_or_attach(&socket, &args).unwrap_or_else(|e2| {
                    // Fall through to the UI even if both attempts
                    // failed — the panel can still render its
                    // settings drawer / setup pane and let the user
                    // recover instead of crashing the process.
                    tracing::error!(
                        error = %e2,
                        "daemon spawn failed for both persisted + default settings; \
                         UI will start without it"
                    );
                    None
                })
            }
        }
    };
    let supervisor = DaemonSupervisor {
        socket: socket.clone(),
        args,
        guard: daemon_guard,
        setup_required,
    };
    let client = IpcClient::new(socket);

    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(client)
        .manage(Mutex::new(supervisor))
        .manage(Mutex::new(initial_prefs.clone()))
        .setup(move |app| {
            // Tray + close-to-hide. Without these, the global hotkeys
            // would lose their host the moment the user dismissed the
            // panel via the window-manager's X button.
            tray::install(app.handle())?;

            // Re-register the persisted hotkeys. If parsing fails for
            // either binding, log it but keep the app running — the
            // user can fix the binding in the Settings drawer.
            if let Err(e) = hotkeys::apply(
                app.handle(),
                initial_prefs.hotkey_toggle.as_deref(),
                initial_prefs.hotkey_repeat.as_deref(),
            ) {
                tracing::warn!(error = %e, "registering persisted hotkeys");
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::trigger,
            commands::list_emoji,
            commands::sync_status,
            commands::list_inputs,
            commands::apply_settings,
            commands::preview_path,
            commands::current_settings,
            commands::setup_status,
            commands::run_setup,
            commands::list_recents,
            commands::current_hotkeys,
            commands::set_hotkeys,
            commands::quit_app,
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
