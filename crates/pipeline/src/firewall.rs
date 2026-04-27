//! `CAPS` / `ACCEPT_CAPS` query firewall on `v4l2sink.sink_pad`. Avoids a
//! concurrent-caps-probe heap race in `gst-plugins-good`'s v4l2 plugin.
//! See `docs/v4l2sink-thread-safety.md`; reproducer at
//! `crates/pipeline/examples/repro_v4l2_slots.rs`.
//!
//! 1. Probe a throwaway `v4l2sink` in READY to surface "device busy" early.
//! 2. Pin the desired raw format (I420 + `WxH` + fps). I420 matches the
//!    compositor's pinned src caps so v4l2sink is a passthrough — see
//!    `pipeline::description`.
//! 3. Install a `QUERY_DOWNSTREAM` probe answering `CAPS` / `ACCEPT_CAPS`
//!    so the default `gst_v4l2sink_get_caps` never runs on streaming threads.

use anyhow::{Context, Result};
use gstreamer::{self as gst, prelude::*};

/// Format the compositor blends in. Alpha-aware (4:4:4 YUV with
/// alpha) so per-pad RGBA alpha survives into the blend.
pub(crate) const COMPOSITOR_BLEND_FORMAT: &str = "AYUV";

/// Format that reaches `v4l2sink`. Cheap to narrow from
/// [`COMPOSITOR_BLEND_FORMAT`] (chroma-subsample + alpha-drop, no
/// colour-space matrix) and is what `v4l2loopback` consumers
/// negotiate against. The firewall and the pipeline's trailing
/// videoconvert *must* agree on this — see
/// `pipeline::description`.
pub(crate) const SINK_INPUT_FORMAT: &str = "I420";

/// Desired fully-fixated output format for the v4l2sink.
#[derive(Debug, Clone, Copy)]
pub(crate) struct OutputCaps {
    pub width: i32,
    pub height: i32,
    pub fps_num: i32,
    pub fps_den: i32,
}

pub(crate) fn install(v4l2sink: &gst::Element, device: &str, caps: OutputCaps) -> Result<()> {
    let firewall_caps = derive_firewall_caps(device, caps)?;
    tracing::debug!(caps = %firewall_caps, "firewall caps");

    let sink_pad = v4l2sink
        .static_pad("sink")
        .context("v4l2sink missing sink pad")?;
    sink_pad.add_probe(gst::PadProbeType::QUERY_DOWNSTREAM, move |_pad, info| {
        let Some(query) = info.query_mut() else {
            return gst::PadProbeReturn::Ok;
        };
        match query.view_mut() {
            gst::QueryViewMut::Caps(q) => {
                let result = q.filter().map_or_else(
                    || firewall_caps.clone(),
                    |filter| firewall_caps.intersect(filter),
                );
                if result.is_empty() {
                    tracing::warn!(
                        filter = ?q.filter(),
                        "firewall: empty intersection with downstream"
                    );
                }
                q.set_result(&result);
                gst::PadProbeReturn::Handled
            }
            gst::QueryViewMut::AcceptCaps(q) => {
                q.set_result(q.caps().can_intersect(&firewall_caps));
                gst::PadProbeReturn::Handled
            }
            _ => gst::PadProbeReturn::Ok,
        }
    });
    Ok(())
}

/// Build the firewall caps. The READY-state probe surfaces
/// "device busy" / "no such device" early. We don't intersect
/// with `query_caps`: v4l2loopback only advertises its current
/// format, which would reject any mode change.
fn derive_firewall_caps(device: &str, caps: OutputCaps) -> Result<gst::Caps> {
    let probe = gst::ElementFactory::make("v4l2sink")
        .property("device", device)
        .property("sync", false)
        .build()
        .context("creating probe v4l2sink")?;
    probe
        .set_state(gst::State::Ready)
        .context("probe v4l2sink to READY")?;
    // `set_state(Ready)` only confirms the transition was *initiated*;
    // for an async element the `state(timeout)` is what proves it
    // actually reached READY. A non-Success here means the device
    // opened but the driver hung — we still propagate caps (the
    // QUERY_DOWNSTREAM probe answers regardless), but log so the real
    // failure isn't only visible later as a confusing link error.
    let (probe_state_result, _, _) = probe.state(gst::ClockTime::from_seconds(2));
    if !matches!(probe_state_result, Ok(gst::StateChangeSuccess::Success)) {
        tracing::warn!(?probe_state_result, %device, "firewall probe didn't reach READY cleanly");
    }
    if let Err(e) = probe.set_state(gst::State::Null) {
        tracing::warn!(error = ?e, %device, "firewall probe NULL teardown failed");
    }

    Ok(gst::Caps::builder("video/x-raw")
        .field("format", SINK_INPUT_FORMAT)
        .field("width", caps.width)
        .field("height", caps.height)
        .field("framerate", gst::Fraction::new(caps.fps_num, caps.fps_den))
        .build())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_are_known_constants() {
        // Locks the load-bearing format strings against accidental
        // edits. If these change, `pipeline::description`'s
        // capsfilter, `derive_firewall_caps`, and the documentation
        // (`docs/architecture.md` "Idle cost") must move in lockstep.
        assert_eq!(COMPOSITOR_BLEND_FORMAT, "AYUV");
        assert_eq!(SINK_INPUT_FORMAT, "I420");
    }
}
