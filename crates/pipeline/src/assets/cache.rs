//! On-disk cache plus blocking HTTP downloader. Layout:
//! `$XDG_CACHE_HOME/gobcam/{previews,animated}/<id>.png`.
//! Writes are atomic (`<dest>.tmp` + rename); per-path locks dedupe
//! concurrent fetches.

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

#[derive(Debug, Clone)]
pub(crate) struct CacheRoot {
    root: PathBuf,
}

impl CacheRoot {
    /// `$XDG_CACHE_HOME/gobcam`, else `$HOME/.cache/gobcam`.
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

    pub(crate) fn preview_path(&self, id: &EmojiId) -> PathBuf {
        self.root.join("previews").join(format!("{id}.png"))
    }

    pub(crate) fn animated_path(&self, id: &EmojiId) -> PathBuf {
        self.root.join("animated").join(format!("{id}.png"))
    }
}

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

    /// Ensure `dest` exists. Concurrent calls for the same path are
    /// serialized; the second caller skips.
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
    // Upstream paths contain spaces; everything else is URL-safe.
    let encoded = path.replace(' ', "%20");
    format!("{prefix}{encoded}")
}

fn with_extension(path: &Path, ext: &str) -> PathBuf {
    // Append (not replace) so `.png.tmp` keeps the original suffix.
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
