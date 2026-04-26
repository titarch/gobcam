//! Enumerate v4l2 capture devices via `v4l2-ctl --list-devices`.
//!
//! For each group of devices reported by v4l2-ctl we pick the first
//! `/dev/video*` path — that's reliably the main capture node in
//! practice (the others are typically metadata/IR/depth siblings).
//! The daemon's own loopback output is filtered out by exact path.
//!
//! Per-device modes (resolution + framerate) come from
//! `v4l2-ctl --list-formats-ext`; we filter to raw formats so the
//! daemon's `video/x-raw` capsfilter can negotiate without an extra
//! decode element. MJPEG/H.264 modes are out of scope for now.

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InputDevice {
    pub device: PathBuf,
    pub name: String,
    pub modes: Vec<Mode>,
}

/// Capture mode (resolution + framerate) the device supports in a
/// raw pixel format. Format is intentionally not exposed —
/// `GStreamer` negotiates the best raw format the camera offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct Mode {
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
}

/// List inputs reported by `v4l2-ctl`, excluding `output` (typically
/// the daemon's own loopback). Returns an empty vector if `v4l2-ctl`
/// isn't installed or fails — callers treat that as "no inputs".
/// Also queries each device for supported raw modes.
pub(crate) fn list(output: &Path) -> Vec<InputDevice> {
    let Ok(out) = Command::new("v4l2-ctl").arg("--list-devices").output() else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let mut devices = parse(&String::from_utf8_lossy(&out.stdout), output);
    for dev in &mut devices {
        dev.modes = list_modes(&dev.device);
    }
    devices
}

/// Query supported raw capture modes for `device` via
/// `v4l2-ctl --list-formats-ext -d <device>`.
pub(crate) fn list_modes(device: &Path) -> Vec<Mode> {
    let Ok(out) = Command::new("v4l2-ctl")
        .arg("--list-formats-ext")
        .arg("-d")
        .arg(device)
        .output()
    else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let mut modes = parse_modes(&String::from_utf8_lossy(&out.stdout));
    modes.sort_by(|a, b| {
        // Highest resolution first, then highest framerate.
        b.width.cmp(&a.width).then(b.height.cmp(&a.height)).then(
            (u64::from(b.fps_num) * u64::from(a.fps_den))
                .cmp(&(u64::from(a.fps_num) * u64::from(b.fps_den))),
        )
    });
    modes.dedup();
    modes
}

fn parse_modes(text: &str) -> Vec<Mode> {
    let mut modes = Vec::new();
    let mut current_fmt: Option<String> = None;
    let mut current_size: Option<(u32, u32)> = None;
    for line in text.lines() {
        let trimmed = line.trim();
        // Format header: "[0]: 'YUYV' (YUYV 4:2:2)"
        if let Some(fmt) = trimmed
            .strip_prefix('[')
            .and_then(|s| s.split_once("]: ").map(|(_, after)| after))
            .and_then(|after| after.strip_prefix('\''))
            .and_then(|s| s.split_once('\'').map(|(f, _)| f.to_string()))
        {
            current_fmt = Some(fmt);
            current_size = None;
            continue;
        }
        // Size: "Size: Discrete 1280x720"
        if let Some((w, h)) = trimmed
            .strip_prefix("Size: Discrete ")
            .and_then(|rest| rest.split_once('x'))
            .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)))
        {
            current_size = Some((w, h));
            continue;
        }
        // Interval: "Interval: Discrete 0.033s (30.000 fps)"
        if trimmed.starts_with("Interval: Discrete ") {
            let Some(fmt) = current_fmt.as_deref() else {
                continue;
            };
            if !is_raw_format(fmt) {
                continue;
            }
            let Some((w, h)) = current_size else {
                continue;
            };
            // Extract fps from the "(N.NNN fps)" suffix.
            let Some(open) = trimmed.rfind('(') else {
                continue;
            };
            let Some(stripped) = trimmed[open + 1..].strip_suffix(" fps)") else {
                continue;
            };
            let Ok(fps) = stripped.parse::<f64>() else {
                continue;
            };
            let (fps_num, fps_den) = fps_to_rational(fps);
            modes.push(Mode {
                width: w,
                height: h,
                fps_num,
                fps_den,
            });
        }
    }
    modes
}

/// Whether a v4l2 fourcc denotes a raw pixel format the daemon's
/// `video/x-raw` capsfilter can negotiate without a decoder.
fn is_raw_format(fourcc: &str) -> bool {
    !matches!(fourcc, "MJPG" | "MPEG" | "H264" | "HEVC" | "VP8" | "VP9")
}

fn fps_to_rational(fps: f64) -> (u32, u32) {
    /// Saturate-cast `f64 ≥ 0` to `u32`. v4l2 fps are always
    /// positive small numbers; the saturate is purely defensive.
    fn cast(x: f64) -> u32 {
        if !x.is_finite() || x < 0.0 {
            return 0;
        }
        if x >= f64::from(u32::MAX) {
            return u32::MAX;
        }
        // Truncate after rounding; the range guard above keeps this lossless.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let n = x as u32;
        n
    }
    if fps.fract().abs() < 1e-3 && fps > 0.0 {
        (cast(fps.round()), 1)
    } else {
        (cast((fps * 1000.0).round()), 1000)
    }
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
                    modes: Vec::new(),
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

    const MODES_SAMPLE: &str = "\
ioctl: VIDIOC_ENUM_FMT
\tType: Video Capture

\t[0]: 'YUYV' (YUYV 4:2:2)
\t\tSize: Discrete 1280x720
\t\t\tInterval: Discrete 0.033s (30.000 fps)
\t\t\tInterval: Discrete 0.067s (15.000 fps)
\t\tSize: Discrete 640x480
\t\t\tInterval: Discrete 0.033s (30.000 fps)
\t[1]: 'MJPG' (Motion-JPEG, compressed)
\t\tSize: Discrete 1920x1080
\t\t\tInterval: Discrete 0.033s (30.000 fps)
";

    #[test]
    fn parses_modes_and_filters_mjpg() {
        let modes = parse_modes(MODES_SAMPLE);
        // 720p@30, 720p@15, 480p@30 — MJPG 1080p excluded
        assert_eq!(modes.len(), 3);
        assert!(
            modes
                .iter()
                .any(|m| m.width == 1280 && m.height == 720 && m.fps_num == 30)
        );
        assert!(
            modes
                .iter()
                .any(|m| m.width == 1280 && m.height == 720 && m.fps_num == 15)
        );
        assert!(
            modes
                .iter()
                .any(|m| m.width == 640 && m.height == 480 && m.fps_num == 30)
        );
        assert!(!modes.iter().any(|m| m.width == 1920));
    }

    #[test]
    fn fractional_fps_uses_thousandths() {
        let s = "\t[0]: 'YUYV' (YUYV)\n\t\tSize: Discrete 320x240\n\t\t\tInterval: Discrete 0.133s (7.500 fps)\n";
        let modes = parse_modes(s);
        assert_eq!(modes.len(), 1);
        assert_eq!(modes[0].fps_num, 7500);
        assert_eq!(modes[0].fps_den, 1000);
    }
}
