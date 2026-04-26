//! Enumerate v4l2 capture devices via `v4l2-ctl --list-devices`.
//!
//! For each group of devices reported by v4l2-ctl we pick the first
//! `/dev/video*` path — that's reliably the main capture node in
//! practice (the others are typically metadata/IR/depth siblings).
//! The daemon's own loopback output is filtered out by exact path.

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InputDevice {
    pub device: PathBuf,
    pub name: String,
}

/// List inputs reported by `v4l2-ctl`, excluding `output` (typically
/// the daemon's own loopback). Returns an empty vector if `v4l2-ctl`
/// isn't installed or fails — callers treat that as "no inputs".
pub(crate) fn list(output: &Path) -> Vec<InputDevice> {
    let Ok(out) = Command::new("v4l2-ctl").arg("--list-devices").output() else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    parse(&String::from_utf8_lossy(&out.stdout), output)
}

fn parse(text: &str, exclude: &Path) -> Vec<InputDevice> {
    let mut devices = Vec::new();
    let mut current_group: Option<String> = None;
    let mut took_first = false;
    for line in text.lines() {
        if line.is_empty() {
            current_group = None;
            took_first = false;
            continue;
        }
        let is_indented = line.starts_with('\t') || line.starts_with(' ');
        if is_indented {
            if took_first {
                continue;
            }
            let trimmed = line.trim();
            // String-prefix check: `Path::starts_with` would compare
            // *components*, so `/dev/video10` would not match `/dev/video`.
            if !trimmed.starts_with("/dev/video") {
                continue;
            }
            let path = PathBuf::from(trimmed);
            if path == exclude {
                continue;
            }
            if let Some(name) = current_group.as_ref() {
                devices.push(InputDevice {
                    device: path,
                    name: friendly_name(name),
                });
                took_first = true;
            }
        } else {
            current_group = Some(line.trim_end_matches(':').to_string());
            took_first = false;
        }
    }
    devices
}

/// Trim the `(bus-info)` suffix v4l2-ctl appends so the UI shows
/// `"Logitech BRIO"` instead of
/// `"Logitech BRIO (usb-0000:00:14.0-6)"`.
fn friendly_name(raw: &str) -> String {
    raw.rsplit_once('(')
        .map_or_else(|| raw.to_string(), |(prefix, _)| prefix.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
Gobcam (platform:v4l2loopback-010):
\t/dev/video10

Logitech BRIO (usb-0000:00:14.0-6):
\t/dev/video0
\t/dev/video1
\t/dev/video2
\t/dev/video3
\t/dev/media0
";

    #[test]
    fn excludes_loopback_and_picks_first_path_per_group() {
        let out = parse(SAMPLE, Path::new("/dev/video10"));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].device, PathBuf::from("/dev/video0"));
        assert_eq!(out[0].name, "Logitech BRIO");
    }

    #[test]
    fn no_exclusion_returns_both() {
        let out = parse(SAMPLE, Path::new("/dev/video99"));
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].name, "Gobcam");
        assert_eq!(out[1].device, PathBuf::from("/dev/video0"));
    }

    #[test]
    fn skips_non_video_entries() {
        let s = "WebCam:\n\t/dev/video0\n\t/dev/media0\n";
        let out = parse(s, Path::new("/dev/null"));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].device, PathBuf::from("/dev/video0"));
    }
}
