//! Live animation parameters for the cascading-emoji engine.
//! [`AnimationStore`] holds the [`AnimationConfig`]; the reactor
//! snapshots it at trigger time so in-flight config edits don't tear
//! apart an active curve. Sampler helpers live here too.

use std::sync::{Arc, RwLock};

use gobcam_protocol::{AnimationConfig, DropPolicy};
use rand::Rng;

/// `f32 → i32` round; inputs are clamped pixel-scale so overflow can't
/// actually fire.
#[allow(clippy::cast_possible_truncation, clippy::missing_const_for_fn)]
fn round_to_i32(v: f32) -> i32 {
    v.round() as i32
}

/// Shared, read-mostly home for the daemon's [`AnimationConfig`].
#[derive(Debug, Clone)]
pub(crate) struct AnimationStore {
    inner: Arc<RwLock<AnimationConfig>>,
}

impl AnimationStore {
    pub(crate) fn new(initial: AnimationConfig) -> Self {
        Self {
            inner: Arc::new(RwLock::new(initial)),
        }
    }

    pub(crate) fn snapshot(&self) -> AnimationConfig {
        self.inner
            .read()
            .expect("animation config poisoned")
            .clone()
    }

    /// Replace the config. Caller is expected to pre-validate.
    pub(crate) fn replace(&self, cfg: AnimationConfig) {
        *self.inner.write().expect("animation config poisoned") = cfg;
    }
}

/// One sampled instance: start/end position plus per-instance
/// speed factor that scales the lifetime.
#[derive(Debug, Clone, Copy)]
pub(crate) struct InstancePlan {
    pub start_x: i32,
    pub start_y: i32,
    pub end_x: i32,
    pub end_y: i32,
    pub speed_factor: f32,
}

/// Sample a one-shot layout for a fresh trigger.
pub(crate) fn plan_instance<R: Rng + ?Sized>(
    cfg: &AnimationConfig,
    canvas: (i32, i32),
    source: (i32, i32),
    rng: &mut R,
) -> InstancePlan {
    let (cw, ch) = canvas;
    let (sw, sh) = source;

    #[allow(clippy::cast_precision_loss)]
    let cw_f = cw as f32;
    let anchor_x = round_to_i32(cfg.start_x_fraction.clamp(0.0, 1.0) * cw_f);
    let jitter_max = cfg.x_jitter_px.max(0.0);
    let jitter = if jitter_max > 0.0 {
        rng.gen_range(-jitter_max..=jitter_max)
    } else {
        0.0
    };
    let start_x = (anchor_x - sw / 2 + round_to_i32(jitter)).clamp(0, (cw - sw).max(0));
    let start_y_offset = round_to_i32(cfg.start_y_offset_px.max(0.0));
    let start_y = (ch - sh - start_y_offset).clamp(0, (ch - sh).max(0));

    let angle_rad = cfg.direction_angle_deg.to_radians();
    let travel = cfg.travel_px.max(0.0);
    // Math-convention angle from +x axis CCW; screen y grows down,
    // so sin is negated (90° = straight up).
    let dx = round_to_i32(travel * angle_rad.cos());
    let dy = round_to_i32(-travel * angle_rad.sin());
    let end_x = (start_x + dx).clamp(0, (cw - sw).max(0));
    // end_y may go negative; the compositor clips off-canvas.
    let end_y = start_y + dy;

    let jitter_pct = cfg.speed_jitter_pct.clamp(0.0, 0.95);
    let speed_factor = if jitter_pct > 0.0 {
        rng.gen_range((1.0 - jitter_pct)..=(1.0 + jitter_pct))
    } else {
        1.0
    };

    InstancePlan {
        start_x,
        start_y,
        end_x,
        end_y,
        speed_factor,
    }
}

/// Clamp APNG playback rate. 0.0 deadlocks the pump (infinite delay).
pub(crate) const fn clamp_apng_speed(multiplier: f32) -> f32 {
    multiplier.clamp(0.1, 5.0)
}

/// Cap `max_concurrent` to the actual slot count.
pub(crate) fn effective_max_concurrent(cfg: &AnimationConfig, slot_count: usize) -> usize {
    (cfg.max_concurrent as usize).min(slot_count)
}

pub(crate) const fn is_drop_oldest(policy: DropPolicy) -> bool {
    matches!(policy, DropPolicy::DropOldest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    fn cfg() -> AnimationConfig {
        AnimationConfig::default()
    }

    #[test]
    fn plan_keeps_start_inside_canvas() {
        let mut rng = StdRng::seed_from_u64(42);
        for _ in 0..200 {
            let plan = plan_instance(&cfg(), (1280, 720), (256, 256), &mut rng);
            assert!(plan.start_x >= 0 && plan.start_x + 256 <= 1280);
            assert!(plan.start_y >= 0 && plan.start_y + 256 <= 720);
        }
    }

    #[test]
    fn straight_up_default_drives_y_negative_x_unchanged() {
        let mut rng = StdRng::seed_from_u64(7);
        let plan = plan_instance(&cfg(), (1280, 720), (256, 256), &mut rng);
        assert_eq!(plan.end_x, plan.start_x);
        // 480 px of travel upward.
        assert_eq!(plan.start_y - plan.end_y, 480);
    }

    #[test]
    fn zero_jitter_is_deterministic() {
        let mut c = cfg();
        c.x_jitter_px = 0.0;
        c.speed_jitter_pct = 0.0;
        let mut rng = StdRng::seed_from_u64(1);
        let a = plan_instance(&c, (1280, 720), (256, 256), &mut rng);
        let b = plan_instance(&c, (1280, 720), (256, 256), &mut rng);
        assert_eq!(a.start_x, b.start_x);
        assert_eq!(a.start_y, b.start_y);
        assert!((a.speed_factor - 1.0).abs() < f32::EPSILON);
        assert!((b.speed_factor - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn speed_factor_within_jitter_envelope() {
        let mut rng = StdRng::seed_from_u64(99);
        let c = cfg();
        for _ in 0..200 {
            let plan = plan_instance(&c, (1280, 720), (256, 256), &mut rng);
            assert!(plan.speed_factor >= 1.0 - c.speed_jitter_pct - 1e-3);
            assert!(plan.speed_factor <= 1.0 + c.speed_jitter_pct + 1e-3);
        }
    }

    #[test]
    fn clamp_apng_speed_pins_to_safe_range() {
        assert!((clamp_apng_speed(0.0) - 0.1).abs() < f32::EPSILON);
        assert!((clamp_apng_speed(100.0) - 5.0).abs() < f32::EPSILON);
        assert!((clamp_apng_speed(1.5) - 1.5).abs() < f32::EPSILON);
    }

    #[test]
    fn effective_max_concurrent_respects_slot_count() {
        let mut c = cfg();
        c.max_concurrent = 100;
        assert_eq!(effective_max_concurrent(&c, 16), 16);
        c.max_concurrent = 4;
        assert_eq!(effective_max_concurrent(&c, 16), 4);
    }

    #[test]
    fn store_snapshot_is_isolated_from_replace() {
        let store = AnimationStore::new(AnimationConfig::default());
        let snap_a = store.snapshot();
        store.replace(AnimationConfig {
            lifetime_ms: 9999,
            ..AnimationConfig::default()
        });
        let snap_b = store.snapshot();
        assert_eq!(snap_a.lifetime_ms, 5000);
        assert_eq!(snap_b.lifetime_ms, 9999);
    }
}
