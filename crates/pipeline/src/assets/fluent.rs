use std::path::PathBuf;
use std::sync::Arc;

use tracing::warn;

use super::{EmojiId, Library, SkinTone, Source, Style, apng};

/// `Library` that loads from the on-disk Fluent asset layout populated by
/// `scripts/sync-emoji.sh`:
///   tone-less:  <root>/<emoji>/<style>/<stub>_<style>.<ext>
///   with tone:  <root>/<emoji>/<tone>/<style>/<stub>_<style>_<tone>.<ext>
/// where `<stub>` matches the emoji id (Fluent uses a parallel naming).
pub(crate) struct FluentLibrary {
    root: PathBuf,
}

impl FluentLibrary {
    pub(crate) fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn file_path(&self, emoji: &EmojiId, style: Style, tone: SkinTone) -> PathBuf {
        let style_tok = style.token();
        let ext = match style {
            Style::Animated | Style::Render3D => "png",
            Style::Color | Style::Flat | Style::HighContrast => "svg",
        };
        let stub = emoji.as_str();
        let mut path = self.root.join(stub);
        let filename = match tone.token() {
            None => {
                path.push(style_tok);
                format!("{stub}_{style_tok}.{ext}")
            }
            Some(tone_tok) => {
                path.push(tone_tok);
                path.push(style_tok);
                format!("{stub}_{style_tok}_{tone_tok}.{ext}")
            }
        };
        path.push(filename);
        path
    }
}

impl Library for FluentLibrary {
    fn lookup(&self, emoji: &EmojiId, style: Style, tone: SkinTone) -> Option<Source> {
        let path = self.file_path(emoji, style, tone);
        if !path.exists() {
            return None;
        }
        match style {
            Style::Animated => match apng::load(&path) {
                Ok(frames) => Some(Source::Animated(Arc::new(frames))),
                Err(err) => {
                    warn!(?path, %err, "failed to decode APNG");
                    None
                }
            },
            Style::Render3D => match image::open(&path) {
                Ok(img) => Some(Source::StaticRaster(Arc::new(img.to_rgba8()))),
                Err(err) => {
                    warn!(?path, %err, "failed to decode PNG");
                    None
                }
            },
            // SVG-backed styles deferred to a future step.
            Style::Color | Style::Flat | Style::HighContrast => None,
        }
    }

    fn fallback_chain(&self) -> &[Style] {
        &[Style::Animated, Style::Render3D]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn file_path_with_tone() {
        let lib = FluentLibrary::new("/x");
        let p = lib.file_path(
            &EmojiId::new("thumbs_up"),
            Style::Render3D,
            SkinTone::Default,
        );
        assert_eq!(
            p,
            PathBuf::from("/x/thumbs_up/default/3d/thumbs_up_3d_default.png")
        );
    }

    #[test]
    fn file_path_no_tone() {
        let lib = FluentLibrary::new("/x");
        let p = lib.file_path(&EmojiId::new("fire"), Style::Animated, SkinTone::None);
        assert_eq!(p, PathBuf::from("/x/fire/animated/fire_animated.png"));
    }

    #[test]
    fn lookup_returns_none_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let lib = FluentLibrary::new(tmp.path());
        assert!(
            lib.lookup(&EmojiId::new("nope"), Style::Render3D, SkinTone::None)
                .is_none()
        );
    }

    #[test]
    fn resolve_walks_fallback_chain() {
        let tmp = tempfile::tempdir().unwrap();
        let lib = FluentLibrary::new(tmp.path());
        let png = tmp.path().join("fire/3d/fire_3d.png");
        fs::create_dir_all(png.parent().unwrap()).unwrap();
        image::RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 0]))
            .save(&png)
            .unwrap();

        let (style, src) = lib
            .resolve(&EmojiId::new("fire"), Style::Animated, SkinTone::None)
            .expect("fallback should land on Render3D");
        assert_eq!(style, Style::Render3D);
        assert!(matches!(src, Source::StaticRaster(_)));
    }
}
