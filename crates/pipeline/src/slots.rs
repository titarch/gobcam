//! Pre-allocated compositor slots. The pipeline grows N permanent overlay
//! branches at build time — `appsrc → videoconvert → queue → compositor` —
//! and reactions toggle slot state via a `Mutex` rather than reshaping the
//! pipeline graph at runtime.
//!
//! Architecture validated end-to-end in
//! `crates/pipeline/examples/pg_v4l2_slots_probe.rs` (multi-slot v4l2 with
//! the caps-query firewall from `crate::firewall`).

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use anyhow::{Context, Result};
use gstreamer::{self as gst, glib, prelude::*};
use gstreamer_app::{self as gst_app, AppSrc};
use serde_json::json;
use tracing::{debug, warn};

use crate::assets::{AnimatedFrame, AnimatedFrames};
use crate::profile;

/// Slot dimensions; matches the Fluent emoji set's 256×256.
pub(crate) const SLOT_DIM: u32 = 256;

#[derive(Clone)]
pub(crate) struct Slot {
    idx: usize,
    sink_pad: gst::Pad,
    state: Arc<Mutex<SlotState>>,
    busy: Arc<AtomicBool>,
}

struct SlotState {
    /// Frames the pump pushes. Initially the transparent placeholder.
    frames: Arc<AnimatedFrames>,
    idx: usize,
    /// Monotonic PTS — never resets across activate/deactivate so
    /// downstream never sees a timestamp regression.
    next_pts: gst::ClockTime,
    /// Trigger id for the *current* armed frames. `None` while idle.
    /// Used by the pump to tag its `slot.first_push` profile event,
    /// and by the deactivate timer to skip if the slot has been
    /// re-armed by a more recent trigger (preemption).
    armed_id: Option<u64>,
    /// Emoji id currently armed on the slot. Read by `Reactor::activate`
    /// to implement same-emoji collapse — a re-trigger of the same
    /// emoji preempts the existing slot rather than stacking alongside.
    armed_emoji: Option<String>,
    /// `true` between activate and the pump's first push of the new
    /// frames; reset by the pump after pushing.
    first_push_pending: bool,
}

impl Slot {
    /// Build the slot's element chain, link to a fresh compositor sink
    /// pad with `alpha=0`, spawn the pump thread.
    pub(crate) fn build(
        pipeline: &gst::Pipeline,
        compositor: &gst::Element,
        idx: usize,
    ) -> Result<Self> {
        let appsrc = AppSrc::builder()
            .name(format!("slot-{idx}-src"))
            .caps(&rgba_caps())
            .format(gst::Format::Time)
            .is_live(true)
            .block(true)
            .stream_type(gst_app::AppStreamType::Stream)
            .build();
        // max-buffers=1 keeps the appsrc queue at a single frame so a
        // freshly-armed slot is consumed by the compositor on the next
        // frame instead of waiting for the prior (transparent) buffer
        // to drain through a 2-deep queue. Cuts ~33 ms (1/30 s) from
        // the activate→first-visible-frame path.
        appsrc.set_property("max-buffers", 1_u64);
        let convert = named("videoconvert", format!("slot-{idx}-cv"))?;
        let queue = named("queue", format!("slot-{idx}-q"))?;
        // GStreamer's default queue holds up to 1 s of buffers, which
        // showed up as ~986 ms avg slot-q element latency in
        // `GST_TRACERS=latency`. Cap depth at 1 buffer; backpressure
        // through appsrc throttles the pump to the compositor's
        // consumption rate. (We tried `leaky=downstream` to drop
        // stale buffers instead, but that removed all backpressure
        // and the pump CPU-spun.)
        queue.set_property("max-size-buffers", 1_u32);
        queue.set_property("max-size-time", 0_u64);
        queue.set_property("max-size-bytes", 0_u32);

        pipeline.add_many([appsrc.upcast_ref(), &convert, &queue])?;
        gst::Element::link_many([appsrc.upcast_ref(), &convert, &queue])?;

        let sink_pad = compositor
            .request_pad_simple("sink_%u")
            .with_context(|| format!("slot {idx}: compositor refused sink pad"))?;
        sink_pad.set_property("alpha", 0.0_f64);
        queue
            .static_pad("src")
            .context("queue missing src pad")?
            .link(&sink_pad)?;

        let state = Arc::new(Mutex::new(SlotState {
            frames: transparent_frames(),
            idx: 0,
            next_pts: gst::ClockTime::ZERO,
            armed_id: None,
            armed_emoji: None,
            first_push_pending: false,
        }));

        spawn_pump(idx, appsrc, state.clone());

        Ok(Self {
            idx,
            sink_pad,
            state,
            busy: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Atomically claim this slot if idle. On success, the slot's pump
    /// starts pushing the new frames immediately and `alpha` flips to 1.
    /// `id` is the originating trigger id, recorded for profile output.
    /// `emoji` is the emoji id that's now armed (used by
    /// [`Self::is_active_with`] for same-emoji preemption).
    pub(crate) fn try_activate(
        &self,
        frames: Arc<AnimatedFrames>,
        position: (i32, i32),
        id: u64,
        emoji: String,
    ) -> bool {
        if self
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return false;
        }
        profile::mark(
            "slot.try_activate.enter",
            json!({ "id": id, "slot_idx": self.idx }),
        );
        {
            let mut s = self.state.lock().expect("slot state poisoned");
            s.frames = frames;
            s.idx = 0;
            s.armed_id = Some(id);
            s.armed_emoji = Some(emoji);
            s.first_push_pending = true;
        }
        self.sink_pad.set_property("xpos", position.0);
        self.sink_pad.set_property("ypos", position.1);
        self.sink_pad.set_property("alpha", 1.0_f64);
        profile::mark(
            "slot.try_activate.exit",
            json!({
                "id": id,
                "slot_idx": self.idx,
                "xpos": position.0,
                "ypos": position.1,
                "alpha": 1.0,
            }),
        );
        true
    }

    /// `true` if the slot is busy and currently armed with `emoji`.
    pub(crate) fn is_active_with(&self, emoji: &str) -> bool {
        if !self.busy.load(Ordering::Acquire) {
            return false;
        }
        let s = self.state.lock().expect("slot state poisoned");
        s.armed_emoji.as_deref() == Some(emoji)
    }

    /// `true` if the slot's currently armed reaction is the one that
    /// trigger `id` produced. Lets a deactivate-timer skip cleanup if a
    /// later trigger has since taken over the slot.
    pub(crate) fn is_active_for(&self, id: u64) -> bool {
        if !self.busy.load(Ordering::Acquire) {
            return false;
        }
        let s = self.state.lock().expect("slot state poisoned");
        s.armed_id == Some(id)
    }

    /// Take over an already-armed slot for trigger `id` without
    /// dropping `alpha` or changing position. Used when a re-trigger
    /// of the same emoji preempts the slot — keeps the emoji visible
    /// at its current spot while the new fade-out timer extends.
    /// Restarts the animation index so the reaction "replays" from
    /// frame 0.
    pub(crate) fn rearm(&self, id: u64, frames: Arc<AnimatedFrames>) {
        let mut s = self.state.lock().expect("slot state poisoned");
        s.frames = frames;
        s.idx = 0;
        s.armed_id = Some(id);
        s.first_push_pending = true;
        // armed_emoji and busy stay set; alpha/xpos/ypos untouched.
    }

    pub(crate) const fn idx(&self) -> usize {
        self.idx
    }

    /// Compositor sink pad — exposed so the effects layer can install
    /// control bindings on `xpos`/`ypos`/`alpha`.
    pub(crate) const fn sink_pad(&self) -> &gst::Pad {
        &self.sink_pad
    }

    /// Release the slot back to idle. Idempotent.
    pub(crate) fn deactivate(&self) {
        let id = {
            let mut s = self.state.lock().expect("slot state poisoned");
            s.frames = transparent_frames();
            s.idx = 0;
            let prev = s.armed_id.take();
            s.armed_emoji = None;
            s.first_push_pending = false;
            prev
        };
        self.sink_pad.set_property("alpha", 0.0_f64);
        self.busy.store(false, Ordering::Release);
        profile::mark("slot.deactivate", json!({ "id": id, "slot_idx": self.idx }));
    }
}

fn rgba_caps() -> gst::Caps {
    let dim = i32::try_from(SLOT_DIM).expect("256 fits in i32");
    gst::Caps::builder("video/x-raw")
        .field("format", "RGBA")
        .field("width", dim)
        .field("height", dim)
        .field("framerate", gst::Fraction::new(30, 1))
        .build()
}

fn named(factory: &str, name: impl Into<glib::GString>) -> Result<gst::Element> {
    gst::ElementFactory::make(factory)
        .name(name)
        .build()
        .with_context(|| format!("creating element {factory}"))
}

fn spawn_pump(idx: usize, appsrc: AppSrc, state: Arc<Mutex<SlotState>>) {
    thread::Builder::new()
        .name(format!("slot-{idx}-pump"))
        .spawn(move || pump(idx, &appsrc, &state))
        .expect("spawn slot pump");
}

fn pump(idx: usize, appsrc: &AppSrc, state: &Arc<Mutex<SlotState>>) {
    debug!(idx, "slot pump started");
    loop {
        let (frames, frame_idx, pts, first_push_id) = {
            let mut s = state.lock().expect("slot state poisoned");
            let frames = s.frames.clone();
            let frame_idx = s.idx;
            let pts = s.next_pts;
            let dur = duration_of(&frames.frames[frame_idx]);
            s.next_pts += dur;
            s.idx = (s.idx + 1) % frames.frames.len();
            let first_push_id = if s.first_push_pending {
                s.first_push_pending = false;
                s.armed_id
            } else {
                None
            };
            drop(s);
            (frames, frame_idx, pts, first_push_id)
        };
        let frame = &frames.frames[frame_idx];
        let raw = frame.rgba.as_raw();
        let dur = duration_of(frame);

        let Ok(mut buffer) = gst::Buffer::with_size(raw.len()) else {
            warn!(idx, "slot pump: buffer alloc failed");
            return;
        };
        {
            let bm = buffer.get_mut().expect("fresh buffer is unique");
            if bm.copy_from_slice(0, raw).is_err() {
                warn!(idx, "slot pump: short copy");
                return;
            }
            bm.set_pts(pts);
            bm.set_duration(dur);
        }
        match appsrc.push_buffer(buffer) {
            Ok(_) => {
                if let Some(id) = first_push_id {
                    profile::mark(
                        "slot.first_push",
                        json!({
                            "id": id,
                            "slot_idx": idx,
                            "pts_ns": pts.nseconds(),
                        }),
                    );
                }
            }
            Err(gst::FlowError::Flushing | gst::FlowError::Eos) => {
                debug!(idx, "slot pump exiting (flow stopped)");
                return;
            }
            Err(other) => {
                warn!(idx, error = ?other, "slot pump push_buffer failed");
                return;
            }
        }
    }
}

fn duration_of(frame: &AnimatedFrame) -> gst::ClockTime {
    gst::ClockTime::from_nseconds(u64::try_from(frame.delay.as_nanos()).unwrap_or(u64::MAX))
}

/// One transparent 256×256 RGBA frame at 33 ms. Used as the pump's source
/// while the slot is idle so the compositor's sink pad always has a buffer
/// to consume (alpha=0 makes it invisible).
fn transparent_frames() -> Arc<AnimatedFrames> {
    let img = image::RgbaImage::from_pixel(SLOT_DIM, SLOT_DIM, image::Rgba([0, 0, 0, 0]));
    let frame = AnimatedFrame {
        rgba: img,
        delay: std::time::Duration::from_millis(33),
    };
    Arc::new(AnimatedFrames {
        frames: vec![frame],
    })
}

/// Find the first idle slot and `try_activate` it. Returns `Some(slot)` on
/// success, `None` if every slot is currently busy.
#[allow(clippy::needless_lifetimes)] // borrow checker requires explicit binding
pub(crate) fn try_claim<'a>(
    slots: &'a [Slot],
    frames: &Arc<AnimatedFrames>,
    position: (i32, i32),
    id: u64,
    emoji: &str,
) -> Option<&'a Slot> {
    slots
        .iter()
        .find(|slot| slot.try_activate(frames.clone(), position, id, emoji.to_owned()))
}

/// Adapt an asset [`crate::assets::Source`] into a frame stream the pump
/// understands. Static rasters become a 1-frame `AnimatedFrames` looped at
/// 30 fps; animated APNGs pass through unchanged.
pub(crate) fn source_to_frames(source: &crate::assets::Source) -> Arc<AnimatedFrames> {
    use crate::assets::Source;
    match source {
        Source::Animated(frames) => frames.clone(),
        Source::StaticRaster(img) => Arc::new(AnimatedFrames {
            frames: vec![AnimatedFrame {
                rgba: (**img).clone(),
                delay: std::time::Duration::from_millis(33),
            }],
        }),
    }
}
