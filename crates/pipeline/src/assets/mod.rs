//! Emoji asset abstraction. A `Library` maps an `(emoji, style, tone)`
//! key to a `Source` the pipeline can render.

pub(crate) mod apng;
pub(crate) mod bootstrap;
pub(crate) mod cache;
pub(crate) mod catalog;
pub(crate) mod fluent;

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use gstreamer as gst;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct EmojiId(String);

impl EmojiId {
    pub(crate) fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
    #[must_use]
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for EmojiId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// SVG-backed styles are not yet wired through the cache.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Style {
    Animated,
    Render3D,
    Color,
    Flat,
    HighContrast,
}

// Per-tone assets aren't downloaded yet; only None/Default work.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SkinTone {
    None,
    Default,
    Light,
    MediumLight,
    Medium,
    MediumDark,
    Dark,
}

pub(crate) struct AnimatedFrame {
    pub rgba: Arc<image::RgbaImage>,
    pub delay: Duration,
}

pub(crate) struct AnimatedFrames {
    pub frames: Vec<AnimatedFrame>,
    /// Shared `gst::Memory` wrappers (`LOCK_READONLY`) parallel to
    /// `frames`. Read-only across slots; downstream `make_mut` triggers
    /// a deep copy in `GStreamer` rather than corrupting shared bytes.
    cached_memory: Vec<OnceLock<gst::Memory>>,
}

impl AnimatedFrames {
    #[must_use]
    pub(crate) fn new(frames: Vec<AnimatedFrame>) -> Self {
        let cached_memory = (0..frames.len()).map(|_| OnceLock::new()).collect();
        Self {
            frames,
            cached_memory,
        }
    }

    #[must_use]
    pub(crate) fn dimensions(&self) -> (u32, u32) {
        self.frames[0].rgba.dimensions()
    }

    /// Shared `gst::Memory` for `frame_idx`, built on first access.
    /// Returned clones are refcount-bumps of the same block;
    /// `from_slice` sets `LOCK_READONLY` so downstream `make_mut`
    /// deep-copies instead of corrupting the shared bytes.
    #[must_use]
    pub(crate) fn cached_memory(&self, frame_idx: usize) -> gst::Memory {
        self.cached_memory[frame_idx]
            .get_or_init(|| {
                let bytes = FrameBytes(Arc::clone(&self.frames[frame_idx].rgba));
                gst::Memory::from_slice(bytes)
            })
            .clone()
    }
}

/// `Arc<RgbaImage>` exposed as `&[u8]` for `gst::Memory::from_slice`.
/// The Arc is held for the Memory's lifetime.
struct FrameBytes(Arc<image::RgbaImage>);

impl AsRef<[u8]> for FrameBytes {
    fn as_ref(&self) -> &[u8] {
        self.0.as_raw()
    }
}

#[derive(Clone)]
pub(crate) enum Source {
    StaticRaster(Arc<image::RgbaImage>),
    Animated(Arc<AnimatedFrames>),
}

impl Source {
    #[must_use]
    pub(crate) fn dimensions(&self) -> (u32, u32) {
        match self {
            Self::StaticRaster(img) => img.dimensions(),
            Self::Animated(frames) => frames.dimensions(),
        }
    }
}

pub(crate) trait Library: Send + Sync {
    fn lookup(&self, emoji: &EmojiId, style: Style, tone: SkinTone) -> Option<Source>;
    fn fallback_chain(&self) -> &[Style];

    /// Enumerate every emoji the library knows about.
    fn list(&self) -> Vec<gobcam_protocol::EmojiInfo>;

    /// Try `preferred`, then walk the fallback chain.
    fn resolve(
        &self,
        emoji: &EmojiId,
        preferred: Style,
        tone: SkinTone,
    ) -> Option<(Style, Source)> {
        if let Some(src) = self.lookup(emoji, preferred, tone) {
            return Some((preferred, src));
        }
        for &s in self.fallback_chain() {
            if s == preferred {
                continue;
            }
            if let Some(src) = self.lookup(emoji, s, tone) {
                return Some((s, src));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_memory_shares_one_underlying_block() {
        gst::init().expect("gst init");

        let img = Arc::new(image::RgbaImage::from_pixel(
            8,
            8,
            image::Rgba([1, 2, 3, 4]),
        ));
        let frames = AnimatedFrames::new(vec![AnimatedFrame {
            rgba: img,
            delay: Duration::from_millis(33),
        }]);

        let m1 = frames.cached_memory(0);
        let m2 = frames.cached_memory(0);

        let map1 = m1.map_readable().expect("map m1");
        let map2 = m2.map_readable().expect("map m2");

        // Same underlying pointer proves clone is a refcount bump.
        assert_eq!(map1.as_slice().as_ptr(), map2.as_slice().as_ptr());
        assert_eq!(map1.size(), 8 * 8 * 4);
        assert_eq!(map1.as_slice()[0..4], [1, 2, 3, 4]);
    }
}
