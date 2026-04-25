//! Triggered overlays: build → attach → run for `duration` → detach cleanly.
//!
//! Detach pattern (the load-bearing tricky bit):
//!   1. Add an IDLE pad probe to the bin's ghost-src pad.
//!   2. When the probe fires (streaming thread, briefly), unlink the pads
//!      and dispatch the heavy teardown to a worker thread.
//!   3. Worker thread does `bin.set_state(Null)` (which can block waiting on
//!      the frame-pump to exit) → `pipeline.remove(&bin)` → release the
//!      compositor request pad.
//!
//! Doing the heavy teardown inside the probe deadlocks: the streaming thread
//! holds locks the state change is waiting for. The IDLE probe ensures no
//! buffer is in flight at unlink time so we don't drop a frame mid-emit.

use std::io::BufRead;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use gstreamer::{self as gst, prelude::*};
use tracing::{debug, error, info, warn};

use crate::assets::{EmojiId, Library, SkinTone, Source, Style};
use crate::overlay::Overlay;

#[allow(unused_imports)]
use anyhow::Context as _;

pub(crate) const DEFAULT_REACTION_DURATION: Duration = Duration::from_secs(3);

pub(crate) struct Reactor {
    pipeline: gst::Pipeline,
    compositor: gst::Element,
    library: Arc<dyn Library>,
    counter: AtomicU64,
}

impl Reactor {
    pub(crate) fn new(pipeline: gst::Pipeline, library: Arc<dyn Library>) -> Result<Self> {
        let compositor = pipeline
            .by_name("mix")
            .context("compositor 'mix' not found in pipeline")?;
        Ok(Self {
            pipeline,
            compositor,
            library,
            counter: AtomicU64::new(0),
        })
    }

    pub(crate) fn trigger(&self, emoji_id: &str, duration: Duration) -> Result<()> {
        let id = self.counter.fetch_add(1, Ordering::Relaxed);

        let (style, source) = self
            .library
            .resolve(&EmojiId::new(emoji_id), Style::Animated, SkinTone::None)
            .or_else(|| {
                self.library
                    .resolve(&EmojiId::new(emoji_id), Style::Animated, SkinTone::Default)
            })
            .with_context(|| format!("emoji '{emoji_id}' not found"))?;
        info!(emoji = emoji_id, ?style, id, "triggering reaction");

        let name_prefix = format!("react-{emoji_id}-{id}");
        let overlay = Overlay::build(&source, &name_prefix)?;
        let position = position_with_jitter(&source, id);
        let sink_pad = crate::pipeline::attach_overlay(&self.pipeline, &overlay, position)?;

        let pipeline = self.pipeline.clone();
        let compositor = self.compositor.clone();
        let elements = overlay.elements;
        thread::Builder::new()
            .name(format!("react-{id}-timer"))
            .spawn(move || {
                thread::sleep(duration);
                schedule_detach(&pipeline, &compositor, &elements, &sink_pad);
            })?;

        Ok(())
    }
}

/// Add an IDLE probe to the terminal element's src pad; on the next idle
/// moment unlink and hand the rest of teardown off to a worker thread.
fn schedule_detach(
    pipeline: &gst::Pipeline,
    compositor: &gst::Element,
    elements: &[gst::Element],
    sink_pad: &gst::Pad,
) {
    let Some(last) = elements.last() else {
        warn!("detach: empty element chain");
        return;
    };
    let Some(src_pad) = last.static_pad("src") else {
        warn!("detach: terminal element has no src pad");
        return;
    };

    // The `Fn` callback can't move state out, so we use Mutex<Option<...>>
    // and take() on the first probe fire.
    let state = Arc::new(Mutex::new(Some(DetachState {
        pipeline: pipeline.clone(),
        compositor: compositor.clone(),
        elements: elements.to_vec(),
        sink_pad: sink_pad.clone(),
    })));

    src_pad.add_probe(gst::PadProbeType::IDLE, move |pad, _info| {
        let Some(s) = state.lock().expect("poisoned").take() else {
            return gst::PadProbeReturn::Remove;
        };
        if pad.unlink(&s.sink_pad).is_err() {
            warn!("detach: unlink failed (already unlinked?)");
        }
        thread::Builder::new()
            .name("react-teardown".into())
            .spawn(move || finish_detach(&s))
            .map_err(|e| warn!(error = %e, "detach: failed to spawn teardown thread"))
            .ok();
        gst::PadProbeReturn::Remove
    });
}

struct DetachState {
    pipeline: gst::Pipeline,
    compositor: gst::Element,
    elements: Vec<gst::Element>,
    sink_pad: gst::Pad,
}

fn finish_detach(s: &DetachState) {
    // Set each element to NULL and remove from the pipeline. set_state can
    // return Async; querying the state with a short timeout blocks until the
    // transition completes, otherwise GStreamer logs "Trying to dispose
    // element X, but it is in PAUSED" criticals at drop time.
    for element in s.elements.iter().rev() {
        if let Err(e) = element.set_state(gst::State::Null) {
            debug!(error = %e, name = element.name().as_str(), "detach: set_state(Null) returned error");
        }
        let _ = element.state(gst::ClockTime::from_seconds(1));
        if let Err(e) = s.pipeline.remove(element) {
            debug!(error = ?e, name = element.name().as_str(), "detach: pipeline.remove failed");
        }
    }
    s.compositor.release_request_pad(&s.sink_pad);
    debug!("reaction torn down");
}

/// Bottom-right anchor with a deterministic per-trigger jitter so stacked
/// reactions don't render on the exact same pixels.
fn position_with_jitter(source: &Source, id: u64) -> (i32, i32) {
    const CANVAS: (i32, i32) = (1280, 720);
    const MARGIN: i32 = 32;
    const JITTER_OFFSET: i32 = 60;
    const JITTER_RANGE: u64 = 121; // JITTER_OFFSET * 2 + 1

    let (w, h) = source.dimensions();
    let w = i32::try_from(w).unwrap_or(CANVAS.0);
    let h = i32::try_from(h).unwrap_or(CANVAS.1);
    let jx =
        i32::try_from(id.wrapping_mul(2_654_435_761) % JITTER_RANGE).unwrap_or(0) - JITTER_OFFSET;
    let jy =
        i32::try_from(id.wrapping_mul(2_246_822_519) % JITTER_RANGE).unwrap_or(0) - JITTER_OFFSET;
    let x = (CANVAS.0 - w - MARGIN + jx).clamp(0, CANVAS.0 - w);
    let y = (CANVAS.1 - h - MARGIN + jy).clamp(0, CANVAS.1 - h);
    (x, y)
}

/// Spawn a thread that reads stdin lines and triggers a reaction per line.
/// EOF and read errors stop the reader cleanly without affecting the daemon.
pub(crate) fn spawn_stdin_reader(reactor: Arc<Reactor>) {
    thread::Builder::new()
        .name("react-stdin".into())
        .spawn(move || {
            let stdin = std::io::stdin();
            for line in stdin.lock().lines() {
                let Ok(line) = line else {
                    warn!("stdin read error; reader exiting");
                    return;
                };
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Err(e) = reactor.trigger(trimmed, DEFAULT_REACTION_DURATION) {
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

    #[test]
    fn jitter_stays_within_canvas() {
        let img = Arc::new(image::RgbaImage::from_pixel(256, 256, image::Rgba([0; 4])));
        let source = Source::StaticRaster(img);
        for id in 0..1000 {
            let (x, y) = position_with_jitter(&source, id);
            assert!(x >= 0 && x + 256 <= 1280, "x out of range: {x}");
            assert!(y >= 0 && y + 256 <= 720, "y out of range: {y}");
        }
    }
}
