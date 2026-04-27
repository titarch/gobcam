//! Global-shortcut registration for panel-toggle and repeat-last-emoji.
//! Both bindings are optional; an unset binding registers nothing.

use std::sync::Mutex;

use gobcam_protocol::safe_mode;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::DaemonSupervisor;
use crate::ipc::IpcClient;
use crate::prefs::UiPrefs;
use crate::tray;

/// Tauri event name dispatched when a `repeat` shortcut press is
/// suppressed because the most-recent emoji is on the safe-mode
/// denylist. The Svelte side listens and surfaces a toast.
const SAFE_MODE_BLOCKED_EVENT: &str = "safe-mode-blocked-trigger";

/// Re-register both hotkeys. Caller must hold no locks — the handler
/// will lock `UiPrefs` + `DaemonSupervisor` when it fires.
pub(crate) fn apply(
    app: &AppHandle,
    toggle: Option<&str>,
    repeat: Option<&str>,
) -> Result<(), String> {
    let gs = app.global_shortcut();
    if let Err(e) = gs.unregister_all() {
        tracing::warn!(error = %e, "unregister_all (continuing)");
    }

    if let Some(s) = toggle {
        let parsed: Shortcut = s
            .parse()
            .map_err(|e| format!("toggle hotkey '{s}' isn't a valid shortcut: {e}"))?;
        gs.on_shortcut(parsed, |app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                tray::toggle_main_window(app);
            }
        })
        .map_err(|e| format!("registering toggle hotkey '{s}': {e}"))?;
    }

    if let Some(s) = repeat {
        let parsed: Shortcut = s
            .parse()
            .map_err(|e| format!("repeat hotkey '{s}' isn't a valid shortcut: {e}"))?;
        gs.on_shortcut(parsed, |app, _shortcut, event| {
            if event.state() != ShortcutState::Pressed {
                return;
            }
            let prefs = app.state::<Mutex<UiPrefs>>();
            let (last, safe_mode_on) = prefs.lock().map_or((None, false), |p| {
                (p.last().map(str::to_string), p.safe_mode)
            });
            let Some(id) = last else {
                tracing::info!("repeat-last hotkey fired but recents is empty");
                return;
            };
            if safe_mode_on && safe_mode::is_denied(&id) {
                tracing::info!(emoji = %id, "repeat-last hotkey suppressed by safe mode");
                if let Err(e) = app.emit(SAFE_MODE_BLOCKED_EVENT, id) {
                    tracing::warn!(error = %e, "emitting {SAFE_MODE_BLOCKED_EVENT}");
                }
                return;
            }
            let ipc = app.state::<IpcClient>();
            let supervisor = app.state::<Mutex<DaemonSupervisor>>();
            if let Err(e) = crate::commands::trigger_emoji(&id, &ipc, &prefs, &supervisor) {
                tracing::warn!(emoji = %id, error = %e, "repeat-last hotkey trigger failed");
            }
        })
        .map_err(|e| format!("registering repeat hotkey '{s}': {e}"))?;
    }

    Ok(())
}
