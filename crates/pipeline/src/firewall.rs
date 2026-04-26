//! `CAPS` / `ACCEPT_CAPS` query firewall on `v4l2sink.sink_pad`.
//!
//! Works around a thread-safety bug in [`gst-plugins-good`]'s
//! `gst_v4l2_object_probe_caps`: with multiple upstream tasks (compositor
//! inputs) querying caps concurrently, the V4L2 plugin's internal
//! `clear_format_list` / `fill_format_list` race on a `GSList` and corrupt
//! the heap. Filing-quality reproducer in
//! `crates/pipeline/examples/pg_v4l2_slots.rs`; full debug log in
//! `docs/step3-debug-report.md`.
//!
//! The firewall:
//!   1. Builds a temporary, isolated `v4l2sink`, sets it to READY (which
//!      opens the device single-threaded), queries device-specific caps,
//!      then drops it. NULL-state caps are too broad (the V4L2 plugin's
//!      pad-template superset).
//!   2. Intersects with the daemon's preferred output (YUY2/1280×720/30fps).
//!   3. Installs a `QUERY_DOWNSTREAM` probe on the real `v4l2sink.sink`
//!      that handles both `CAPS` and `ACCEPT_CAPS` queries with the
//!      precomputed caps and returns `Handled`. The default
//!      `gst_v4l2sink_get_caps` is never invoked from streaming threads.

use anyhow::{Context, Result};
use gstreamer::{self as gst, prelude::*};

/// Desired output format we want the v4l2sink (and therefore the
/// loopback) to negotiate. The firewall narrows the device's
/// reported caps to this single fully-fixated format so the
/// compositor has nothing else to settle on.
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
                        "firewall: empty intersection — caps mismatch with downstream"
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

/// Build the firewall caps from the requested output format. We
/// briefly open the v4l2sink in READY to surface "device busy" /
/// "no such device" errors early, but we *don't* intersect with
/// `query_caps` — v4l2loopback only advertises its *currently set*
/// format (locked in by the last writer), so intersecting would
/// reject any mode change. The loopback is permissive about the
/// caps a fresh writer pushes; the actual `SET_FMT` happens during
/// pipeline preroll regardless of what the firewall returns.
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
        .field("format", "YUY2")
        .field("width", caps.width)
        .field("height", caps.height)
        .field("framerate", gst::Fraction::new(caps.fps_num, caps.fps_den))
        .build())
}
