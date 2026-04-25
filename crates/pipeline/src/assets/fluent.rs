//! `Library` backed by the bundled Fluent catalog and an
//! on-disk cache. Static 3D previews are predownloaded by
//! [`super::bootstrap`]; animated APNGs are fetched lazily on the
//! first [`Library::lookup`] call.
//!
//! Skin-tone variants are not yet exposed by the cache — `Style ==
//! Animated|Render3D` with `SkinTone::None|Default` is the only
//! supported axis. Other styles return `None`.

use std::sync::Arc;

use gobcam_protocol::EmojiInfo;
use tracing::warn;

use super::cache::{Base, CacheRoot, Downloader};
use super::catalog::{Catalog, CatalogEntry};
use super::{EmojiId, Library, SkinTone, Source, Style, apng};

pub(crate) struct FluentLibrary {
    cache: CacheRoot,
    catalog: Arc<Catalog>,
    downloader: Arc<Downloader>,
}

impl FluentLibrary {
    pub(crate) const fn new(
        cache: CacheRoot,
        catalog: Arc<Catalog>,
        downloader: Arc<Downloader>,
    ) -> Self {
        Self {
            cache,
            catalog,
            downloader,
        }
    }

    fn ensure_static(&self, entry: &CatalogEntry) -> Option<std::path::PathBuf> {
        let dest = self.cache.preview_path(&EmojiId::new(entry.id.clone()));
        if entry.static_path.is_empty() {
            return None;
        }
        match self
            .downloader
            .ensure(&dest, Base::Static, &entry.static_path)
        {
            Ok(()) => Some(dest),
            Err(err) => {
                warn!(id = %entry.id, %err, "static preview download failed");
                None
            }
        }
    }

    fn ensure_animated(&self, entry: &CatalogEntry) -> Option<std::path::PathBuf> {
        if !entry.has_animated || entry.animated_path.is_empty() {
            return None;
        }
        let dest = self.cache.animated_path(&EmojiId::new(entry.id.clone()));
        match self
            .downloader
            .ensure(&dest, Base::Animated, &entry.animated_path)
        {
            Ok(()) => Some(dest),
            Err(err) => {
                warn!(id = %entry.id, %err, "animated download failed");
                None
            }
        }
    }
}

impl Library for FluentLibrary {
    fn lookup(&self, emoji: &EmojiId, style: Style, tone: SkinTone) -> Option<Source> {
        // The cache is single-tone for now. Reject explicit non-default
        // skin-tone requests; treat None and Default as equivalent.
        if !matches!(tone, SkinTone::None | SkinTone::Default) {
            return None;
        }
        let entry = self.catalog.get(emoji)?;

        match style {
            Style::Animated => {
                let path = self.ensure_animated(entry)?;
                match apng::load(&path) {
                    Ok(frames) => Some(Source::Animated(Arc::new(frames))),
                    Err(err) => {
                        warn!(?path, %err, "failed to decode APNG");
                        None
                    }
                }
            }
            Style::Render3D => {
                let path = self.ensure_static(entry)?;
                match image::open(&path) {
                    Ok(img) => Some(Source::StaticRaster(Arc::new(img.to_rgba8()))),
                    Err(err) => {
                        warn!(?path, %err, "failed to decode PNG");
                        None
                    }
                }
            }
            // SVG-backed styles still deferred.
            Style::Color | Style::Flat | Style::HighContrast => None,
        }
    }

    fn fallback_chain(&self) -> &[Style] {
        &[Style::Animated, Style::Render3D]
    }

    fn list(&self) -> Vec<EmojiInfo> {
        self.catalog
            .entries()
            .iter()
            .map(|e| EmojiInfo {
                id: e.id.clone(),
                name: e.name.clone(),
                glyph: e.glyph.clone(),
                group: e.group.clone(),
                keywords: e.keywords.clone(),
                has_animated: e.has_animated,
                preview_path: self.cache.preview_path(&EmojiId::new(e.id.clone())),
            })
            .collect()
    }
}
