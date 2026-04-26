//! `#[tauri::command]` handlers exposed to the Svelte frontend.
//!
//! Every handler maps onto a `gobcam_protocol::Command`, dispatches it
//! through the shared [`IpcClient`], and converts the daemon's
//! `Response::Error.message` into a JS-side rejection.

use std::path::PathBuf;
use std::sync::Mutex;

use gobcam_protocol::{Command, EmojiInfo, InputDeviceInfo, Response};
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

/// Dispatch a `Trigger` and, on success, push the emoji onto the
/// recents list and persist. Shared between the Tauri `trigger`
/// command (called from the UI button) and the global-shortcut
/// "repeat last emoji" callback so both code paths record the same
/// way.
pub(crate) fn trigger_emoji(
    emoji_id: &str,
    ipc: &IpcClient,
    prefs: &Mutex<UiPrefs>,
    supervisor: &Mutex<DaemonSupervisor>,
) -> Result<(), String> {
    match ipc.send(&Command::Trigger {
        emoji_id: emoji_id.to_string(),
    })? {
        Response::Ok => {
            let prefs_snapshot = {
                let mut p = prefs.lock().map_err(|e| format!("prefs poisoned: {e}"))?;
                if !p.record(emoji_id) {
                    // No-op recordings (re-firing the most recent emoji
                    // back-to-back) skip the disk write entirely — the
                    // common case for the repeat-last hotkey.
                    return Ok(());
                }
                p.clone()
            };
            let args_snapshot = {
                let sup = supervisor
                    .lock()
                    .map_err(|e| format!("supervisor poisoned: {e}"))?;
                sup.args.clone()
            };
            config::save(&config::StoredConfig::from_state(
                &args_snapshot,
                &prefs_snapshot,
            ));
            Ok(())
        }
        Response::Error { message } => Err(message),
        other => Err(format!("unexpected response: {other:?}")),
    }
}

#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub(crate) fn trigger(
    emoji_id: String,
    ipc: State<'_, IpcClient>,
    prefs: State<'_, Mutex<UiPrefs>>,
    supervisor: State<'_, Mutex<DaemonSupervisor>>,
) -> Result<(), String> {
    trigger_emoji(&emoji_id, &ipc, &prefs, &supervisor)
}

#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub(crate) fn list_emoji(ipc: State<'_, IpcClient>) -> Result<Vec<EmojiInfo>, String> {
    match ipc.send(&Command::ListEmoji)? {
        Response::EmojiList { items } => Ok(items),
        Response::Error { message } => Err(message),
        other => Err(format!("unexpected response: {other:?}")),
    }
}

#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub(crate) fn sync_status(ipc: State<'_, IpcClient>) -> Result<SyncStatusInfo, String> {
    match ipc.send(&Command::SyncStatus)? {
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
        other => Err(format!("unexpected response: {other:?}")),
    }
}

/// Absolute path the daemon's preview branch writes to. UI uses
/// `convertFileSrc` to display it. Mirrors the daemon's `CacheRoot`
/// resolution so the two stay in sync without an extra IPC.
#[tauri::command]
pub(crate) fn preview_path() -> Result<String, String> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .ok_or_else(|| "neither XDG_CACHE_HOME nor HOME is set".to_string())?;
    Ok(base
        .join("gobcam")
        .join("runtime-preview.jpg")
        .to_string_lossy()
        .into_owned())
}

#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub(crate) fn list_inputs(ipc: State<'_, IpcClient>) -> Result<Vec<InputDeviceInfo>, String> {
    match ipc.send(&Command::ListInputs)? {
        Response::InputList { items } => Ok(items),
        Response::Error { message } => Err(message),
        other => Err(format!("unexpected response: {other:?}")),
    }
}

/// Settings the user applies from the UI's settings drawer. All
/// fields are required so the UI's logic stays simple — picking a
/// new device sends the device's first mode, picking a new mode
/// keeps the current device.
#[derive(Debug, Clone, serde::Deserialize)]
pub(crate) struct AppliedSettings {
    pub device: String,
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub preview: bool,
}

/// Drop the current daemon, mutate the spawn args, and respawn with
/// the new device + mode. The IPC client cache is reset so the next
/// request reconnects to the new daemon.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub(crate) fn apply_settings(
    settings: AppliedSettings,
    supervisor: State<'_, Mutex<DaemonSupervisor>>,
    ipc: State<'_, IpcClient>,
    prefs: State<'_, Mutex<UiPrefs>>,
) -> Result<(), String> {
    let new_input = PathBuf::from(&settings.device);
    let (socket, args) = {
        let mut sup = supervisor
            .lock()
            .map_err(|e| format!("supervisor poisoned: {e}"))?;
        let unchanged = sup.args.input == new_input
            && sup.args.width == settings.width
            && sup.args.height == settings.height
            && sup.args.fps_num == settings.fps_num
            && sup.args.fps_den == settings.fps_den
            && sup.args.preview == settings.preview;
        if unchanged {
            return Ok(());
        }
        sup.args.input = new_input;
        sup.args.width = settings.width;
        sup.args.height = settings.height;
        sup.args.fps_num = settings.fps_num;
        sup.args.fps_den = settings.fps_den;
        sup.args.preview = settings.preview;
        // Drop the existing guard first — its `Drop` closes stdin and
        // waits for the daemon to exit cleanly before we respawn.
        sup.guard = None;
        (sup.socket.clone(), sup.args.clone())
    };
    ipc.reset();
    let new_guard = match daemon::spawn_or_attach(&socket, &args) {
        Ok(g) => g,
        Err(spawn_err) => {
            // Most likely cause when respawn fails post-bind: the
            // loopback is locked at the previous mode. Try a
            // module reset and retry once.
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
    let prefs_snapshot = prefs
        .lock()
        .map_err(|e| format!("prefs poisoned: {e}"))?
        .clone();
    config::save(&config::StoredConfig::from_state(&args, &prefs_snapshot));
    Ok(())
}

/// Snapshot of the current `DaemonSupervisor` args so the UI can
/// hydrate its dropdowns with the persisted (or fallback) values
/// instead of always defaulting to the first device.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct CurrentSettings {
    pub device: String,
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub preview: bool,
}

#[allow(clippy::needless_pass_by_value)]
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
    })
}

/// Returned by `list_recents` / `current_hotkeys`: a snapshot of the
/// in-memory `UiPrefs` for the frontend.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct PrefsSnapshot {
    pub recents: Vec<String>,
    pub hotkey_toggle: Option<String>,
    pub hotkey_repeat: Option<String>,
}

#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub(crate) fn list_recents(prefs: State<'_, Mutex<UiPrefs>>) -> Result<Vec<String>, String> {
    let p = prefs.lock().map_err(|e| format!("prefs poisoned: {e}"))?;
    Ok(p.recents.clone())
}

#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub(crate) fn current_hotkeys(prefs: State<'_, Mutex<UiPrefs>>) -> Result<PrefsSnapshot, String> {
    let p = prefs.lock().map_err(|e| format!("prefs poisoned: {e}"))?;
    Ok(PrefsSnapshot {
        recents: p.recents.clone(),
        hotkey_toggle: p.hotkey_toggle.clone(),
        hotkey_repeat: p.hotkey_repeat.clone(),
    })
}

/// Validate, register, and persist the two configurable hotkeys. On
/// any registration failure the prefs aren't mutated and the previous
/// bindings remain active.
#[allow(clippy::needless_pass_by_value)]
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

    let prefs_snapshot = {
        let mut p = prefs.lock().map_err(|e| format!("prefs poisoned: {e}"))?;
        p.hotkey_toggle = toggle;
        p.hotkey_repeat = repeat;
        p.clone()
    };
    let args_snapshot = supervisor
        .lock()
        .map_err(|e| format!("supervisor poisoned: {e}"))?
        .args
        .clone();
    config::save(&config::StoredConfig::from_state(
        &args_snapshot,
        &prefs_snapshot,
    ));
    Ok(())
}

/// Tray-equivalent quit, callable from the JS layer (e.g. a
/// "quit-to-system-tray" menu inside the panel itself). Returns
/// before the runtime tears down so Tauri can exit cleanly.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub(crate) fn quit_app(app: AppHandle) {
    app.exit(0);
}

/// First-run state surfaced to the UI. `required` is set on startup
/// when the configured loopback device is missing, so the panel can
/// show a setup pane instead of a broken main view. `script_bundled`
/// tells the panel whether `run_setup` will succeed at all (false in
/// dev builds without resources installed).
#[derive(Debug, Clone, Serialize)]
pub(crate) struct SetupStatus {
    pub required: bool,
    pub output_path: String,
    pub script_bundled: bool,
}

#[allow(clippy::needless_pass_by_value)]
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

/// Exec the bundled `gobcam-setup` (which self-elevates via
/// `pkexec`), then — assuming the loopback device is now present —
/// spawn the daemon and clear the `setup_required` flag. The UI
/// polls `setup_status` after this to know when to swap views.
#[allow(clippy::needless_pass_by_value)]
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
