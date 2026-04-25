//! Gobcam pipeline daemon: a `GStreamer` graph that pulls from a v4l2
//! capture device, optionally composites an emoji overlay, and pushes the
//! result to a v4l2loopback sink.

mod assets;
mod cli;
mod overlay;
mod pipeline;
mod runner;

pub use cli::Cli;

use anyhow::{Context, Result};
use tracing::info;

use crate::assets::fluent::FluentLibrary;
use crate::assets::{EmojiId, Library, SkinTone, Style};
use crate::overlay::Overlay;

pub fn run(cli: &Cli) -> Result<()> {
    init_tracing();
    gstreamer::init()?;
    info!(
        input = %cli.input.display(),
        output = %cli.output.display(),
        overlay = ?cli.overlay,
        "starting pipeline"
    );
    let pipeline = pipeline::build_passthrough(cli)?;
    if let Some(emoji_id) = &cli.overlay {
        attach_emoji(&pipeline, cli, emoji_id)?;
    }
    runner::run(&pipeline)
}

fn attach_emoji(pipeline: &gstreamer::Pipeline, cli: &Cli, emoji_id: &str) -> Result<()> {
    let library = FluentLibrary::new(&cli.asset_root);
    let (style, source) = library
        .resolve(&EmojiId::new(emoji_id), Style::Animated, SkinTone::None)
        .or_else(|| library.resolve(&EmojiId::new(emoji_id), Style::Animated, SkinTone::Default))
        .with_context(|| {
            format!(
                "emoji '{emoji_id}' not found under {}; did you run `just sync-emoji`?",
                cli.asset_root.display()
            )
        })?;
    info!(emoji = emoji_id, ?style, "resolved overlay");
    let overlay_bin = Overlay::build(&source, &format!("overlay-{emoji_id}"))?;
    pipeline::attach_overlay(pipeline, &overlay_bin, default_position(&source))?;
    Ok(())
}

/// Bottom-right corner placement for v1. Step 3 will animate this via `GstController`.
fn default_position(source: &assets::Source) -> (i32, i32) {
    let (w, h) = source.dimensions();
    // Assumes a 1280x720 main canvas; a less-fragile lookup will land in Step 3
    // when we attach to a known compositor output size.
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
