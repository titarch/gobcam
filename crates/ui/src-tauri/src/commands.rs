//! `#[tauri::command]` handlers exposed to the Svelte frontend.
//!
//! Every handler maps onto a `gobcam_protocol::Command`, dispatches it
//! through the shared [`IpcClient`], and converts the daemon's
//! `Response::Error.message` into a JS-side rejection.

use gobcam_protocol::{Command, Response};
use tauri::State;

use crate::ipc::IpcClient;

// `State<'_, T>` must be taken by value — Tauri's command macro extracts
// it from the type-keyed managed-state container.
#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
pub(crate) fn trigger(emoji_id: String, ipc: State<'_, IpcClient>) -> Result<(), String> {
    match ipc.send(&Command::Trigger { emoji_id })? {
        Response::Ok => Ok(()),
        Response::Error { message } => Err(message),
    }
}
