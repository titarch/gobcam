//! Gobcam pipeline daemon: a `GStreamer` graph that pulls from a v4l2
//! capture device, optionally composites an emoji overlay, and pushes the
//! result to a v4l2loopback sink.

mod assets;
mod cli;
mod overlay;
mod pipeline;
mod reactions;
mod runner;

pub use cli::Cli;

use std::sync::Arc;

use anyhow::{Context, Result};
use tracing::info;

use crate::assets::fluent::FluentLibrary;
use crate::assets::{EmojiId, Library, SkinTone, Style};
use crate::overlay::Overlay;
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

    let pipeline = pipeline::build_passthrough(cli)?;
    let library: Arc<dyn Library> = Arc::new(FluentLibrary::new(&cli.asset_root));

    if let Some(emoji_id) = &cli.overlay {
        attach_always_on(&pipeline, &*library, emoji_id, &cli.asset_root)?;
    }

    if cli.triggers_stdin {
        let reactor = Arc::new(Reactor::new(pipeline.clone(), library)?);
        reactions::spawn_stdin_reader(reactor);
    }

    runner::run(&pipeline)
}

fn attach_always_on(
    pipeline: &gstreamer::Pipeline,
    library: &dyn Library,
    emoji_id: &str,
    asset_root: &std::path::Path,
) -> Result<()> {
    let (style, source) = library
        .resolve(&EmojiId::new(emoji_id), Style::Animated, SkinTone::None)
        .or_else(|| library.resolve(&EmojiId::new(emoji_id), Style::Animated, SkinTone::Default))
        .with_context(|| {
            format!(
                "emoji '{emoji_id}' not found under {}; did you run `just sync-emoji`?",
                asset_root.display()
            )
        })?;
    info!(emoji = emoji_id, ?style, "resolved always-on overlay");
    let overlay = Overlay::build(&source, &format!("overlay-{emoji_id}"))?;
    let _sink_pad = pipeline::attach_overlay(pipeline, &overlay, default_position(&source))?;
    Ok(())
}

fn default_position(source: &assets::Source) -> (i32, i32) {
    let (w, h) = source.dimensions();
    let canvas = (1280_i32, 720_i32);
    let margin = 32_i32;
    let w = i32::try_from(w).unwrap_or(canvas.0);
    let h = i32::try_from(h).unwrap_or(canvas.1);
    (
        (canvas.0 - w - margin).max(0),
        (canvas.1 - h - margin).max(0),
    )
}

fn init_tracing() {
    use tracing_subscriber::{EnvFilter, fmt};
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = fmt().with_env_filter(filter).try_init();
}
