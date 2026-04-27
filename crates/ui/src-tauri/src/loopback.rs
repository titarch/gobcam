//! Recovery path for `apply_settings`: `rmmod` + `modprobe` the
//! v4l2loopback kernel module via passwordless sudo when the loopback
//! is locked at the previous mode.
//!
//! Requires the sudoers drop-in installed by `scripts/gobcam-setup`.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};
use tracing::info;

const VIDEO_NR: &str = "10";
const CARD_LABEL: &str = "Gobcam";

pub(crate) fn reset() -> Result<()> {
    info!("attempting v4l2loopback reset via sudo");
    let rmmod_path = locate("rmmod");
    let rm = Command::new("sudo")
        .arg("-n")
        .arg(rmmod_path)
        .arg("v4l2loopback")
        .output()
        .context("invoking sudo rmmod")?;
    if !rm.status.success() {
        let stderr = String::from_utf8_lossy(&rm.stderr);
        let lower = stderr.to_lowercase();
        if lower.contains("password") {
            bail!(
                "auto-reset needs passwordless sudo. Run `just install-loopback` to install \
                 the sudoers drop-in (one-time)."
            );
        }
        if lower.contains("in use") || lower.contains("resource busy") {
            bail!(
                "v4l2loopback is in use by another process. Close any active video consumers \
                 (Teams, view-loopback, etc.) and try again."
            );
        }
        if lower.contains("is not currently loaded") {
            // Nothing to remove; fall through to modprobe.
        } else {
            bail!("rmmod v4l2loopback failed: {}", stderr.trim());
        }
    }

    let modprobe_path = locate("modprobe");
    let mp = Command::new("sudo")
        .arg("-n")
        .arg(modprobe_path)
        .arg("v4l2loopback")
        .arg("devices=1")
        .arg(format!("video_nr={VIDEO_NR}"))
        .arg(format!("card_label={CARD_LABEL}"))
        .arg("exclusive_caps=1")
        .output()
        .context("invoking sudo modprobe")?;
    if !mp.status.success() {
        let stderr = String::from_utf8_lossy(&mp.stderr);
        bail!("modprobe v4l2loopback failed: {}", stderr.trim());
    }
    info!("v4l2loopback reset complete");
    Ok(())
}

/// Resolve `name` to an absolute path matching the sudoers drop-in.
fn locate(name: &str) -> String {
    for prefix in ["/usr/bin", "/usr/sbin", "/sbin"] {
        let candidate = format!("{prefix}/{name}");
        if Path::new(&candidate).is_file() {
            return candidate;
        }
    }
    format!("/usr/bin/{name}")
}
