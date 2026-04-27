//! Pre-allocated compositor slots: N permanent `appsrc → videoconvert →
//! queue → compositor` branches toggled via `Mutex` instead of reshaping
//! the graph at runtime.

use std::sync::Arc;
use std::sync::Condvar;
use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Instant;

use anyhow::{Context, Result};
use gstreamer::{self as gst, glib, prelude::*};
use gstreamer_app::{self as gst_app, AppSrc};
use serde_json::json;
use tracing::{debug, warn};

use crate::assets::{AnimatedFrame, AnimatedFrames};
use crate::profile;

/// Native asset size. Appsrc caps must match this so row stride aligns
/// with the buffer; declaring a smaller width shifts rows by
/// `(SOURCE_DIM − dim) × 4` bytes and produces diagonal banding. The
/// on-screen dim is enforced by a downstream `videoscale + capsfilter`.
const SOURCE_DIM: u32 = 256;

#[derive(Clone)]
pub(crate) struct Slot {
    idx: usize,
    sink_pad: gst::Pad,
    state: Arc<Mutex<SlotState>>,
    /// Pump parks here while idle; `try_activate` wakes it.
    wake: Arc<Condvar>,
    busy: Arc<AtomicBool>,
}

struct SlotState {
    frames: Arc<AnimatedFrames>,
    idx: usize,
    /// Monotonic across activate/deactivate; never regresses.
    next_pts: gst::ClockTime,
    /// Trigger id of the currently armed frames; `None` while idle.
    armed_id: Option<u64>,
    /// `true` between activate and the pump's first push of the new frames.
    first_push_pending: bool,
    started_at: Option<Instant>,
    speed_multiplier: f32,
}

impl Slot {
    pub(crate) fn build(
        pipeline: &gst::Pipeline,
        compositor: &gst::Element,
        idx: usize,
        dim: u32,
    ) -> Result<Self> {
        let appsrc = AppSrc::builder()
            .name(format!("slot-{idx}-src"))
            .caps(&rgba_caps(SOURCE_DIM))
            .format(gst::Format::Time)
            .is_live(true)
            .block(true)
            .stream_type(gst_app::AppStreamType::Stream)
            .build();
        // Single-frame queue so a freshly-armed slot lands on the next
        // frame instead of waiting for the stale transparent buffer.
        appsrc.set_property("max-buffers", 1_u64);
        let scale = named("videoscale", format!("slot-{idx}-sc"))?;
        let scale_caps = gst::ElementFactory::make("capsfilter")
            .name(format!("slot-{idx}-sc-caps"))
            .property("caps", rgba_caps(dim))
            .build()
            .context("creating slot capsfilter")?;
        let convert = named("videoconvert", format!("slot-{idx}-cv"))?;
        let queue = named("queue", format!("slot-{idx}-q"))?;
        // Default queue holds 1 s of buffers; cap at 1 and rely on
        // appsrc backpressure. Don't use `leaky` — it removes
        // backpressure and the pump CPU-spins.
        queue.set_property("max-size-buffers", 1_u32);
        queue.set_property("max-size-time", 0_u64);
        queue.set_property("max-size-bytes", 0_u32);

        pipeline.add_many([appsrc.upcast_ref(), &scale, &scale_caps, &convert, &queue])?;
        gst::Element::link_many([appsrc.upcast_ref(), &scale, &scale_caps, &convert, &queue])?;

        let sink_pad = compositor
            .request_pad_simple("sink_%u")
            .with_context(|| format!("slot {idx}: compositor refused sink pad"))?;
        sink_pad.set_property("alpha", 0.0_f64);
        // Pairs with compositor's `ignore-inactive-pads=true`: idle
        // pads drop out of the blend once their last buffer expires.
        sink_pad.set_property("max-last-buffer-repeat", 0_u64);
        queue
            .static_pad("src")
            .context("queue missing src pad")?
            .link(&sink_pad)?;

        let state = Arc::new(Mutex::new(SlotState {
            frames: transparent_frames(),
            idx: 0,
            next_pts: gst::ClockTime::ZERO,
            armed_id: None,
            first_push_pending: false,
            started_at: None,
            speed_multiplier: 1.0,
        }));
        let wake = Arc::new(Condvar::new());

        spawn_pump(idx, appsrc, state.clone(), wake.clone());

        Ok(Self {
            idx,
            sink_pad,
            state,
            wake,
            busy: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Atomically claim this slot if idle. On success the pump starts
    /// pushing `frames` and `alpha` flips to 1.
    pub(crate) fn try_activate(
        &self,
        frames: Arc<AnimatedFrames>,
        position: (i32, i32),
        id: u64,
        speed_multiplier: f32,
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
            s.first_push_pending = true;
            s.started_at = Some(Instant::now());
            s.speed_multiplier = speed_multiplier;
        }
        // Notify after dropping the state lock so the wakened thread
        // doesn't immediately re-block on it.
        self.wake.notify_one();
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

    /// `true` if the slot is still armed for trigger `id` (not preempted).
    pub(crate) fn is_active_for(&self, id: u64) -> bool {
        if !self.busy.load(Ordering::Acquire) {
            return false;
        }
        let s = self.state.lock().expect("slot state poisoned");
        s.armed_id == Some(id)
    }

    pub(crate) fn is_busy(&self) -> bool {
        self.busy.load(Ordering::Acquire)
    }

    /// Activation timestamp of the busy slot, or `None` if idle.
    pub(crate) fn started_at(&self) -> Option<Instant> {
        if !self.busy.load(Ordering::Acquire) {
            return None;
        }
        self.state.lock().expect("slot state poisoned").started_at
    }

    pub(crate) const fn idx(&self) -> usize {
        self.idx
    }

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
            s.first_push_pending = false;
            s.started_at = None;
            s.speed_multiplier = 1.0;
            prev
        };
        self.sink_pad.set_property("alpha", 0.0_f64);
        self.busy.store(false, Ordering::Release);
        profile::mark("slot.deactivate", json!({ "id": id, "slot_idx": self.idx }));
    }
}

fn rgba_caps(dim: u32) -> gst::Caps {
    let d = i32::try_from(dim).expect("dim fits in i32");
    gst::Caps::builder("video/x-raw")
        .field("format", "RGBA")
        .field("width", d)
        .field("height", d)
        .field("framerate", gst::Fraction::new(30, 1))
        .build()
}

fn named(factory: &str, name: impl Into<glib::GString>) -> Result<gst::Element> {
    gst::ElementFactory::make(factory)
        .name(name)
        .build()
        .with_context(|| format!("creating element {factory}"))
}

fn spawn_pump(idx: usize, appsrc: AppSrc, state: Arc<Mutex<SlotState>>, wake: Arc<Condvar>) {
    thread::Builder::new()
        .name(format!("slot-{idx}-pump"))
        .spawn(move || pump(idx, &appsrc, &state, &wake))
        .expect("spawn slot pump");
}

/// Push one transparent seed buffer to negotiate caps, then park on
/// the condvar. Each `try_activate` wakes the pump to push the emoji's
/// frames until disarmed.
fn pump(idx: usize, appsrc: &AppSrc, state: &Arc<Mutex<SlotState>>, wake: &Condvar) {
    debug!(idx, "slot pump started");

    if push_one(appsrc, state).is_err() {
        return;
    }

    loop {
        {
            let mut s = state.lock().expect("slot state poisoned");
            while s.armed_id.is_none() {
                s = wake.wait(s).expect("condvar wait poisoned");
            }
            drop(s);
        }

        loop {
            let frame = {
                let mut s = state.lock().expect("slot state poisoned");
                if s.armed_id.is_none() {
                    break;
                }
                let frames = s.frames.clone();
                let frame_idx = s.idx;
                let pts = s.next_pts;
                let dur = duration_of(&frames.frames[frame_idx], s.speed_multiplier);
                s.next_pts += dur;
                s.idx = (s.idx + 1) % frames.frames.len();
                let first_push_id = if s.first_push_pending {
                    s.first_push_pending = false;
                    s.armed_id
                } else {
                    None
                };
                drop(s);
                (frames, frame_idx, pts, dur, first_push_id)
            };
            let (frames, frame_idx, pts, dur, first_push_id) = frame;

            let memory = frames.cached_memory(frame_idx);
            let mut buffer = gst::Buffer::new();
            {
                let bm = buffer.get_mut().expect("fresh buffer is unique");
                bm.append_memory(memory);
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
}

/// Push frame 0 once; advances `next_pts` to keep PTS monotonic.
fn push_one(
    appsrc: &AppSrc,
    state: &Arc<Mutex<SlotState>>,
) -> std::result::Result<(), gst::FlowError> {
    let (frames, pts, dur) = {
        let mut s = state.lock().expect("slot state poisoned");
        let frames = s.frames.clone();
        let pts = s.next_pts;
        let dur = duration_of(&frames.frames[0], s.speed_multiplier);
        s.next_pts += dur;
        drop(s);
        (frames, pts, dur)
    };
    let memory = frames.cached_memory(0);
    let mut buffer = gst::Buffer::new();
    {
        let bm = buffer.get_mut().expect("fresh buffer is unique");
        bm.append_memory(memory);
        bm.set_pts(pts);
        bm.set_duration(dur);
    }
    appsrc.push_buffer(buffer).map(|_| ())
}

fn duration_of(frame: &AnimatedFrame, speed_multiplier: f32) -> gst::ClockTime {
    let nanos = frame.delay.as_nanos();
    let scaled = if speed_multiplier > 0.0 {
        #[allow(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss
        )]
        let ns = (nanos as f64 / f64::from(speed_multiplier)).round() as u128;
        ns
    } else {
        nanos
    };
    gst::ClockTime::from_nseconds(u64::try_from(scaled).unwrap_or(u64::MAX))
}

/// Process-wide transparent seed frame at [`SOURCE_DIM`]. 33 ms duration
/// keeps activation latency low under `max-size-buffers=1` backpressure;
/// the singleton lets all idle pumps share one cached `gst::Memory`.
fn transparent_frames() -> Arc<AnimatedFrames> {
    static SINGLETON: OnceLock<Arc<AnimatedFrames>> = OnceLock::new();
    Arc::clone(SINGLETON.get_or_init(|| {
        let img = image::RgbaImage::from_pixel(SOURCE_DIM, SOURCE_DIM, image::Rgba([0, 0, 0, 0]));
        let frame = AnimatedFrame {
            rgba: Arc::new(img),
            delay: std::time::Duration::from_millis(33),
        };
        Arc::new(AnimatedFrames::new(vec![frame]))
    }))
}

/// First idle slot; `None` if all busy.
#[allow(clippy::needless_lifetimes)] // borrow checker requires explicit binding
pub(crate) fn try_claim<'a>(
    slots: &'a [Slot],
    frames: &Arc<AnimatedFrames>,
    position: (i32, i32),
    id: u64,
    speed_multiplier: f32,
) -> Option<&'a Slot> {
    slots
        .iter()
        .find(|slot| slot.try_activate(frames.clone(), position, id, speed_multiplier))
}

/// Adapt a [`crate::assets::Source`] into pump frames. Static rasters
/// become a 1-frame loop at 30 fps; animated APNGs pass through.
pub(crate) fn source_to_frames(source: &crate::assets::Source) -> Arc<AnimatedFrames> {
    use crate::assets::Source;
    match source {
        Source::Animated(frames) => frames.clone(),
        Source::StaticRaster(img) => Arc::new(AnimatedFrames::new(vec![AnimatedFrame {
            rgba: Arc::clone(img),
            delay: std::time::Duration::from_millis(33),
        }])),
    }
}
