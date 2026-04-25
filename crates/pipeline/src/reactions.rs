//! Reaction triggering: pick an idle slot, activate it with the requested
//! emoji's frames, optionally schedule deactivation after a duration.

use std::io::BufRead;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use tracing::{debug, error, info};

use crate::assets::{EmojiId, Library, SkinTone, Source, Style};
use crate::slots::{self, Slot};

pub(crate) const DEFAULT_REACTION_DURATION: Duration = Duration::from_secs(3);

pub(crate) struct Reactor {
    slots: Vec<Slot>,
    library: Arc<dyn Library>,
    counter: AtomicU64,
}

impl Reactor {
    pub(crate) fn new(slots: Vec<Slot>, library: Arc<dyn Library>) -> Self {
        Self {
            slots,
            library,
            counter: AtomicU64::new(0),
        }
    }

    /// Activate an emoji on a free slot. `duration: None` keeps it on
    /// indefinitely (always-on overlay path); `Some(d)` schedules
    /// deactivation after `d`.
    pub(crate) fn activate(&self, emoji_id: &str, duration: Option<Duration>) -> Result<()> {
        let id = self.counter.fetch_add(1, Ordering::Relaxed);
        let (style, source) = self
            .library
            .resolve(&EmojiId::new(emoji_id), Style::Animated, SkinTone::None)
            .or_else(|| {
                self.library
                    .resolve(&EmojiId::new(emoji_id), Style::Animated, SkinTone::Default)
            })
            .with_context(|| format!("emoji '{emoji_id}' not found"))?;
        info!(
            emoji = emoji_id,
            ?style,
            ?duration,
            id,
            "activating reaction"
        );

        let position = position_with_jitter(&source, id);
        let frames = slots::source_to_frames(&source);
        let Some(slot) = slots::try_claim(&self.slots, &frames, position) else {
            anyhow::bail!("all {} slots busy", self.slots.len());
        };

        if let Some(d) = duration {
            let slot = slot.clone();
            thread::Builder::new()
                .name(format!("react-{id}-timer"))
                .spawn(move || {
                    thread::sleep(d);
                    slot.deactivate();
                    debug!(id, "reaction deactivated");
                })?;
        }
        Ok(())
    }
}

/// Bottom-right anchor with a deterministic per-trigger jitter so stacked
/// reactions don't render on the exact same pixels.
fn position_with_jitter(source: &Source, id: u64) -> (i32, i32) {
    const CANVAS: (i32, i32) = (1280, 720);
    const MARGIN: i32 = 32;
    const JITTER_OFFSET: i32 = 60;
    const JITTER_RANGE: u64 = 121;

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
                let Ok(line) = line else { return };
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Err(e) = reactor.activate(trimmed, Some(DEFAULT_REACTION_DURATION)) {
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
