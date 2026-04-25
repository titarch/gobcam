//! `#[tauri::command]` handlers exposed to the Svelte frontend.
//!
//! Every handler maps onto a `gobcam_protocol::Command`, dispatches it
//! through the shared [`IpcClient`], and converts the daemon's
//! `Response::Error.message` into a JS-side rejection.

use gobcam_protocol::{Command, EmojiInfo, Response};
use serde::Serialize;
use tauri::State;

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
