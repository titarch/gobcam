use std::path::PathBuf;

use clap::Parser;

/// Gobcam pipeline daemon — passthrough mode (Step 1).
#[derive(Parser, Debug, Clone)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// v4l2 capture device (the real webcam).
    #[arg(short, long, default_value = "/dev/video0", env = "GOBCAM_INPUT")]
    pub input: PathBuf,

    /// v4l2loopback sink device exposed to apps.
    #[arg(short, long, default_value = "/dev/video10", env = "GOBCAM_OUTPUT")]
    pub output: PathBuf,
}
