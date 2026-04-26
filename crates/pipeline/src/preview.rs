//! Preview branch: receives JPEG-encoded frames from a `GStreamer`
//! `appsink` and writes each one atomically to the cache root so the
//! UI can `<img src=…>` it. Off-by-default; enabled via `--preview`.
//!
//! Atomic write = `fs::write(tmp)` + `fs::rename(tmp, dest)`. Same-FS
//! rename is atomic on Linux, so the UI never reads a half-written
//! file.
//!
//! Path: `<cache>/runtime-preview.jpg`. Living under the cache root
//! means Tauri's existing `assetProtocol.scope` (`$CACHE/gobcam/**`)
//! covers it without further config.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use gstreamer::{self as gst, prelude::*};
use gstreamer_app::{self as gst_app, AppSink};
use tracing::warn;

use crate::assets::cache::CacheRoot;

const FILENAME: &str = "runtime-preview.jpg";

/// Path the UI loads via `convertFileSrc`.
pub(crate) fn path(cache: &CacheRoot) -> PathBuf {
    cache.root().join(FILENAME)
}

/// Wire up the appsink that writes preview frames. The pipeline
/// builder provides an appsink named `preview`; we attach the
/// new-sample callback that does the file-write dance.
pub(crate) fn install(pipeline: &gst::Pipeline, cache: &CacheRoot) -> Result<()> {
    let sink = pipeline
        .by_name("preview")
        .context("preview branch's appsink not found")?
        .dynamic_cast::<AppSink>()
        .map_err(|_| anyhow::anyhow!("element named 'preview' is not an appsink"))?;

    let dest = Arc::new(path(cache));
    let tmp = Arc::new(dest.with_extension("jpg.tmp"));

    sink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Error)?;
                let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                if let Err(e) = write_atomic(&dest, &tmp, map.as_slice()) {
                    warn!(error = %e, "preview write failed");
                }
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );
    Ok(())
}

fn write_atomic(dest: &Path, tmp: &Path, bytes: &[u8]) -> Result<()> {
    fs::write(tmp, bytes).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(tmp, dest)
        .with_context(|| format!("renaming {} -> {}", tmp.display(), dest.display()))?;
    Ok(())
}
