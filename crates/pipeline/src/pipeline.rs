use anyhow::{Context, Result, anyhow};
use gstreamer::{self as gst, prelude::*};
use serde_json::json;

use crate::assets::cache::CacheRoot;
use crate::cli::Cli;
use crate::firewall;
use crate::preview;
use crate::profile;
use crate::slots::Slot;

/// How many overlay slots to allocate. Bound on simultaneously-visible
/// reactions; raise for more concurrency at a small idle CPU cost.
pub(crate) const SLOT_COUNT: usize = 4;

/// Pipeline topology: camera → compositor → loopback. The compositor's
/// `sink_0` is the camera; `sink_1..sink_N` are the pre-allocated slots
/// (`slots::Slot::build`); the V4L2 caps-query firewall (`firewall::install`)
/// is attached to `v4l2sink`'s sink pad.
fn description(cli: &Cli) -> Result<String> {
    let input = path_str(&cli.input, "--input")?;
    let output = path_str(&cli.output, "--output")?;
    // - camera-side capsfilter fixates framerate so compositor's latency
    //   queries can be answered before negotiation completes.
    // - `queue` between live source and compositor prevents aggregator
    //   latency deadlocks when mixing live and non-live overlay branches.
    // - v4l2sink has `name=sink` so we can look it up to attach the
    //   firewall probe (see `firewall::install`).
    // - With `--preview`, a `tee` after the post-mix videoconvert fans
    //   out one branch to the v4l2 sink (unchanged) and one to a JPEG
    //   appsink (`name=preview`). The `appsink` callback writes the
    //   latest frame to `<cache>/runtime-preview.jpg` for the UI to
    //   `<img>`-render.
    let body = if cli.preview {
        format!(
            "videoconvert ! tee name=split \
             split. ! queue ! v4l2sink name=sink device={output} sync=false \
             split. ! queue max-size-buffers=1 max-size-time=0 max-size-bytes=0 leaky=downstream ! \
                videoconvert ! videoscale ! \
                video/x-raw,width=320,height=180 ! \
                jpegenc quality=70 ! \
                appsink name=preview sync=false drop=true max-buffers=1"
        )
    } else {
        format!("videoconvert ! v4l2sink name=sink device={output} sync=false")
    };
    Ok(format!(
        "v4l2src device={input} ! \
         video/x-raw,width={w},height={h},framerate={fn_}/{fd} ! \
         queue ! videoconvert ! \
         compositor name=mix background=black ! \
         {body}",
        w = cli.width,
        h = cli.height,
        fn_ = cli.fps_num,
        fd = cli.fps_den,
    ))
}

/// Build the camera→compositor→sink pipeline, install the v4l2sink caps
/// firewall, and pre-allocate `SLOT_COUNT` overlay slots attached to the
/// compositor. When `--preview` is set, the `cache` is also used to
/// install the preview-frame writer. Pipeline returns in NULL state.
pub(crate) fn build(cli: &Cli, cache: &CacheRoot) -> Result<(gst::Pipeline, Vec<Slot>)> {
    let desc = description(cli)?;
    let pipeline = gst::parse::launch(&desc)
        .with_context(|| format!("parsing pipeline: {desc}"))?
        .downcast::<gst::Pipeline>()
        .map_err(|_| anyhow!("parsed element is not a gst::Pipeline"))?;

    let v4l2sink = pipeline
        .by_name("sink")
        .context("v4l2sink 'sink' not found")?;
    let output = path_str(&cli.output, "--output")?;
    let caps = firewall::OutputCaps {
        width: i32::try_from(cli.width).unwrap_or(i32::MAX),
        height: i32::try_from(cli.height).unwrap_or(i32::MAX),
        fps_num: i32::try_from(cli.fps_num).unwrap_or(i32::MAX),
        fps_den: i32::try_from(cli.fps_den).unwrap_or(i32::MAX),
    };
    firewall::install(&v4l2sink, output, caps)
        .context("installing v4l2sink caps-query firewall")?;

    let compositor = pipeline
        .by_name("mix")
        .context("compositor 'mix' not found")?;
    let mut slots = Vec::with_capacity(SLOT_COUNT);
    for idx in 0..SLOT_COUNT {
        slots.push(Slot::build(&pipeline, &compositor, idx)?);
    }

    if profile::enabled() {
        install_profile_probe(&v4l2sink, &slots)?;
    }

    if cli.preview {
        preview::install(&pipeline, cache).context("installing preview writer")?;
    }

    Ok((pipeline, slots))
}

/// Buffer probe on `v4l2sink.sink`: every output buffer triggers a
/// profile event recording its PTS plus the current `alpha` of every
/// slot pad. Lets the post-processor reconstruct what the compositor
/// blended for each frame.
fn install_profile_probe(v4l2sink: &gst::Element, slots: &[Slot]) -> Result<()> {
    let pad = v4l2sink
        .static_pad("sink")
        .context("v4l2sink missing sink pad for profile probe")?;
    let slot_pads: Vec<gst::Pad> = slots.iter().map(|s| s.sink_pad().clone()).collect();
    pad.add_probe(gst::PadProbeType::BUFFER, move |_pad, info| {
        let pts_ns = info
            .buffer()
            .and_then(|b| b.pts())
            .map(gst::ClockTime::nseconds);
        let alphas: Vec<f64> = slot_pads
            .iter()
            .map(|p| p.property::<f64>("alpha"))
            .collect();
        profile::mark(
            "v4l2sink.output",
            json!({
                "pts_ns": pts_ns,
                "alphas": alphas,
            }),
        );
        gst::PadProbeReturn::Ok
    });
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
    fn description_contains_compositor_and_named_sink() {
        let cli = Cli {
            input: "/dev/video0".into(),
            output: "/dev/video10".into(),
            overlay: None,
            cache_root: None,
            triggers_stdin: false,
            socket: None,
            profile_log: None,
            exit_on_stdin_eof: false,
            width: 1280,
            height: 720,
            fps_num: 30,
            fps_den: 1,
            preview: false,
        };
        let desc = description(&cli).unwrap();
        assert!(desc.contains("compositor name=mix"), "desc was: {desc}");
        assert!(desc.contains("v4l2sink name=sink"), "desc was: {desc}");
    }
}
