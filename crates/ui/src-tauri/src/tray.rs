//! System-tray icon plus close-to-hide.
//!
//! Window close hides the panel instead of quitting; only the tray
//! "Quit" entry (or an external signal) ends the process. This keeps
//! the global hotkey alive after the user dismisses the panel.

use tauri::{
    AppHandle, Manager, WindowEvent,
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};

const MAIN_WINDOW_LABEL: &str = "main";

pub(crate) fn install(app: &AppHandle) -> tauri::Result<()> {
    install_close_to_hide(app);
    install_tray(app)?;
    // Set the WM type hint before the first map so tiling WMs that
    // honour `_NET_WM_WINDOW_TYPE_UTILITY` float the window automatically.
    // The window starts `visible: false` in tauri.conf.json so we own
    // the first show.
    #[cfg(target_os = "linux")]
    set_window_type_hint(app);
    if let Some(win) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        let _ = win.show();
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn set_window_type_hint(app: &AppHandle) {
    use gtk::prelude::GtkWindowExt;
    let Some(win) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        return;
    };
    match win.gtk_window() {
        Ok(gtk_win) => gtk_win.set_type_hint(gtk::gdk::WindowTypeHint::Utility),
        Err(e) => tracing::warn!(error = %e, "could not set GTK window type hint"),
    }
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
            // Linux DEs disagree on left-click semantics; we handle
            // toggling ourselves with `show_menu_on_left_click(false)`.
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
