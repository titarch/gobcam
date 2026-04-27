//! Wire types for the gobcam daemon's IPC surface.
//!
//! Transport: a Unix domain socket whose path is configured via
//! `--socket` / `GOBCAM_SOCKET`. The protocol is line-delimited JSON —
//! one [`Command`] per line in, one [`Response`] per line out, in the
//! same order. Connections are short- or long-lived at the client's
//! discretion.
//!
//! ```no_run
//! use gobcam_protocol::{Command, Response};
//! let cmd = Command::Trigger { emoji_id: "fire".into() };
//! let line = serde_json::to_string(&cmd).unwrap();
//! //  ─►  {"type":"trigger","emoji_id":"fire"}
//! let _: Response = serde_json::from_str(r#"{"type":"ok"}"#).unwrap();
//! ```

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Daemon → client reply for a single [`Command`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    /// Command accepted and executed.
    Ok,
    /// Command rejected — `message` is human-readable, not stable.
    Error { message: String },
    /// Reply to [`Command::ListEmoji`] — every catalog entry the
    /// daemon knows about. `preview_path` may not exist on disk yet
    /// during the initial bootstrap; clients should be tolerant.
    EmojiList { items: Vec<EmojiInfo> },
    /// Reply to [`Command::SyncStatus`] — predownload progress.
    /// `complete` is `true` once the daemon has either fetched or
    /// given up on every entry.
    SyncStatus {
        fetched: u32,
        total: u32,
        complete: bool,
    },
    /// Reply to [`Command::ListInputs`] — v4l2 capture devices the
    /// daemon found, excluding its own loopback output.
    InputList { items: Vec<InputDeviceInfo> },
    /// Reply to [`Command::PreviewUrl`]. `None` when the daemon was
    /// started without `--preview`. The URL is a localhost MJPEG
    /// stream the UI can point an `<img>` tag at.
    PreviewUrl { url: Option<String> },
}

/// Client → daemon request.
///
/// `Eq` is intentionally not derived: [`Command::SetAnimationConfig`]
/// carries `f32` fields, which can't satisfy `Eq` (NaN). `PartialEq`
/// is enough for tests and IPC handler dispatch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    /// Fire a reaction with the daemon's default duration. The
    /// `emoji_id` is a key from the daemon's bundled Fluent catalog.
    Trigger { emoji_id: String },
    /// Enumerate every emoji in the catalog (~1500 entries).
    ListEmoji,
    /// Query the preview-bootstrap downloader's progress.
    SyncStatus,
    /// Enumerate v4l2 capture devices available on the host.
    ListInputs,
    /// Replace the daemon's live animation parameters. Applies to
    /// every subsequent [`Command::Trigger`]; reactions already
    /// in flight keep their original curves. No respawn required.
    SetAnimationConfig { config: AnimationConfig },
    /// Ask the daemon for its preview MJPEG URL. The reply's `url`
    /// is `None` if the daemon was started without `--preview`.
    PreviewUrl,
}

/// What to do when a [`Command::Trigger`] arrives but
/// [`AnimationConfig::max_concurrent`] is already saturated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DropPolicy {
    /// Silently drop the new trigger. Existing cascade keeps its
    /// pacing — once a reaction completes its lifetime the slot
    /// frees and new triggers land again.
    #[default]
    DropNew,
    /// Force-fade the oldest active reaction and reuse its slot for
    /// the new trigger. More responsive to spam, can look glitchy.
    DropOldest,
}

/// Tunable parameters for the cascading-emoji animation engine.
/// Per-emoji overrides live in [`AnimationConfig::overrides`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimationConfig {
    /// On-screen lifetime in milliseconds (before the per-instance
    /// `speed_factor` random multiplier is applied).
    pub lifetime_ms: u32,
    /// Fade-in ramp duration.
    pub fade_in_ms: u32,
    /// Time after activation when fade-out starts (relative to the
    /// reaction's effective lifetime; pre-jitter).
    pub fade_out_start_ms: u32,
    /// Fade-out ramp duration.
    pub fade_out_ms: u32,
    /// Distance traveled along `direction_angle_deg` over the
    /// reaction's lifetime, in canvas pixels.
    pub travel_px: f32,
    /// Per-instance speed jitter as a fraction of `lifetime_ms`. A
    /// value of 0.25 means each instance's lifetime is sampled
    /// uniformly from [0.75×, 1.25×].
    pub speed_jitter_pct: f32,
    /// Horizontal anchor of the spawn band as a fraction of canvas
    /// width (0.0 = left edge, 1.0 = right edge).
    pub start_x_fraction: f32,
    /// Vertical offset above the bottom edge where reactions spawn,
    /// in canvas pixels.
    pub start_y_offset_px: f32,
    /// Half-width of the horizontal random band around the anchor.
    pub x_jitter_px: f32,
    /// Direction of travel in degrees; 90.0 is straight up.
    pub direction_angle_deg: f32,
    /// APNG playback-rate multiplier. 1.0 = native delays. Clamped
    /// to `[0.1, 5.0]` daemon-side.
    pub apng_speed_multiplier: f32,
    /// Cap on simultaneously-visible reactions. Clamped to `<= slot_count`.
    pub max_concurrent: u32,
    /// What happens when `max_concurrent` is already reached.
    pub drop_policy: DropPolicy,
    /// Per-emoji overrides on top of the global values.
    #[serde(default)]
    pub overrides: BTreeMap<String, AnimationOverrides>,
}

impl Default for AnimationConfig {
    fn default() -> Self {
        Self {
            lifetime_ms: 5000,
            fade_in_ms: 200,
            fade_out_start_ms: 3000,
            fade_out_ms: 2000,
            travel_px: 480.0,
            speed_jitter_pct: 0.25,
            start_x_fraction: 0.5,
            start_y_offset_px: 80.0,
            x_jitter_px: 220.0,
            direction_angle_deg: 90.0,
            apng_speed_multiplier: 1.0,
            max_concurrent: 32,
            drop_policy: DropPolicy::DropNew,
            overrides: BTreeMap::new(),
        }
    }
}

/// Per-emoji partial overrides on top of the global [`AnimationConfig`].
/// `None` means "use the global value".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct AnimationOverrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifetime_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_in_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_out_start_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_out_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub travel_px: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_jitter_pct: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_x_fraction: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_y_offset_px: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x_jitter_px: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction_angle_deg: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub apng_speed_multiplier: Option<f32>,
}

/// One v4l2 capture device. The daemon is re-launched against `device`
/// when the user switches inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputDeviceInfo {
    /// `/dev/videoN` path.
    pub device: PathBuf,
    /// Human-friendly label (e.g. `"Logitech BRIO"`).
    pub name: String,
    /// Supported raw capture modes, sorted highest first.
    pub modes: Vec<Mode>,
}

/// One supported capture mode: resolution + framerate as a rational.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mode {
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
}

/// One entry of the bundled Fluent catalog as the UI sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmojiInfo {
    /// Stable, `snake_case` id used by [`Command::Trigger`].
    pub id: String,
    /// Upstream display name.
    pub name: String,
    /// Unicode glyph for fallback rendering before the preview downloads.
    pub glyph: String,
    /// Unicode group for sectioning.
    pub group: String,
    /// Free-text keywords for client-side search.
    pub keywords: Vec<String>,
    /// `true` if an APNG exists; otherwise the static image is used as
    /// a single-frame loop.
    pub has_animated: bool,
    /// Where the daemon expects the static preview to live after
    /// bootstrap. May not exist yet when [`Command::ListEmoji`] is answered.
    pub preview_path: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_round_trip() {
        let cmd = Command::Trigger {
            emoji_id: "fire".into(),
        };
        let line = serde_json::to_string(&cmd).unwrap();
        assert_eq!(line, r#"{"type":"trigger","emoji_id":"fire"}"#);
        let back: Command = serde_json::from_str(&line).unwrap();
        assert_eq!(back, cmd);
    }

    #[test]
    fn list_emoji_serializes_compactly() {
        assert_eq!(
            serde_json::to_string(&Command::ListEmoji).unwrap(),
            r#"{"type":"list_emoji"}"#
        );
    }

    #[test]
    fn sync_status_command_round_trip() {
        let line = serde_json::to_string(&Command::SyncStatus).unwrap();
        assert_eq!(line, r#"{"type":"sync_status"}"#);
        let back: Command = serde_json::from_str(&line).unwrap();
        assert_eq!(back, Command::SyncStatus);
    }

    #[test]
    fn response_ok_serializes_compactly() {
        let line = serde_json::to_string(&Response::Ok).unwrap();
        assert_eq!(line, r#"{"type":"ok"}"#);
    }

    #[test]
    fn response_error_round_trip() {
        let r = Response::Error {
            message: "all 4 slots busy".into(),
        };
        let line = serde_json::to_string(&r).unwrap();
        let back: Response = serde_json::from_str(&line).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn emoji_list_round_trip() {
        let r = Response::EmojiList {
            items: vec![EmojiInfo {
                id: "fire".into(),
                name: "Fire".into(),
                glyph: "🔥".into(),
                group: "Travel & Places".into(),
                keywords: vec!["fire".into(), "flame".into()],
                has_animated: true,
                preview_path: PathBuf::from("/home/u/.cache/gobcam/previews/fire.png"),
            }],
        };
        let line = serde_json::to_string(&r).unwrap();
        let back: Response = serde_json::from_str(&line).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn sync_status_response_round_trip() {
        let r = Response::SyncStatus {
            fetched: 42,
            total: 1500,
            complete: false,
        };
        let line = serde_json::to_string(&r).unwrap();
        assert_eq!(
            line,
            r#"{"type":"sync_status","fetched":42,"total":1500,"complete":false}"#
        );
        let back: Response = serde_json::from_str(&line).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn input_list_round_trip() {
        let r = Response::InputList {
            items: vec![InputDeviceInfo {
                device: PathBuf::from("/dev/video0"),
                name: "Logitech BRIO".into(),
                modes: vec![Mode {
                    width: 1280,
                    height: 720,
                    fps_num: 30,
                    fps_den: 1,
                }],
            }],
        };
        let line = serde_json::to_string(&r).unwrap();
        let back: Response = serde_json::from_str(&line).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn animation_config_default_round_trip() {
        let cfg = AnimationConfig::default();
        let cmd = Command::SetAnimationConfig {
            config: cfg.clone(),
        };
        let line = serde_json::to_string(&cmd).unwrap();
        let back: Command = serde_json::from_str(&line).unwrap();
        let Command::SetAnimationConfig { config: back_cfg } = back else {
            panic!("variant lost in round-trip");
        };
        assert_eq!(back_cfg, cfg);
    }

    #[test]
    fn drop_policy_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&DropPolicy::DropNew).unwrap(),
            r#""drop_new""#
        );
        assert_eq!(
            serde_json::to_string(&DropPolicy::DropOldest).unwrap(),
            r#""drop_oldest""#
        );
    }

    #[test]
    fn animation_overrides_round_trip_only_set_fields() {
        let ov = AnimationOverrides {
            lifetime_ms: Some(8000),
            travel_px: Some(640.0),
            ..Default::default()
        };
        let line = serde_json::to_string(&ov).unwrap();
        assert_eq!(line, r#"{"lifetime_ms":8000,"travel_px":640.0}"#);
        let back: AnimationOverrides = serde_json::from_str(&line).unwrap();
        assert_eq!(back, ov);
    }

    #[test]
    fn animation_config_accepts_missing_overrides() {
        let line = r#"{
            "lifetime_ms": 5000, "fade_in_ms": 200, "fade_out_start_ms": 3000,
            "fade_out_ms": 2000, "travel_px": 480.0, "speed_jitter_pct": 0.25,
            "start_x_fraction": 0.5, "start_y_offset_px": 80.0,
            "x_jitter_px": 220.0, "direction_angle_deg": 90.0,
            "apng_speed_multiplier": 1.0, "max_concurrent": 32,
            "drop_policy": "drop_new"
        }"#;
        let cfg: AnimationConfig = serde_json::from_str(line).unwrap();
        assert!(cfg.overrides.is_empty());
    }

    #[test]
    fn preview_url_round_trip() {
        let cmd_line = serde_json::to_string(&Command::PreviewUrl).unwrap();
        assert_eq!(cmd_line, r#"{"type":"preview_url"}"#);
        let back: Command = serde_json::from_str(&cmd_line).unwrap();
        assert_eq!(back, Command::PreviewUrl);

        let r = Response::PreviewUrl {
            url: Some("http://127.0.0.1:34567/preview.mjpg".into()),
        };
        let line = serde_json::to_string(&r).unwrap();
        let back: Response = serde_json::from_str(&line).unwrap();
        assert_eq!(back, r);

        let r_none = Response::PreviewUrl { url: None };
        let line = serde_json::to_string(&r_none).unwrap();
        let back: Response = serde_json::from_str(&line).unwrap();
        assert_eq!(back, r_none);
    }

    #[test]
    fn unknown_command_type_is_rejected() {
        let err =
            serde_json::from_str::<Command>(r#"{"type":"explode","emoji_id":"x"}"#).unwrap_err();
        assert!(err.to_string().contains("unknown variant"));
    }
}
