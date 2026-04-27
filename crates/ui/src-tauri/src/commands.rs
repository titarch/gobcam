//! `#[tauri::command]` handlers exposed to the Svelte frontend.

#![allow(clippy::needless_pass_by_value)] // Tauri command handlers take owned IPC payloads.

use std::path::PathBuf;
use std::sync::Mutex;

use gobcam_protocol::{AnimationConfig, Command, EmojiInfo, InputDeviceInfo, Response};
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::DaemonSupervisor;
use crate::config;
use crate::daemon;
use crate::hotkeys;
use crate::ipc::IpcClient;
use crate::loopback;
use crate::prefs::UiPrefs;
use crate::setup;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct SyncStatusInfo {
    pub fetched: u32,
    pub total: u32,
    pub complete: bool,
}

fn send_ok(ipc: &IpcClient, command: Command) -> Result<(), String> {
    match ipc.send(&command)? {
        Response::Ok => Ok(()),
        Response::Error { message } => Err(message),
        other => Err(unexpected_response(other)),
    }
}

fn expect_emoji_list(response: Response) -> Result<Vec<EmojiInfo>, String> {
    match response {
        Response::EmojiList { items } => Ok(items),
        Response::Error { message } => Err(message),
        other => Err(unexpected_response(other)),
    }
}

fn expect_sync_status(response: Response) -> Result<SyncStatusInfo, String> {
    match response {
        Response::SyncStatus {
            fetched,
            total,
            complete,
        } => Ok(SyncStatusInfo {
            fetched,
            total,
            complete,
        }),
        Response::Error { message } => Err(message),
        other => Err(unexpected_response(other)),
    }
}

fn expect_preview_url(response: Response) -> Result<Option<String>, String> {
    match response {
        Response::PreviewUrl { url } => Ok(url),
        Response::Error { message } => Err(message),
        other => Err(unexpected_response(other)),
    }
}

fn expect_input_list(response: Response) -> Result<Vec<InputDeviceInfo>, String> {
    match response {
        Response::InputList { items } => Ok(items),
        Response::Error { message } => Err(message),
        other => Err(unexpected_response(other)),
    }
}

fn unexpected_response(response: Response) -> String {
    format!("unexpected response: {response:?}")
}

fn clone_prefs(prefs: &Mutex<UiPrefs>) -> Result<UiPrefs, String> {
    prefs
        .lock()
        .map_err(|e| format!("prefs poisoned: {e}"))
        .map(|p| p.clone())
}

fn save_config(args: &daemon::DaemonArgs, prefs: &UiPrefs, animations: &AnimationConfig) {
    config::save(&config::StoredConfig::from_state(args, prefs, animations));
}

fn save_current_config(
    supervisor: &Mutex<DaemonSupervisor>,
    prefs: &Mutex<UiPrefs>,
) -> Result<(), String> {
    let prefs_snapshot = clone_prefs(prefs)?;
    let (args_snapshot, animations_snapshot) = {
        let sup = supervisor
            .lock()
            .map_err(|e| format!("supervisor poisoned: {e}"))?;
        (sup.args.clone(), sup.animations.clone())
    };
    save_config(&args_snapshot, &prefs_snapshot, &animations_snapshot);
    Ok(())
}

fn save_with_current_prefs(
    args: &daemon::DaemonArgs,
    animations: &AnimationConfig,
    prefs: &Mutex<UiPrefs>,
) -> Result<(), String> {
    let prefs_snapshot = clone_prefs(prefs)?;
    save_config(args, &prefs_snapshot, animations);
    Ok(())
}

/// Dispatch a `Trigger`, record the emoji into recents, and persist.
/// Shared by the `trigger` command and the repeat-last hotkey.
pub(crate) fn trigger_emoji(
    emoji_id: &str,
    ipc: &IpcClient,
    prefs: &Mutex<UiPrefs>,
    supervisor: &Mutex<DaemonSupervisor>,
) -> Result<(), String> {
    send_ok(
        ipc,
        Command::Trigger {
            emoji_id: emoji_id.to_string(),
        },
    )?;

    let changed = {
        let mut p = prefs.lock().map_err(|e| format!("prefs poisoned: {e}"))?;
        p.record(emoji_id)
    };
    if changed {
        save_current_config(supervisor, prefs)?;
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn trigger(
    emoji_id: String,
    ipc: State<'_, IpcClient>,
    prefs: State<'_, Mutex<UiPrefs>>,
    supervisor: State<'_, Mutex<DaemonSupervisor>>,
) -> Result<(), String> {
    trigger_emoji(&emoji_id, &ipc, &prefs, &supervisor)
}

#[tauri::command]
pub(crate) fn list_emoji(ipc: State<'_, IpcClient>) -> Result<Vec<EmojiInfo>, String> {
    expect_emoji_list(ipc.send(&Command::ListEmoji)?)
}

#[tauri::command]
pub(crate) fn sync_status(ipc: State<'_, IpcClient>) -> Result<SyncStatusInfo, String> {
    expect_sync_status(ipc.send(&Command::SyncStatus)?)
}

/// Localhost MJPEG preview URL, or `None` when the daemon was started
/// without `--preview`.
#[tauri::command]
pub(crate) fn preview_url(ipc: State<'_, IpcClient>) -> Result<Option<String>, String> {
    expect_preview_url(ipc.send(&Command::PreviewUrl)?)
}

#[tauri::command]
pub(crate) fn list_inputs(ipc: State<'_, IpcClient>) -> Result<Vec<InputDeviceInfo>, String> {
    expect_input_list(ipc.send(&Command::ListInputs)?)
}

#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct AppliedSettings {
    pub device: String,
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub preview: bool,
    #[serde(default = "default_slot_count_setting")]
    pub slot_count: usize,
    #[serde(default = "default_slot_dim_setting")]
    pub slot_dim: u32,
}

const fn default_slot_count_setting() -> usize {
    48
}

const fn default_slot_dim_setting() -> u32 {
    256
}

/// Drop the current daemon, mutate the spawn args, and respawn.
#[tauri::command]
pub(crate) fn apply_settings(
    settings: AppliedSettings,
    supervisor: State<'_, Mutex<DaemonSupervisor>>,
    ipc: State<'_, IpcClient>,
    prefs: State<'_, Mutex<UiPrefs>>,
) -> Result<(), String> {
    let new_input = PathBuf::from(&settings.device);
    let (socket, args, animations) = {
        let mut sup = supervisor
            .lock()
            .map_err(|e| format!("supervisor poisoned: {e}"))?;
        let unchanged = sup.args.input == new_input
            && sup.args.width == settings.width
            && sup.args.height == settings.height
            && sup.args.fps_num == settings.fps_num
            && sup.args.fps_den == settings.fps_den
            && sup.args.preview == settings.preview
            && sup.args.slot_count == settings.slot_count
            && sup.args.slot_dim == settings.slot_dim;
        if unchanged {
            return Ok(());
        }
        sup.args.input = new_input;
        sup.args.width = settings.width;
        sup.args.height = settings.height;
        sup.args.fps_num = settings.fps_num;
        sup.args.fps_den = settings.fps_den;
        sup.args.preview = settings.preview;
        sup.args.slot_count = settings.slot_count;
        sup.args.slot_dim = settings.slot_dim;
        // Drop the existing guard first so its `Drop` fully shuts down
        // the previous daemon before we respawn.
        sup.guard = None;
        (sup.socket.clone(), sup.args.clone(), sup.animations.clone())
    };
    ipc.reset();
    let new_guard = match daemon::spawn_or_attach(&socket, &args) {
        Ok(g) => g,
        Err(spawn_err) => {
            // Most common cause of a respawn failure: the loopback is
            // still locked at the previous mode. One module reset + retry.
            let initial_msg = format!("{spawn_err:#}");
            tracing::warn!(
                error = %initial_msg,
                "first respawn failed; attempting v4l2loopback reset"
            );
            match loopback::reset() {
                Ok(()) => {
                    ipc.reset();
                    daemon::spawn_or_attach(&socket, &args)
                        .map_err(|e2| format!("after auto-reset: {e2:#}"))?
                }
                Err(reset_err) => {
                    return Err(format!(
                        "{initial_msg}\n\nAuto-reset also failed: {reset_err:#}"
                    ));
                }
            }
        }
    };
    supervisor
        .lock()
        .map_err(|e| format!("supervisor poisoned: {e}"))?
        .guard = new_guard;
    if let Err(e) = send_ok(
        &ipc,
        Command::SetAnimationConfig {
            config: animations.clone(),
        },
    ) {
        tracing::warn!(error = %e, "re-pushing animation config after respawn failed");
    }
    save_with_current_prefs(&args, &animations, &prefs)
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CurrentSettings {
    pub device: String,
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub preview: bool,
    pub slot_count: usize,
    pub slot_dim: u32,
}

#[tauri::command]
pub(crate) fn current_settings(
    supervisor: State<'_, Mutex<DaemonSupervisor>>,
) -> Result<CurrentSettings, String> {
    let sup = supervisor
        .lock()
        .map_err(|e| format!("supervisor poisoned: {e}"))?;
    Ok(CurrentSettings {
        device: sup.args.input.to_string_lossy().into_owned(),
        width: sup.args.width,
        height: sup.args.height,
        fps_num: sup.args.fps_num,
        fps_den: sup.args.fps_den,
        preview: sup.args.preview,
        slot_count: sup.args.slot_count,
        slot_dim: sup.args.slot_dim,
    })
}

/// Push a new live animation config to the daemon and persist. No
/// daemon respawn — the daemon hot-swaps its snapshot.
#[tauri::command]
pub(crate) fn set_animation_config(
    config: AnimationConfig,
    ipc: State<'_, IpcClient>,
    supervisor: State<'_, Mutex<DaemonSupervisor>>,
    prefs: State<'_, Mutex<UiPrefs>>,
) -> Result<(), String> {
    send_ok(
        &ipc,
        Command::SetAnimationConfig {
            config: config.clone(),
        },
    )?;
    {
        let mut sup = supervisor
            .lock()
            .map_err(|e| format!("supervisor poisoned: {e}"))?;
        sup.animations = config;
    }
    save_current_config(&supervisor, &prefs)
}

#[tauri::command]
pub(crate) fn current_animations(
    supervisor: State<'_, Mutex<DaemonSupervisor>>,
) -> Result<AnimationConfig, String> {
    let sup = supervisor
        .lock()
        .map_err(|e| format!("supervisor poisoned: {e}"))?;
    Ok(sup.animations.clone())
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct PrefsSnapshot {
    pub recents: Vec<String>,
    pub favorites: Vec<String>,
    pub hotkey_toggle: Option<String>,
    pub hotkey_repeat: Option<String>,
    pub color_scheme: String,
    pub safe_mode: bool,
}

#[tauri::command]
pub(crate) fn list_recents(prefs: State<'_, Mutex<UiPrefs>>) -> Result<Vec<String>, String> {
    let p = prefs.lock().map_err(|e| format!("prefs poisoned: {e}"))?;
    Ok(p.recents.clone())
}

#[tauri::command]
pub(crate) fn current_hotkeys(prefs: State<'_, Mutex<UiPrefs>>) -> Result<PrefsSnapshot, String> {
    let p = prefs.lock().map_err(|e| format!("prefs poisoned: {e}"))?;
    Ok(PrefsSnapshot {
        recents: p.recents.clone(),
        favorites: p.favorites.clone(),
        hotkey_toggle: p.hotkey_toggle.clone(),
        hotkey_repeat: p.hotkey_repeat.clone(),
        color_scheme: p.color_scheme.clone(),
        safe_mode: p.safe_mode,
    })
}

#[tauri::command]
pub(crate) fn list_favorites(prefs: State<'_, Mutex<UiPrefs>>) -> Result<Vec<String>, String> {
    let p = prefs.lock().map_err(|e| format!("prefs poisoned: {e}"))?;
    Ok(p.favorites.clone())
}

/// Toggle the favorite state of an emoji. Returns the new `is_favorite` value.
#[tauri::command]
pub(crate) fn toggle_favorite(
    emoji_id: String,
    prefs: State<'_, Mutex<UiPrefs>>,
    supervisor: State<'_, Mutex<DaemonSupervisor>>,
) -> Result<bool, String> {
    let is_favorite = {
        let mut p = prefs.lock().map_err(|e| format!("prefs poisoned: {e}"))?;
        p.toggle_favorite(&emoji_id)
    };
    save_current_config(&supervisor, &prefs)?;
    Ok(is_favorite)
}

#[tauri::command]
pub(crate) fn set_color_scheme(
    scheme: String,
    prefs: State<'_, Mutex<UiPrefs>>,
    supervisor: State<'_, Mutex<DaemonSupervisor>>,
) -> Result<(), String> {
    {
        let mut p = prefs.lock().map_err(|e| format!("prefs poisoned: {e}"))?;
        p.color_scheme = scheme;
    }
    save_current_config(&supervisor, &prefs)
}

#[tauri::command]
pub(crate) fn set_safe_mode(
    enabled: bool,
    prefs: State<'_, Mutex<UiPrefs>>,
    supervisor: State<'_, Mutex<DaemonSupervisor>>,
) -> Result<(), String> {
    {
        let mut p = prefs.lock().map_err(|e| format!("prefs poisoned: {e}"))?;
        p.safe_mode = enabled;
    }
    save_current_config(&supervisor, &prefs)
}

/// Validate, register, and persist the two configurable hotkeys. On
/// any registration failure the previous bindings remain active.
#[tauri::command]
pub(crate) fn set_hotkeys(
    toggle: Option<String>,
    repeat: Option<String>,
    app: AppHandle,
    prefs: State<'_, Mutex<UiPrefs>>,
    supervisor: State<'_, Mutex<DaemonSupervisor>>,
) -> Result<(), String> {
    let trim = |s: Option<String>| {
        s.and_then(|x| {
            let t = x.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        })
    };
    let toggle = trim(toggle);
    let repeat = trim(repeat);

    hotkeys::apply(&app, toggle.as_deref(), repeat.as_deref())?;

    {
        let mut p = prefs.lock().map_err(|e| format!("prefs poisoned: {e}"))?;
        p.hotkey_toggle = toggle;
        p.hotkey_repeat = repeat;
    }
    save_current_config(&supervisor, &prefs)
}

#[tauri::command]
pub(crate) fn quit_app(app: AppHandle) {
    app.exit(0);
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct SetupStatus {
    pub required: bool,
    pub output_path: String,
    pub script_bundled: bool,
}

#[tauri::command]
pub(crate) fn setup_status(
    app: AppHandle,
    supervisor: State<'_, Mutex<DaemonSupervisor>>,
) -> Result<SetupStatus, String> {
    let sup = supervisor
        .lock()
        .map_err(|e| format!("supervisor poisoned: {e}"))?;
    Ok(SetupStatus {
        required: sup.setup_required,
        output_path: sup.args.output.to_string_lossy().into_owned(),
        script_bundled: setup::locate(&app).is_some(),
    })
}

/// Run the bundled `gobcam-setup` (which self-elevates via `pkexec`),
/// then spawn the daemon and clear `setup_required`.
#[tauri::command]
pub(crate) fn run_setup(
    app: AppHandle,
    supervisor: State<'_, Mutex<DaemonSupervisor>>,
    ipc: State<'_, IpcClient>,
) -> Result<(), String> {
    let script = setup::locate(&app)
        .ok_or_else(|| "gobcam-setup not bundled with this build".to_string())?;
    setup::run(&script)?;

    let (socket, args) = {
        let sup = supervisor
            .lock()
            .map_err(|e| format!("supervisor poisoned: {e}"))?;
        if !sup.args.output.exists() {
            return Err(format!(
                "gobcam-setup ran but {} still doesn't exist — \
                 the polkit prompt may have been cancelled, or the \
                 v4l2loopback DKMS module is still building. Try again \
                 in a few seconds, or reboot.",
                sup.args.output.display()
            ));
        }
        (sup.socket.clone(), sup.args.clone())
    };

    ipc.reset();
    let new_guard = daemon::spawn_or_attach(&socket, &args)
        .map_err(|e| format!("setup succeeded but daemon spawn failed: {e:#}"))?;

    {
        let mut sup = supervisor
            .lock()
            .map_err(|e| format!("supervisor poisoned: {e}"))?;
        sup.guard = new_guard;
        sup.setup_required = false;
    }
    Ok(())
}
