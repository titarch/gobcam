//! `#[tauri::command]` handlers exposed to the Svelte frontend.
//!
//! Every handler maps onto a `gobcam_protocol::Command`, dispatches it
//! through the shared [`IpcClient`], and converts the daemon's
//! `Response::Error.message` into a JS-side rejection.

use std::path::PathBuf;
use std::sync::Mutex;

use gobcam_protocol::{Command, EmojiInfo, InputDeviceInfo, Response};
use serde::Serialize;
use tauri::State;

use crate::DaemonSupervisor;
use crate::daemon;
use crate::ipc::IpcClient;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct SyncStatusInfo {
    pub fetched: u32,
    pub total: u32,
    pub complete: bool,
}

// `State<'_, T>` must be taken by value — Tauri's command macro extracts
// it from the type-keyed managed-state container.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub(crate) fn trigger(emoji_id: String, ipc: State<'_, IpcClient>) -> Result<(), String> {
    match ipc.send(&Command::Trigger { emoji_id })? {
        Response::Ok => Ok(()),
        Response::Error { message } => Err(message),
        other => Err(format!("unexpected response: {other:?}")),
    }
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
    let new_guard = daemon::spawn_or_attach(&socket, &args).map_err(|e| format!("{e:#}"))?;
    supervisor
        .lock()
        .map_err(|e| format!("supervisor poisoned: {e}"))?
        .guard = new_guard;
    Ok(())
}
