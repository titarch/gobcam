use std::path::PathBuf;

use clap::Parser;

/// Gobcam pipeline daemon — passthrough with optional emoji overlay.
#[derive(Parser, Debug, Clone)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// v4l2 capture device (the real webcam).
    #[arg(short, long, default_value = "/dev/video0", env = "GOBCAM_INPUT")]
    pub input: PathBuf,

    /// v4l2loopback sink device exposed to apps.
    #[arg(short, long, default_value = "/dev/video10", env = "GOBCAM_OUTPUT")]
    pub output: PathBuf,

    /// Always-on overlay emoji id from the Fluent library (e.g. `fire`, `thumbs_up`).
    #[arg(long, env = "GOBCAM_OVERLAY")]
    pub overlay: Option<String>,

    /// Root directory of the synced Fluent asset tree (`scripts/sync-emoji.sh`).
    #[arg(long, default_value = "assets/fluent", env = "GOBCAM_ASSET_ROOT")]
    pub asset_root: PathBuf,

    /// Read emoji ids from stdin (one per line) and fire each as a 3-second reaction.
    #[arg(long, env = "GOBCAM_TRIGGERS_STDIN")]
    pub triggers_stdin: bool,

    /// Path to a Unix domain socket to accept commands on
    /// (line-delimited JSON; see `gobcam-protocol`). Created on
    /// startup, removed on shutdown. A typical value is
    /// `$XDG_RUNTIME_DIR/gobcam.sock`.
    #[arg(long, env = "GOBCAM_SOCKET")]
    pub socket: Option<PathBuf>,
}
