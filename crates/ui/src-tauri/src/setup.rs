//! First-run setup glue: locate the bundled `gobcam-setup` script
//! and exec it.
//!
//! The script self-elevates via `pkexec` (graphical password
//! prompt), so the UI can drive the install flow even from inside
//! an `AppImage` where the package manager couldn't drop `/etc/`
//! files at install time. Same script the `.deb` postinst would
//! have run on a Debian host.

use std::os::unix::fs::PermissionsExt;
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
///
/// Inside an `AppImage` the script lives on a FUSE mount that's
/// configured user-only by default — the `pkexec`'d root-side bash
/// then fails with `Permission denied` trying to re-read the
/// script. To dodge that we copy the script out of any potentially
/// user-only filesystem first; the staged copy lives in
/// `$XDG_RUNTIME_DIR` (mode-0700 dir owned by the user, but root
/// has unrestricted FS access) and is removed when this function
/// returns.
pub(crate) fn run(script: &Path) -> Result<(), String> {
    let staged = stage_for_pkexec(script)
        .map_err(|e| format!("staging gobcam-setup outside the AppImage mount: {e}"))?;
    let _cleanup = StageGuard(staged.path.clone());

    let out = Command::new("bash")
        .arg(&staged.path)
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

struct Staged {
    path: PathBuf,
}

fn stage_for_pkexec(src: &Path) -> std::io::Result<Staged> {
    // Prefer $XDG_RUNTIME_DIR (typically /run/user/$UID, mode 0700,
    // owned by us) over /tmp — both are root-readable, but the
    // runtime dir is per-user-private so the staged script doesn't
    // briefly land in a world-readable location.
    let base = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
    let path = base.join(format!("gobcam-setup.{}.sh", std::process::id()));
    std::fs::copy(src, &path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
    Ok(Staged { path })
}

struct StageGuard(PathBuf);

impl Drop for StageGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
