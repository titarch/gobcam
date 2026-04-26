//! On-disk cache for static + animated emoji blobs, plus the HTTP
//! downloader that backfills it from the upstream Microsoft repos.
//!
//! Layout:
//! ```text
//! $XDG_CACHE_HOME/gobcam/
//! ├── previews/<id>.png    # 3D static, 256×256, ~30 KB each
//! └── animated/<id>.png    # APNG, ~70-300 KB each (lazy)
//! ```
//!
//! Atomic writes via `<dest>.tmp` + rename. Per-path locks
//! coordinate concurrent fetches of the same emoji so the
//! background bootstrap thread and an eager `Trigger` don't both
//! hit the network for the same file.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use tracing::debug;

use crate::assets::EmojiId;

const STATIC_BASE: &str = "https://raw.githubusercontent.com/microsoft/fluentui-emoji/main/";
const ANIMATED_BASE: &str =
    "https://media.githubusercontent.com/media/microsoft/fluentui-emoji-animated/main/";

/// Resolved on-disk cache root. `previews/` and `animated/` live underneath.
#[derive(Debug, Clone)]
pub(crate) struct CacheRoot {
    root: PathBuf,
}

impl CacheRoot {
    /// Pick `$XDG_CACHE_HOME/gobcam` when set, else `$HOME/.cache/gobcam`.
    /// Caller may override via [`Self::with_path`].
    pub(crate) fn resolve_default() -> Result<Self> {
        let base = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
            .context("neither XDG_CACHE_HOME nor HOME is set")?;
        Self::with_path(base.join("gobcam"))
    }

    pub(crate) fn with_path(root: PathBuf) -> Result<Self> {
        fs::create_dir_all(root.join("previews")).context("creating previews/ cache dir")?;
        fs::create_dir_all(root.join("animated")).context("creating animated/ cache dir")?;
        Ok(Self { root })
    }

    /// Cache root directory itself — used by the preview-frame
    /// writer to put a known file at the top level.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn preview_path(&self, id: &EmojiId) -> PathBuf {
        self.root.join("previews").join(format!("{id}.png"))
    }

    pub(crate) fn animated_path(&self, id: &EmojiId) -> PathBuf {
        self.root.join("animated").join(format!("{id}.png"))
    }
}

/// Blocking HTTP client + per-file mutex map.
#[derive(Debug)]
pub(crate) struct Downloader {
    client: reqwest::blocking::Client,
    locks: Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>,
}

impl Downloader {
    pub(crate) fn new() -> Result<Self> {
        let client = reqwest::blocking::Client::builder()
            .user_agent(concat!("gobcam/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .build()
            .context("building reqwest client")?;
        Ok(Self {
            client,
            locks: Mutex::new(HashMap::new()),
        })
    }

    /// Ensure `dest` exists on disk. If absent, fetch from `<base>/<upstream_path>`
    /// and write atomically. Concurrent calls for the same `dest` are
    /// serialized; the second caller observes the first's result and skips.
    pub(crate) fn ensure(&self, dest: &Path, base: Base, upstream_path: &str) -> Result<()> {
        if dest.exists() {
            return Ok(());
        }
        let lock = self.lock_for(dest);
        let _g = lock.lock().expect("file lock poisoned");
        if dest.exists() {
            return Ok(());
        }
        let url = url_for(base, upstream_path);
        debug!(%url, dest = %dest.display(), "downloading");
        self.fetch(dest, &url)
    }

    fn lock_for(&self, path: &Path) -> Arc<Mutex<()>> {
        let mut guard = self.locks.lock().expect("locks poisoned");
        Arc::clone(
            guard
                .entry(path.to_path_buf())
                .or_insert_with(|| Arc::new(Mutex::new(()))),
        )
    }

    fn fetch(&self, dest: &Path, url: &str) -> Result<()> {
        let response = self
            .client
            .get(url)
            .send()
            .with_context(|| format!("GET {url}"))?
            .error_for_status()
            .with_context(|| format!("HTTP status for {url}"))?;
        let bytes = response.bytes().context("reading response body")?;
        let tmp = with_extension(dest, "tmp");
        fs::write(&tmp, &bytes).with_context(|| format!("writing {}", tmp.display()))?;
        fs::rename(&tmp, dest)
            .with_context(|| format!("rename {} -> {}", tmp.display(), dest.display()))?;
        Ok(())
    }
}

/// Which upstream repo hosts a given asset.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Base {
    Static,
    Animated,
}

fn url_for(base: Base, path: &str) -> String {
    let prefix = match base {
        Base::Static => STATIC_BASE,
        Base::Animated => ANIMATED_BASE,
    };
    // The upstream paths contain spaces (e.g. "Person raising hand"). Encode
    // them; everything else in the GitHub paths is already URL-safe.
    let encoded = path.replace(' ', "%20");
    format!("{prefix}{encoded}")
}

fn with_extension(path: &Path, ext: &str) -> PathBuf {
    // `Path::with_extension` would replace `.png` with `.tmp`, losing the
    // suffix and risking collisions. Append instead.
    let mut p = path.as_os_str().to_owned();
    p.push(".");
    p.push(ext);
    PathBuf::from(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_construction_encodes_spaces() {
        assert_eq!(
            url_for(Base::Static, "assets/Person raising hand/3D/x.png"),
            "https://raw.githubusercontent.com/microsoft/fluentui-emoji/main/\
             assets/Person%20raising%20hand/3D/x.png"
        );
    }

    #[test]
    fn cache_root_with_path_creates_subdirs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = CacheRoot::with_path(tmp.path().join("g")).unwrap();
        assert!(tmp.path().join("g/previews").is_dir());
        assert!(tmp.path().join("g/animated").is_dir());
        let id = EmojiId::new("fire");
        assert!(root.preview_path(&id).ends_with("previews/fire.png"));
        assert!(root.animated_path(&id).ends_with("animated/fire.png"));
    }

    #[test]
    fn tmp_filename_appends_not_replaces() {
        let p = Path::new("/x/foo.png");
        assert_eq!(with_extension(p, "tmp"), Path::new("/x/foo.png.tmp"));
    }
}
