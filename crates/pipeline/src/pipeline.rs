use anyhow::{Context, Result, anyhow};
use gstreamer::{self as gst, prelude::*};
use serde_json::json;

use crate::cli::Cli;
use crate::firewall;
use crate::profile;
use crate::slots::Slot;

/// Camera → compositor → v4l2sink. Compositor `sink_0` is the camera;
/// `sink_1..` are slots from `slots::Slot::build`.
fn description(cli: &Cli) -> Result<String> {
    let input = path_str(&cli.input, "--input")?;
    let output = path_str(&cli.output, "--output")?;
    let io_mode = io_mode_token(&cli.input_io_mode)?;
    // Camera-side capsfilter fixates framerate so the compositor can
    // answer latency queries pre-negotiation. The `queue` between the
    // live source and compositor prevents aggregator latency
    // deadlocks. With `--preview`, a `tee` fans out a JPEG appsink
    // branch to the MJPEG-over-HTTP preview socket.
    //
    // Compositor blends in AYUV (alpha-aware YUV 4:4:4) so per-pad
    // RGBA alpha survives into the blend. The trailing videoconvert
    // narrows AYUV → I420 for v4l2sink; this is *much* cheaper than
    // RGBA → I420 (no colour-space matrix, just chroma subsample +
    // alpha drop), and is what we trade for keeping transparent
    // pixels transparent over the camera. Idle cost ≈ 10 % at 1080p
    // vs ~50 % when the full RGBA round-trip was in the path.
    let body = if cli.preview {
        format!(
            "videoconvert ! video/x-raw,format=I420,width={w},height={h} ! \
             tee name=split \
             split. ! queue ! v4l2sink name=sink device={output} sync=false \
             split. ! queue max-size-buffers=1 max-size-time=0 max-size-bytes=0 leaky=downstream ! \
                videoscale ! \
                video/x-raw,width=320,height=180 ! \
                jpegenc quality=70 ! \
                appsink name=preview sync=false drop=true max-buffers=1",
            w = cli.width,
            h = cli.height,
        )
    } else {
        format!(
            "videoconvert ! video/x-raw,format=I420,width={w},height={h} ! \
             v4l2sink name=sink device={output} sync=false",
            w = cli.width,
            h = cli.height,
        )
    };
    // `ignore-inactive-pads=true` pairs with each slot's
    // `max-last-buffer-repeat=0` so idle slots drop out of the blend.
    Ok(format!(
        "v4l2src device={input} io-mode={io_mode} ! \
         video/x-raw,width={w},height={h},framerate={fn_}/{fd} ! \
         queue ! videoconvert ! \
         compositor name=mix background=black ignore-inactive-pads=true ! \
         video/x-raw,format=AYUV,width={w},height={h} ! \
         {body}",
        w = cli.width,
        h = cli.height,
        fn_ = cli.fps_num,
        fd = cli.fps_den,
    ))
}

/// Build the pipeline, install the v4l2sink caps firewall, and
/// pre-allocate overlay slots. Returns in NULL state.
pub(crate) fn build(cli: &Cli) -> Result<(gst::Pipeline, Vec<Slot>)> {
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
    // See docs/v4l2sink-thread-safety.md for rationale.
    firewall::install(&v4l2sink, output, caps)
        .context("installing v4l2sink caps-query firewall")?;

    let compositor = pipeline
        .by_name("mix")
        .context("compositor 'mix' not found")?;
    let mut slots = Vec::with_capacity(cli.slot_count);
    for idx in 0..cli.slot_count {
        slots.push(Slot::build(&pipeline, &compositor, idx, cli.slot_dim)?);
    }

    if profile::enabled() {
        install_profile_probe(&v4l2sink, &slots)?;
    }

    Ok((pipeline, slots))
}

/// Buffer probe recording each output frame's PTS and per-slot alpha.
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

fn io_mode_token(name: &str) -> Result<&'static str> {
    match name {
        "auto" => Ok("0"),
        "rw" => Ok("1"),
        "mmap" => Ok("2"),
        "userptr" => Ok("3"),
        "dmabuf" => Ok("4"),
        "dmabuf-import" => Ok("5"),
        other => Err(anyhow!(
            "--input-io-mode must be one of auto|rw|mmap|userptr|dmabuf|dmabuf-import, got: {other}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn description_contains_compositor_and_named_sink() {
        let cli = Cli {
            input: "/dev/video0".into(),
            input_io_mode: "auto".into(),
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
            slot_count: 48,
            slot_dim: 256,
            preview: false,
        };
        let desc = description(&cli).unwrap();
        assert!(desc.contains("compositor name=mix"), "desc was: {desc}");
        assert!(desc.contains("v4l2sink name=sink"), "desc was: {desc}");
    }
}
