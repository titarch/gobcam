use std::path::PathBuf;

use clap::Parser;

/// Gobcam pipeline daemon — passthrough with optional emoji overlay.
#[derive(Parser, Debug, Clone)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// v4l2 capture device (the real webcam).
    #[arg(short, long, default_value = "/dev/video0", env = "GOBCAM_INPUT")]
    pub input: PathBuf,

    /// `v4l2src` `io-mode`. Default `auto` works for most real
    /// cameras; set to `rw` when reading from a v4l2loopback (loopback
    /// devices don't support the buffer pool that AUTO/MMAP wants).
    #[arg(long, default_value = "auto", env = "GOBCAM_INPUT_IO_MODE")]
    pub input_io_mode: String,

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

    /// How many compositor sink pads to pre-allocate. Bound on
    /// simultaneously-visible reactions.
    #[arg(long, default_value_t = 48, env = "GOBCAM_SLOT_COUNT")]
    pub slot_count: usize,

    /// Slot canvas dimension in pixels (square). Lower trades
    /// sharpness for blend cost.
    #[arg(long, default_value_t = 256, env = "GOBCAM_SLOT_DIM")]
    pub slot_dim: u32,

    /// Expose the compositor's output as a localhost MJPEG stream
    /// for the UI's preview pane.
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
    #[arg(long, env = "GOBCAM_PROFILE_LOG")]
    pub profile_log: Option<PathBuf>,

    /// Exit cleanly when stdin reaches EOF. Used by the UI as a
    /// supervisor: pipe-EOF when the UI dies shuts down the daemon.
    #[arg(long, env = "GOBCAM_EXIT_ON_STDIN_EOF")]
    pub exit_on_stdin_eof: bool,
}
