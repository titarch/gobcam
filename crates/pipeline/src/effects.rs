//! Procedural transforms on slot compositor pads via `GstController`.
//! Cascade reactions attach `InterpolationControlSource` curves to
//! `alpha`, `ypos`, and `xpos` from a live [`AnimationConfig`]
//! snapshot. [`clear`] must run on deactivation so manual property
//! writes (`alpha=0`) take effect.

use std::time::Duration;

use anyhow::{Context, Result};
use gobcam_protocol::AnimationConfig;
use gstreamer::{self as gst, prelude::*};
use gstreamer_controller::prelude::*;
use gstreamer_controller::{DirectControlBinding, InterpolationControlSource, InterpolationMode};
use serde_json::json;

use crate::animation::InstancePlan;
use crate::profile;

const ANIMATED_PROPERTIES: [&str; 3] = ["alpha", "ypos", "xpos"];

/// Anchor curves at the compositor's current running time. Returns
/// the effective lifetime (scaled by `plan.speed_factor`) so the
/// reactor can schedule the deactivate timer.
pub(crate) fn apply_cascade(
    pad: &gst::Pad,
    cfg: &AnimationConfig,
    plan: &InstancePlan,
    id: u64,
) -> Result<Duration> {
    profile::mark("effects.apply.enter", json!({ "id": id }));
    clear(pad);
    let start = pad
        .parent_element()
        .and_then(|el| el.current_running_time())
        .unwrap_or(gst::ClockTime::ZERO);

    let speed = if plan.speed_factor > 0.0 {
        plan.speed_factor
    } else {
        1.0
    };
    let lifetime = Duration::from_millis(scale_ms(cfg.lifetime_ms, speed));
    let fade_in = Duration::from_millis(u64::from(cfg.fade_in_ms));
    let fade_out = Duration::from_millis(u64::from(cfg.fade_out_ms));
    let fade_out_start = Duration::from_millis(scale_ms(cfg.fade_out_start_ms, speed));

    let total_ct = clock_time_from_duration(lifetime);
    let fade_in_ct = clock_time_from_duration(fade_in);
    let fade_out_start_ct = clock_time_from_duration(fade_out_start);
    let fade_out_ct = clock_time_from_duration(fade_out);

    bind_alpha(
        pad,
        start,
        total_ct,
        fade_in_ct,
        fade_out_start_ct,
        fade_out_ct,
    )?;
    bind_ypos(pad, start, total_ct, plan.start_y, plan.end_y)?;
    if plan.start_x != plan.end_x {
        bind_xpos(pad, start, total_ct, plan.start_x, plan.end_x)?;
    }

    profile::mark(
        "effects.apply.exit",
        json!({
            "id": id,
            "start_ns": start.nseconds(),
            "lifetime_ms": u64::try_from(lifetime.as_millis()).unwrap_or(u64::MAX),
            "speed_factor": plan.speed_factor,
        }),
    );
    Ok(lifetime)
}

/// Drop any control bindings the effects layer installed. Idempotent.
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
    fade_out_start: gst::ClockTime,
    fade_out: gst::ClockTime,
) -> Result<()> {
    let source = InterpolationControlSource::new();
    source.set_mode(InterpolationMode::Linear);
    // (ZERO, 0.0) pre-key: `InterpolationControlSource` returns "no
    // value" for t < first keyframe, leaving the manual α=1 visible.
    source.set(gst::ClockTime::ZERO, 0.0);
    source.set(start, 0.0);
    let visible_at = start + fade_in.min(total);
    source.set(visible_at, 1.0);
    let hold_until = start + fade_out_start.min(total);
    if hold_until > visible_at {
        source.set(hold_until, 1.0);
    }
    let fade_done = (hold_until + fade_out).min(start + total);
    source.set(fade_done, 0.0);
    if fade_done < start + total {
        source.set(start + total, 0.0);
    }

    let binding = DirectControlBinding::new_absolute(pad, "alpha", &source);
    pad.add_control_binding(&binding)
        .context("add alpha control binding")?;
    Ok(())
}

fn bind_ypos(
    pad: &gst::Pad,
    start: gst::ClockTime,
    total: gst::ClockTime,
    start_y: i32,
    end_y: i32,
) -> Result<()> {
    let source = InterpolationControlSource::new();
    source.set_mode(InterpolationMode::Linear);
    source.set(start, f64::from(start_y));
    source.set(start + total, f64::from(end_y));

    let binding = DirectControlBinding::new_absolute(pad, "ypos", &source);
    pad.add_control_binding(&binding)
        .context("add ypos control binding")?;
    Ok(())
}

fn bind_xpos(
    pad: &gst::Pad,
    start: gst::ClockTime,
    total: gst::ClockTime,
    start_x: i32,
    end_x: i32,
) -> Result<()> {
    let source = InterpolationControlSource::new();
    source.set_mode(InterpolationMode::Linear);
    source.set(start, f64::from(start_x));
    source.set(start + total, f64::from(end_x));

    let binding = DirectControlBinding::new_absolute(pad, "xpos", &source);
    pad.add_control_binding(&binding)
        .context("add xpos control binding")?;
    Ok(())
}

// 2⁶² gates the f64→u64 cast.
const SCALE_MS_UPPER: f64 = 4_611_686_018_427_387_904.0;

fn scale_ms(ms: u32, speed: f32) -> u64 {
    let scaled = (f64::from(ms) * f64::from(speed)).round();
    if scaled.is_finite() && (0.0..=SCALE_MS_UPPER).contains(&scaled) {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let out = scaled as u64;
        out
    } else {
        u64::from(ms)
    }
}

fn clock_time_from_duration(d: Duration) -> gst::ClockTime {
    gst::ClockTime::from_nseconds(u64::try_from(d.as_nanos()).unwrap_or(u64::MAX))
}
