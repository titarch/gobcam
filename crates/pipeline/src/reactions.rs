//! Pick an idle slot, activate it with the requested emoji's frames,
//! optionally schedule deactivation after a duration.

use std::io::BufRead;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use rand::SeedableRng;
use rand::rngs::StdRng;
use serde_json::json;
use tracing::{debug, error, info};

use crate::animation::{
    AnimationStore, InstancePlan, clamp_apng_speed, effective_max_concurrent, is_drop_oldest,
    plan_instance,
};
use crate::assets::{EmojiId, Library, SkinTone, Source, Style};
use crate::effects;
use crate::profile;
use crate::slots::{self, Slot};

pub(crate) struct Reactor {
    slots: Vec<Slot>,
    library: Arc<dyn Library>,
    config: AnimationStore,
    canvas: (i32, i32),
    counter: AtomicU64,
}

impl Reactor {
    pub(crate) fn new(
        slots: Vec<Slot>,
        library: Arc<dyn Library>,
        config: AnimationStore,
        canvas: (i32, i32),
    ) -> Self {
        Self {
            slots,
            library,
            config,
            canvas,
            counter: AtomicU64::new(0),
        }
    }

    pub(crate) fn library(&self) -> &Arc<dyn Library> {
        &self.library
    }

    pub(crate) const fn config(&self) -> &AnimationStore {
        &self.config
    }

    /// Activate an emoji on a free slot. `duration_override: None`
    /// uses the live animation config; `Some(_)` selects the always-on
    /// overlay path (no cascade animation).
    pub(crate) fn activate(
        &self,
        emoji_id: &str,
        duration_override: Option<Duration>,
    ) -> Result<()> {
        let id = self.counter.fetch_add(1, Ordering::Relaxed);
        profile::mark(
            "reactor.activate.enter",
            json!({ "id": id, "emoji": emoji_id }),
        );

        let cfg = self.config.snapshot();
        let (style, source) = self
            .library
            .resolve(&EmojiId::new(emoji_id), Style::Animated, SkinTone::None)
            .or_else(|| {
                self.library
                    .resolve(&EmojiId::new(emoji_id), Style::Animated, SkinTone::Default)
            })
            .with_context(|| format!("emoji '{emoji_id}' not found"))?;
        profile::mark(
            "reactor.activate.resolved",
            json!({ "id": id, "style": format!("{style:?}") }),
        );
        info!(emoji = emoji_id, ?style, id, "activating reaction");

        let frames = slots::source_to_frames(&source);
        profile::mark(
            "reactor.activate.frames_ready",
            json!({ "id": id, "frame_count": frames.frames.len() }),
        );

        let plan = sample_plan(&cfg, &source, self.canvas, id);

        let speed_multiplier = clamp_apng_speed(cfg.apng_speed_multiplier);
        let active_count = self.slots.iter().filter(|s| s.is_busy()).count();
        let max = effective_max_concurrent(&cfg, self.slots.len());
        if active_count >= max && duration_override.is_none() {
            if !is_drop_oldest(cfg.drop_policy) {
                profile::mark(
                    "reactor.drop_new",
                    json!({ "id": id, "active": active_count, "max": max }),
                );
                return Ok(());
            }
            if let Some(victim) = oldest_active(&self.slots) {
                profile::mark(
                    "reactor.drop_oldest",
                    json!({ "id": id, "slot_idx": victim.idx() }),
                );
                effects::clear(victim.sink_pad());
                victim.deactivate();
            }
        }

        let Some(slot) = slots::try_claim(
            &self.slots,
            &frames,
            (plan.start_x, plan.start_y),
            id,
            speed_multiplier,
        ) else {
            anyhow::bail!("all {} slots busy", self.slots.len());
        };

        if duration_override.is_some() {
            profile::mark(
                "reactor.activate.exit",
                json!({ "id": id, "overlay": true }),
            );
            return Ok(());
        }

        let lifetime = match effects::apply_cascade(slot.sink_pad(), &cfg, &plan, id) {
            Ok(d) => d,
            Err(e) => {
                tracing::warn!(id, error = %e, "applying cascade effects failed");
                Duration::from_millis(u64::from(cfg.lifetime_ms))
            }
        };

        let slot_for_timer = slot.clone();
        thread::Builder::new()
            .name(format!("react-{id}-timer"))
            .spawn(move || {
                thread::sleep(lifetime);
                if !slot_for_timer.is_active_for(id) {
                    profile::mark(
                        "reactor.deactivate.skipped",
                        json!({ "id": id, "reason": "preempted" }),
                    );
                    return;
                }
                profile::mark("reactor.deactivate.enter", json!({ "id": id }));
                effects::clear(slot_for_timer.sink_pad());
                slot_for_timer.deactivate();
                profile::mark("reactor.deactivate.exit", json!({ "id": id }));
                debug!(id, "reaction deactivated");
            })?;

        profile::mark("reactor.activate.exit", json!({ "id": id }));
        Ok(())
    }
}

fn sample_plan(
    cfg: &gobcam_protocol::AnimationConfig,
    source: &Source,
    canvas: (i32, i32),
    id: u64,
) -> InstancePlan {
    let (sw, sh) = source.dimensions();
    let sw = i32::try_from(sw).unwrap_or(canvas.0);
    let sh = i32::try_from(sh).unwrap_or(canvas.1);
    // Seed with id for reproducible captures.
    let mut rng = StdRng::seed_from_u64(id ^ 0x9E37_79B9_7F4A_7C15);
    plan_instance(cfg, canvas, (sw, sh), &mut rng)
}

fn oldest_active(slots: &[Slot]) -> Option<&Slot> {
    slots
        .iter()
        .filter_map(|s| s.started_at().map(|t| (t, s)))
        .min_by_key(|(t, _)| *t)
        .map(|(_, s)| s)
}

/// One reaction per line of stdin. EOF stops the reader.
pub(crate) fn spawn_stdin_reader(reactor: Arc<Reactor>) {
    thread::Builder::new()
        .name("react-stdin".into())
        .spawn(move || {
            let stdin = std::io::stdin();
            for line in stdin.lock().lines() {
                let Ok(line) = line else { return };
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Err(e) = reactor.activate(trimmed, None) {
                    error!(emoji = trimmed, error = %e, "trigger failed");
                }
            }
            debug!("stdin closed; reader exiting");
        })
        .expect("spawn stdin reader");
}

#[cfg(test)]
mod tests {
    use super::*;
    use gobcam_protocol::AnimationConfig;

    #[test]
    fn sample_plan_keeps_start_inside_canvas() {
        let img = Arc::new(image::RgbaImage::from_pixel(256, 256, image::Rgba([0; 4])));
        let source = Source::StaticRaster(img);
        let cfg = AnimationConfig::default();
        let canvas = (1280, 720);
        for id in 0..1000 {
            let plan = sample_plan(&cfg, &source, canvas, id);
            assert!(plan.start_x >= 0 && plan.start_x + 256 <= canvas.0);
            assert!(plan.start_y >= 0 && plan.start_y + 256 <= canvas.1);
        }
    }
}
