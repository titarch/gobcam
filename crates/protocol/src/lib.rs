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
}

/// Client → daemon request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    /// Fire a reaction with the daemon's default duration. The
    /// `emoji_id` is a key from the daemon's bundled Fluent catalog.
    Trigger { emoji_id: String },
    /// Enumerate every emoji in the catalog (~1500 entries).
    ListEmoji,
    /// Query the preview-bootstrap downloader's progress.
    SyncStatus,
}

/// One entry of the bundled Fluent catalog as the UI sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmojiInfo {
    /// Stable, `snake_case` id used by [`Command::Trigger`].
    pub id: String,
    /// Upstream display name (e.g. `"Smiling face with smiling eyes"`).
    pub name: String,
    /// Unicode glyph for fallback rendering when the preview image
    /// hasn't downloaded yet.
    pub glyph: String,
    /// Unicode group (e.g. `"Smileys & Emotion"`) for sectioning.
    pub group: String,
    /// Free-text keywords for client-side search.
    pub keywords: Vec<String>,
    /// `true` if Microsoft's animated repo carries an APNG for this
    /// emoji. When `false`, [`Command::Trigger`] uses the static 3D
    /// image as a single-frame loop.
    pub has_animated: bool,
    /// Absolute path the daemon expects the static 3D preview to live
    /// at after bootstrap (`$XDG_CACHE_HOME/gobcam/previews/<id>.png`).
    /// May not yet exist when [`Command::ListEmoji`] is answered.
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
    fn unknown_command_type_is_rejected() {
        let err =
            serde_json::from_str::<Command>(r#"{"type":"explode","emoji_id":"x"}"#).unwrap_err();
        assert!(err.to_string().contains("unknown variant"));
    }
}
