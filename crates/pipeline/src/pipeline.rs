use anyhow::{Context, Result, anyhow};
use gstreamer::{self as gst, prelude::*};

use crate::cli::Cli;
use crate::overlay::Overlay;

/// Pipeline topology with a `compositor` so overlays can be attached.
/// Camera feed is always `sink_0`; overlays request `sink_%u` pads after.
fn description(cli: &Cli) -> Result<String> {
    let input = path_str(&cli.input, "--input")?;
    let output = path_str(&cli.output, "--output")?;
    // - capsfilter pins the camera framerate so compositor can answer latency queries
    // - `queue` between live source and compositor prevents aggregator latency
    //   deadlocks when mixing live and non-live overlay branches
    Ok(format!(
        "v4l2src device={input} ! \
         video/x-raw,width=1280,height=720,framerate=30/1 ! \
         queue ! videoconvert ! \
         compositor name=mix background=black ! videoconvert ! \
         v4l2sink device={output} sync=false"
    ))
}

pub(crate) fn build_passthrough(cli: &Cli) -> Result<gst::Pipeline> {
    let desc = description(cli)?;
    gst::parse::launch(&desc)
        .with_context(|| format!("parsing pipeline: {desc}"))?
        .downcast::<gst::Pipeline>()
        .map_err(|_| anyhow!("parsed element is not a gst::Pipeline"))
}

pub(crate) fn attach_overlay(
    pipeline: &gst::Pipeline,
    overlay: &Overlay,
    position: (i32, i32),
) -> Result<()> {
    let compositor = pipeline
        .by_name("mix")
        .context("compositor 'mix' not found in pipeline")?;
    pipeline.add(&overlay.bin)?;

    let sink_pad = compositor
        .request_pad_simple("sink_%u")
        .context("compositor refused a new sink pad")?;
    let src_pad = overlay
        .bin
        .static_pad("src")
        .context("overlay bin missing ghost src pad")?;
    src_pad.link(&sink_pad)?;

    let (x, y) = position;
    sink_pad.set_property("xpos", x);
    sink_pad.set_property("ypos", y);
    Ok(())
}

fn path_str<'a>(path: &'a std::path::Path, flag: &str) -> Result<&'a str> {
    path.to_str()
        .with_context(|| format!("{flag} must be a valid UTF-8 path: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn description_contains_compositor() {
        let cli = Cli {
            input: "/dev/video0".into(),
            output: "/dev/video10".into(),
            overlay: None,
            asset_root: "assets/fluent".into(),
        };
        let desc = description(&cli).unwrap();
        assert!(desc.contains("compositor name=mix"), "desc was: {desc}");
    }
}
