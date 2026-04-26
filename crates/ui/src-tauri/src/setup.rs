//! First-run setup glue: locate the bundled `gobcam-setup` script
//! and exec it.
//!
//! The script self-elevates via `pkexec` (graphical password
//! prompt), so the UI can drive the install flow even from inside
//! an `AppImage` where the package manager couldn't drop `/etc/`
//! files at install time. Same script the `.deb` postinst would
//! have run on a Debian host.

use std::path::{Path, PathBuf};
use std::process::Command;

use tauri::{AppHandle, Manager};

/// Resolve the bundled `gobcam-setup` script.
///
/// Looks in this order:
///   1. Tauri's resource directory (production: `usr/lib/Gobcam/`
///      inside `.deb`, `AppImage`'s mounted squashfs at runtime).
///   2. Workspace fallback for `cargo run`/`tauri dev` — walk up
///      from `current_exe()` looking for `scripts/gobcam-setup`.
pub(crate) fn locate(app: &AppHandle) -> Option<PathBuf> {
    if let Ok(dir) = app.path().resource_dir() {
        let candidate = dir.join("gobcam-setup");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    std::env::current_exe().ok()?.ancestors().find_map(|p| {
        let candidate = p.join("scripts").join("gobcam-setup");
        candidate.is_file().then_some(candidate)
    })
}

/// Run the script. It self-elevates via `pkexec` (or `sudo` if
/// `pkexec` isn't on PATH), so we just invoke it as the current
/// user and surface stderr verbatim on failure. Blocks the calling
/// thread until the user closes the polkit dialog.
pub(crate) fn run(script: &Path) -> Result<(), String> {
    let out = Command::new("bash")
        .arg(script)
        .output()
        .map_err(|e| format!("invoking gobcam-setup: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let trimmed = stderr.trim();
    if trimmed.is_empty() {
        Err(format!(
            "gobcam-setup exited with {} (no stderr — pkexec dialog \
             may have been cancelled)",
            out.status
        ))
    } else {
        Err(trimmed.to_string())
    }
}
