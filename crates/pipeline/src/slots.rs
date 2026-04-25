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
use tracing::{debug, warn};

use crate::assets::{AnimatedFrame, AnimatedFrames};

/// Slot dimensions; matches the Fluent emoji set's 256×256.
pub(crate) const SLOT_DIM: u32 = 256;

#[derive(Clone)]
pub(crate) struct Slot {
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
        appsrc.set_property("max-buffers", 2_u64);
        let convert = named("videoconvert", format!("slot-{idx}-cv"))?;
        let queue = named("queue", format!("slot-{idx}-q"))?;

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
        }));

        spawn_pump(idx, appsrc, state.clone());

        Ok(Self {
            sink_pad,
            state,
            busy: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Atomically claim this slot if idle. On success, the slot's pump
    /// starts pushing the new frames immediately and `alpha` flips to 1.
    pub(crate) fn try_activate(&self, frames: Arc<AnimatedFrames>, position: (i32, i32)) -> bool {
        if self
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return false;
        }
        {
            let mut s = self.state.lock().expect("slot state poisoned");
            s.frames = frames;
            s.idx = 0;
        }
        self.sink_pad.set_property("xpos", position.0);
        self.sink_pad.set_property("ypos", position.1);
        self.sink_pad.set_property("alpha", 1.0_f64);
        true
    }

    /// Compositor sink pad — exposed so the effects layer can install
    /// control bindings on `xpos`/`ypos`/`alpha`.
    pub(crate) const fn sink_pad(&self) -> &gst::Pad {
        &self.sink_pad
    }

    /// Release the slot back to idle. Idempotent.
    pub(crate) fn deactivate(&self) {
        {
            let mut s = self.state.lock().expect("slot state poisoned");
            s.frames = transparent_frames();
            s.idx = 0;
        }
        self.sink_pad.set_property("alpha", 0.0_f64);
        self.busy.store(false, Ordering::Release);
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
        let (frames, frame_idx, pts) = {
            let mut s = state.lock().expect("slot state poisoned");
            let frames = s.frames.clone();
            let frame_idx = s.idx;
            let pts = s.next_pts;
            let dur = duration_of(&frames.frames[frame_idx]);
            s.next_pts += dur;
            s.idx = (s.idx + 1) % frames.frames.len();
            drop(s);
            (frames, frame_idx, pts)
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
            Ok(_) => {}
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
) -> Option<&'a Slot> {
    slots
        .iter()
        .find(|slot| slot.try_activate(frames.clone(), position))
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
