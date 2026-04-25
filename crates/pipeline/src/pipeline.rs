use anyhow::{Context, Result, anyhow};
use gstreamer::{self as gst, prelude::*};

use crate::cli::Cli;
use crate::overlay::Overlay;

/// Pipeline topology with a `compositor` so overlays can be attached.
/// Camera feed is always `sink_0`; overlays request `sink_%u` pads after.
fn description(cli: &Cli) -> Result<String> {
    let input = path_str(&cli.input, "--input")?;
    let output = path_str(&cli.output, "--output")?;
    // - camera-side capsfilter fixates framerate so compositor's latency
    //   queries can be answered before negotiation completes.
    // - `queue` between live source and compositor prevents aggregator
    //   latency deadlocks when mixing live and non-live overlay branches.
    // - compositor `ignore-inactive-pads=true` lets aggregation proceed
    //   when an overlay branch hasn't produced its first buffer yet.
    // - compositor-output capsfilter pins YUY2 so adding an RGBA overlay
    //   mid-flight doesn't trigger output-format renegotiation that
    //   v4l2sink can't honor (Device busy on /dev/videoN).
    Ok(format!(
        "v4l2src device={input} ! \
         video/x-raw,width=1280,height=720,framerate=30/1 ! \
         queue ! videoconvert ! \
         compositor name=mix background=black ignore-inactive-pads=true ! \
         video/x-raw,format=YUY2,width=1280,height=720,framerate=30/1 ! \
         videoconvert ! v4l2sink device={output} sync=false"
    ))
}

pub(crate) fn build_passthrough(cli: &Cli) -> Result<gst::Pipeline> {
    let desc = description(cli)?;
    gst::parse::launch(&desc)
        .with_context(|| format!("parsing pipeline: {desc}"))?
        .downcast::<gst::Pipeline>()
        .map_err(|_| anyhow!("parsed element is not a gst::Pipeline"))
}

/// Add the overlay's elements to the pipeline, link them in chain, and connect
/// the terminal element's src pad to a fresh compositor sink pad. Returns the
/// compositor sink pad so callers can manipulate its `xpos`/`ypos`/`alpha`.
pub(crate) fn attach_overlay(
    pipeline: &gst::Pipeline,
    overlay: &Overlay,
    position: (i32, i32),
) -> Result<gst::Pad> {
    let compositor = pipeline
        .by_name("mix")
        .context("compositor 'mix' not found in pipeline")?;

    let element_refs: Vec<&gst::Element> = overlay.elements.iter().collect();
    pipeline.add_many(element_refs.as_slice())?;
    gst::Element::link_many(element_refs.as_slice())?;

    let sink_pad = compositor
        .request_pad_simple("sink_%u")
        .context("compositor refused a new sink pad")?;
    // Explicitly activate the new request pad before linking — when the
    // compositor is already in PLAYING, gst-core's automatic activation
    // doesn't fire for a fresh request pad.
    sink_pad.set_active(true)?;
    overlay.src_pad.link(&sink_pad)?;

    let (x, y) = position;
    sink_pad.set_property("xpos", x);
    sink_pad.set_property("ypos", y);

    // Sync each element's state from downstream to upstream — by the time
    // appsrc transitions to PLAYING and its loop starts pushing, every
    // downstream element is already in PLAYING. The reverse order avoids the
    // not-linked race where appsrc activates first and pushes into a still-
    // NULL videoconvert.
    for element in overlay.elements.iter().rev() {
        element.sync_state_with_parent()?;
    }

    Ok(sink_pad)
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
            triggers_stdin: false,
        };
        let desc = description(&cli).unwrap();
        assert!(desc.contains("compositor name=mix"), "desc was: {desc}");
    }
}
