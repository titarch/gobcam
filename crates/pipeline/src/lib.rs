//! Gobcam pipeline daemon: a `GStreamer` graph that pulls from a v4l2
//! capture device and pushes to a v4l2loopback sink. Step 1 is plain
//! passthrough.

mod cli;
mod pipeline;
mod runner;

pub use cli::Cli;

use anyhow::Result;
use tracing::info;

pub fn run(cli: &Cli) -> Result<()> {
    init_tracing();
    gstreamer::init()?;
    info!(
        input = %cli.input.display(),
        output = %cli.output.display(),
        "starting passthrough pipeline"
    );
    let pipeline = pipeline::build_passthrough(cli)?;
    runner::run(&pipeline)
}

fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = fmt().with_env_filter(filter).try_init();
}
