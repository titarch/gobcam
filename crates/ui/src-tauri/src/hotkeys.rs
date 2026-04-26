//! Global-shortcut registration for the panel-toggle and
//! repeat-last-emoji hotkeys.
//!
//! Both bindings are optional; an unset binding registers nothing.
//! Re-registration is wholesale — `unregister_all` then a fresh
//! `on_shortcut` per binding — because the plugin doesn't expose a
//! "replace this one" primitive and our binding count is tiny (≤ 2).
//!
//! Platform note: `tauri-plugin-global-shortcut` uses the
//! `global-hotkey` crate, which works on X11 and Wayland-via-portal.
//! Tiling WMs (Sway, Hyprland) may intercept the chord first if it
//! matches a WM binding — that's a user-side conflict, not something
//! we can detect. Registration *failures* (e.g. another app already
//! holds the chord) bubble up as `Err` strings so the UI can surface
//! them in the Settings drawer.

use std::sync::Mutex;

use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::DaemonSupervisor;
use crate::ipc::IpcClient;
use crate::prefs::UiPrefs;
use crate::tray;

/// Re-register both hotkeys from the current `UiPrefs`. Caller must
/// hold no locks — the global-shortcut handler will lock `UiPrefs`
/// + `DaemonSupervisor` when it fires, so we avoid nesting.
pub(crate) fn apply(
    app: &AppHandle,
    toggle: Option<&str>,
    repeat: Option<&str>,
) -> Result<(), String> {
    let gs = app.global_shortcut();
    // Wholesale clear: simpler than diffing against the previously
    // registered set, and we only have at most two shortcuts.
    if let Err(e) = gs.unregister_all() {
        // Ignore "not registered" but propagate real errors — the
        // plugin returns the same error type for both, so we just
        // log and continue. A dangling registration would surface
        // via `register` returning AlreadyRegistered.
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
            // Pull the most recent emoji and fire it through the same
            // path the UI uses. No window focus required.
            let prefs = app.state::<Mutex<UiPrefs>>();
            let last = prefs.lock().ok().and_then(|p| p.last().map(str::to_string));
            let Some(id) = last else {
                tracing::info!("repeat-last hotkey fired but recents is empty");
                return;
            };
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
