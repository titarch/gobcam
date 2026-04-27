//! Gobcam Tauri shell. Resolves the daemon socket, hands a lazy
//! [`IpcClient`] to the webview, and supervises the `gobcam-pipeline`
//! daemon for the UI's lifetime.

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
use gobcam_protocol::AnimationConfig;

use crate::daemon::DaemonArgs;
use crate::ipc::IpcClient;
use crate::prefs::UiPrefs;

pub(crate) struct DaemonSupervisor {
    pub socket: PathBuf,
    pub args: DaemonArgs,
    pub guard: Option<daemon::DaemonGuard>,
    /// Set when the loopback device is missing; UI shows the setup pane.
    pub setup_required: bool,
    /// UI is the source of truth; pushed to the daemon on startup, on
    /// every change, and re-pushed on respawn.
    pub animations: AnimationConfig,
}

#[derive(Parser, Debug, Clone)]
#[command(version, about)]
struct Cli {
    /// Path to the daemon's Unix socket. Defaults to
    /// `$XDG_RUNTIME_DIR/gobcam.sock`.
    #[arg(long, env = "GOBCAM_SOCKET")]
    socket: Option<PathBuf>,
    /// Attach to an existing daemon instead of spawning one.
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
    let initial_animations = stored.animations.clone();
    let mut args = DaemonArgs::from(stored);
    // First-run / post-uninstall: skip daemon spawn so the UI can show
    // a setup pane instead of failing on a missing loopback device.
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
                    // Keep the UI alive so the user can recover via the settings drawer.
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
        animations: initial_animations.clone(),
    };
    let client = IpcClient::new(socket);

    if !setup_required
        && let Err(e) = client.send(&gobcam_protocol::Command::SetAnimationConfig {
            config: initial_animations,
        })
    {
        tracing::warn!(error = %e, "initial SetAnimationConfig failed; will retry on first user change");
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(client)
        .manage(Mutex::new(supervisor))
        .manage(Mutex::new(initial_prefs.clone()))
        .setup(move |app| {
            tray::install(app.handle())?;

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
            commands::preview_url,
            commands::current_settings,
            commands::current_animations,
            commands::set_animation_config,
            commands::setup_status,
            commands::run_setup,
            commands::list_recents,
            commands::current_hotkeys,
            commands::set_hotkeys,
            commands::list_favorites,
            commands::toggle_favorite,
            commands::set_color_scheme,
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
