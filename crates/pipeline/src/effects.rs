//! Procedural transforms on slot compositor pads via `GstController`.
//!
//! For triggered reactions we attach `InterpolationControlSource` curves
//! to the slot's `alpha` and `ypos` properties so reactions fade in,
//! drift slightly upward, and fade out before deactivation. Always-on
//! overlays (Step 2 path) skip this and stay at the static `alpha=1` /
//! pinned position set by `Slot::try_activate`.
//!
//! Control bindings replace the next manual property write on the same
//! property, so on deactivation we explicitly remove them via [`clear`]
//! before `Slot::deactivate` resets `alpha=0`.

use std::time::Duration;

use anyhow::{Context, Result};
use gstreamer::{self as gst, prelude::*};
use gstreamer_controller::prelude::*;
use gstreamer_controller::{DirectControlBinding, InterpolationControlSource, InterpolationMode};
use serde_json::json;

use crate::profile;

const FADE_IN: Duration = Duration::from_millis(120);
const FADE_OUT: Duration = Duration::from_millis(400);
const DRIFT_UP_PX: f64 = 30.0;

const ANIMATED_PROPERTIES: [&str; 2] = ["alpha", "ypos"];

/// Anchor curves at the compositor element's current running time so they
/// align with the slot pump's PTS. The slot pad's parent is the
/// compositor. `id` is the originating trigger id, recorded for profile
/// output.
pub(crate) fn apply_default(
    pad: &gst::Pad,
    duration: Duration,
    base_position: (i32, i32),
    id: u64,
) -> Result<()> {
    profile::mark("effects.apply.enter", json!({ "id": id }));
    clear(pad);
    let start = pad
        .parent_element()
        .and_then(|el| el.current_running_time())
        .unwrap_or(gst::ClockTime::ZERO);
    let total = clock_time_from_duration(duration);
    let fade_in = clock_time_from_duration(FADE_IN);
    let fade_out = clock_time_from_duration(FADE_OUT);

    bind_alpha(pad, start, total, fade_in, fade_out)?;
    bind_ypos(pad, start, total, base_position.1)?;
    profile::mark(
        "effects.apply.exit",
        json!({
            "id": id,
            "start_ns": start.nseconds(),
            "fade_in_ms": u64::try_from(FADE_IN.as_millis()).unwrap_or(u64::MAX),
            "fade_out_ms": u64::try_from(FADE_OUT.as_millis()).unwrap_or(u64::MAX),
        }),
    );
    Ok(())
}

/// Drop any control bindings the effects layer installed. Idempotent —
/// no-op if the property has no binding (e.g. an always-on slot).
pub(crate) fn clear(pad: &gst::Pad) {
    for prop in ANIMATED_PROPERTIES {
        if let Some(binding) = pad.control_binding(prop) {
            let _ = pad.remove_control_binding(&binding);
        }
    }
}

fn bind_alpha(
    pad: &gst::Pad,
    start: gst::ClockTime,
    total: gst::ClockTime,
    fade_in: gst::ClockTime,
    fade_out: gst::ClockTime,
) -> Result<()> {
    let source = InterpolationControlSource::new();
    source.set_mode(InterpolationMode::Linear);
    // Pad the curve with a keyframe at the pipeline-time origin so any
    // sync at t < `start` reads α=0 (interpolated 0→0) rather than
    // "no value", which would leave the manual α=1 from
    // `Slot::try_activate` visible for one or two frames. Confirmed
    // root cause of the "click-twice flicker" via profile capture.
    source.set(gst::ClockTime::ZERO, 0.0);
    source.set(start, 0.0);
    let visible_at = start + fade_in.min(total);
    source.set(visible_at, 1.0);
    if total > fade_in + fade_out {
        source.set(start + total - fade_out, 1.0);
    }
    source.set(start + total, 0.0);

    let binding = DirectControlBinding::new_absolute(pad, "alpha", &source);
    pad.add_control_binding(&binding)
        .context("add alpha control binding")?;
    Ok(())
}

fn bind_ypos(
    pad: &gst::Pad,
    start: gst::ClockTime,
    total: gst::ClockTime,
    base_y: i32,
) -> Result<()> {
    let source = InterpolationControlSource::new();
    source.set_mode(InterpolationMode::Linear);
    let y0 = f64::from(base_y);
    source.set(start, y0);
    source.set(start + total, y0 - DRIFT_UP_PX);

    let binding = DirectControlBinding::new_absolute(pad, "ypos", &source);
    pad.add_control_binding(&binding)
        .context("add ypos control binding")?;
    Ok(())
}

fn clock_time_from_duration(d: Duration) -> gst::ClockTime {
    gst::ClockTime::from_nseconds(u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
}
