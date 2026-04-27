//! Locate the bundled `gobcam-setup` script and exec it. The script
//! self-elevates via `pkexec` (or `sudo`).

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use tauri::{AppHandle, Manager};

/// Look in Tauri's resource dir first (production), then walk up from
/// `current_exe()` for `scripts/gobcam-setup` (dev mode).
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

/// Run the script. Blocks until the polkit dialog is closed.
///
/// The script is staged outside the squashfs first because the
/// `AppImage` FUSE mount is user-only by default, so `pkexec`'d root
/// can't read it from the bundled location.
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
    // `$XDG_RUNTIME_DIR` (mode 0700, owned by us) keeps the staged
    // script per-user-private; root has unrestricted FS access anyway.
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
