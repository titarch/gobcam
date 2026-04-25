//! Emoji asset abstraction. A `Library` maps an `(emoji, style, tone)` key
//! to a `Source` the pipeline can render. Multiple libraries (Fluent, future
//! Twemoji/OpenMoji) can coexist behind the same trait.

pub(crate) mod apng;
pub(crate) mod bootstrap;
pub(crate) mod cache;
pub(crate) mod catalog;
pub(crate) mod fluent;

use std::sync::Arc;
use std::time::Duration;

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

// Some variants are reserved for future styles (SVG-backed Color/Flat/HighContrast)
// but we want them in the type today so the Library trait stays stable.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Style {
    Animated,
    Render3D,
    Color,
    Flat,
    HighContrast,
}

// Skin-tone variants beyond None/Default are reserved for a future
// cache layer that downloads per-tone assets; the v1 cache only
// holds the Default tone.
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
    pub rgba: image::RgbaImage,
    pub delay: Duration,
}

pub(crate) struct AnimatedFrames {
    pub frames: Vec<AnimatedFrame>,
}

impl AnimatedFrames {
    #[must_use]
    pub(crate) fn dimensions(&self) -> (u32, u32) {
        self.frames[0].rgba.dimensions()
    }
}

#[derive(Clone)]
pub(crate) enum Source {
    StaticRaster(Arc<image::RgbaImage>),
    Animated(Arc<AnimatedFrames>),
    // Vector source reserved for future SVG support.
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

    /// Enumerate every emoji the library knows about. Used by the
    /// IPC `list_emoji` command to populate the UI catalog.
    fn list(&self) -> Vec<gobcam_protocol::EmojiInfo>;

    /// Resolve with fallback applied: try `preferred` first, then walk the chain.
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
