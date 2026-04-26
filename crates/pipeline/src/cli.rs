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

    /// Camera capture width in pixels.
    #[arg(long, default_value_t = 1280, env = "GOBCAM_WIDTH")]
    pub width: u32,

    /// Camera capture height in pixels.
    #[arg(long, default_value_t = 720, env = "GOBCAM_HEIGHT")]
    pub height: u32,

    /// Camera framerate numerator (e.g. `30` for 30 fps).
    #[arg(long, default_value_t = 30, env = "GOBCAM_FPS_NUM")]
    pub fps_num: u32,

    /// Camera framerate denominator (usually `1`; use `1000` for
    /// fractional-fps modes like 7.5 = 7500/1000).
    #[arg(long, default_value_t = 1, env = "GOBCAM_FPS_DEN")]
    pub fps_den: u32,

    /// Branch the compositor's output through a JPEG encoder and
    /// write the latest frame to `<cache>/runtime-preview.jpg` for
    /// the UI's preview pane. Adds ~3 % CPU at 320×180.
    #[arg(long, env = "GOBCAM_PREVIEW")]
    pub preview: bool,

    /// Always-on overlay emoji id from the Fluent library (e.g. `fire`, `thumbs_up`).
    #[arg(long, env = "GOBCAM_OVERLAY")]
    pub overlay: Option<String>,

    /// Override the on-disk preview/animated cache root. Defaults to
    /// `$XDG_CACHE_HOME/gobcam` (or `~/.cache/gobcam`).
    #[arg(long, env = "GOBCAM_CACHE_ROOT")]
    pub cache_root: Option<PathBuf>,

    /// Read emoji ids from stdin (one per line) and fire each as a 3-second reaction.
    #[arg(long, env = "GOBCAM_TRIGGERS_STDIN")]
    pub triggers_stdin: bool,

    /// Path to a Unix domain socket to accept commands on
    /// (line-delimited JSON; see `gobcam-protocol`). Created on
    /// startup, removed on shutdown. A typical value is
    /// `$XDG_RUNTIME_DIR/gobcam.sock`.
    #[arg(long, env = "GOBCAM_SOCKET")]
    pub socket: Option<PathBuf>,

    /// Path for a JSONL profile log of trigger-path latency events.
    /// Off by default; opt in via this flag or `GOBCAM_PROFILE_LOG`.
    /// See `crates/pipeline/src/profile.rs` for the schema.
    #[arg(long, env = "GOBCAM_PROFILE_LOG")]
    pub profile_log: Option<PathBuf>,

    /// Exit cleanly when stdin reaches EOF. The UI process passes this
    /// flag and keeps an open pipe to the daemon's stdin; if the UI
    /// dies for any reason (including SIGKILL), the kernel closes the
    /// pipe and the daemon shuts itself down.
    #[arg(long, env = "GOBCAM_EXIT_ON_STDIN_EOF")]
    pub exit_on_stdin_eof: bool,
}
