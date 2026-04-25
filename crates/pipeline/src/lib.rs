//! Gobcam pipeline daemon: a `GStreamer` graph that pulls from a v4l2
//! capture device, optionally composites emoji reactions via a pool of
//! pre-allocated compositor slots, and pushes the result to a v4l2loopback
//! sink. The `firewall` module works around a thread-safety bug in
//! `gst-plugins-good`'s `gst_v4l2_object_probe_caps`; without it the
//! pipeline aborts during preroll. See `docs/step3-debug-report.md`.

mod assets;
mod cli;
mod firewall;
mod pipeline;
mod reactions;
mod runner;
mod slots;

pub use cli::Cli;

use std::sync::Arc;

use anyhow::Result;
use tracing::info;

use crate::assets::Library;
use crate::assets::fluent::FluentLibrary;
use crate::reactions::Reactor;

pub fn run(cli: &Cli) -> Result<()> {
    init_tracing();
    gstreamer::init()?;
    info!(
        input = %cli.input.display(),
        output = %cli.output.display(),
        overlay = ?cli.overlay,
        triggers_stdin = cli.triggers_stdin,
        "starting pipeline"
    );

    let (pipeline, slots) = pipeline::build(cli)?;
    let library: Arc<dyn Library> = Arc::new(FluentLibrary::new(&cli.asset_root));
    let reactor = Arc::new(Reactor::new(slots, library));

    if let Some(emoji_id) = &cli.overlay {
        // Always-on overlay = a reaction with no deadline.
        reactor.activate(emoji_id, None)?;
    }

    if cli.triggers_stdin {
        reactions::spawn_stdin_reader(Arc::clone(&reactor));
    }

    runner::run(&pipeline)
}

fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = fmt().with_env_filter(filter).try_init();
}
