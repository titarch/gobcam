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
    let _ = probe.state(gst::ClockTime::from_seconds(2));
    probe.set_state(gst::State::Null).ok();

    Ok(gst::Caps::builder("video/x-raw")
        .field("format", "I420")
        .field("width", caps.width)
        .field("height", caps.height)
        .field("framerate", gst::Fraction::new(caps.fps_num, caps.fps_den))
        .build())
}
