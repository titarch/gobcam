//! System-tray icon + window-close-to-tray glue.
//!
//! The tray's left-click toggles window visibility; right-click opens
//! a context menu with **Show / Hide** + **Quit**. Closing the panel
//! window via its window-manager X button is intercepted to *hide*
//! the window instead of exiting — Quit (tray menu or process signal)
//! is the only path that actually terminates.
//!
//! Why hide-on-close: with a global hotkey in play, the user expects
//! the app to keep running in the background after dismissing the
//! panel. Without this, closing the window destroys the daemon
//! supervisor (`DaemonGuard::Drop`) and the tray icon along with it,
//! so the next hotkey press would do nothing.

use tauri::{
    AppHandle, Manager, WindowEvent,
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

const MAIN_WINDOW_LABEL: &str = "main";

/// Build the tray icon and attach the close-to-hide handler. Called
/// once from the Tauri `setup` closure.
pub(crate) fn install(app: &AppHandle) -> tauri::Result<()> {
    install_close_to_hide(app);
    install_tray(app)?;
    Ok(())
}

fn install_tray(app: &AppHandle) -> tauri::Result<()> {
    let show_hide = MenuItem::with_id(app, "show_hide", "Show / Hide", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_hide, &separator, &quit])?;

    let icon: Image<'_> = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| tauri::Error::AssetNotFound("default window icon".into()))?;

    TrayIconBuilder::with_id("main")
        .icon(icon)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("Gobcam")
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show_hide" => toggle_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            // Linux DEs differ in how they treat left-click: GNOME
            // tends to map it to "open menu", KDE to "primary
            // activate". With `show_menu_on_left_click(false)` we
            // get the click event uniformly and toggle ourselves.
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle_main_window(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

fn install_close_to_hide(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return;
    };
    let captured = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = captured.hide();
        }
    });
}

/// Public so [`hotkeys::apply`] can call it from the global-shortcut
/// callback without recreating the toggle logic.
pub(crate) fn toggle_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return;
    };
    let visible = window.is_visible().unwrap_or(false);
    if visible {
        let _ = window.hide();
    } else {
        let _ = window.show();
        let _ = window.set_focus();
    }
}
