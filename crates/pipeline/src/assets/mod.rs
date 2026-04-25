//! Emoji asset abstraction. A `Library` maps an `(emoji, style, tone)` key
//! to a `Source` the pipeline can render. Multiple libraries (Fluent, future
//! Twemoji/OpenMoji) can coexist behind the same trait.

pub(crate) mod apng;
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

impl Style {
    /// Token used in our local asset path layout (also matches Fluent's lowercase form).
    #[must_use]
    pub(crate) const fn token(self) -> &'static str {
        match self {
            Self::Animated => "animated",
            Self::Render3D => "3d",
            Self::Color => "color",
            Self::Flat => "flat",
            Self::HighContrast => "high_contrast",
        }
    }
}

// Skin-tone variants beyond None/Default are reserved; the manifest can
// request them but the v1 daemon only consumes None/Default.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SkinTone {
    /// Emoji has no skin-tone variants (e.g. fire, heart).
    None,
    Default,
    Light,
    MediumLight,
    Medium,
    MediumDark,
    Dark,
}

impl SkinTone {
    #[must_use]
    pub(crate) const fn token(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Default => Some("default"),
            Self::Light => Some("light"),
            Self::MediumLight => Some("medium_light"),
            Self::Medium => Some("medium"),
            Self::MediumDark => Some("medium_dark"),
            Self::Dark => Some("dark"),
        }
    }
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
