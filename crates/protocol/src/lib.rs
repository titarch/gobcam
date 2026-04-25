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

use serde::{Deserialize, Serialize};

/// Daemon → client reply for a single [`Command`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Response {
    /// Command accepted and executed.
    Ok,
    /// Command rejected — `message` is human-readable, not stable.
    Error { message: String },
}

/// Client → daemon request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Command {
    /// Fire a reaction with the daemon's default duration. The
    /// `emoji_id` is a key from the daemon's asset library
    /// (`assets/fluent/manifest.toml`).
    Trigger { emoji_id: String },
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
    fn unknown_command_type_is_rejected() {
        let err =
            serde_json::from_str::<Command>(r#"{"type":"explode","emoji_id":"x"}"#).unwrap_err();
        assert!(err.to_string().contains("unknown variant"));
    }
}
