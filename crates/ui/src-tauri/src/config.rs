//! Persisted UI settings: input device, capture mode, preview flag.
//!
//! Lives at `$XDG_CONFIG_HOME/gobcam/config.json` (default
//! `~/.config/gobcam/config.json`). The UI is the source of truth —
//! it loads on startup, hands the values to the daemon as CLI args,
//! and atomically saves whenever `apply_settings` succeeds. The
//! daemon itself is stateless across launches.
//!
//! JSON over TOML deliberately: we already have `serde_json` in the
//! workspace, and the file is machine-written, machine-read.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::daemon::DaemonArgs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StoredConfig {
    pub input: PathBuf,
    pub output: PathBuf,
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub preview: bool,
}

impl Default for StoredConfig {
    fn default() -> Self {
        let d = DaemonArgs::default();
        Self {
            input: d.input,
            output: d.output,
            width: d.width,
            height: d.height,
            fps_num: d.fps_num,
            fps_den: d.fps_den,
            preview: d.preview,
        }
    }
}

impl From<&DaemonArgs> for StoredConfig {
    fn from(args: &DaemonArgs) -> Self {
        Self {
            input: args.input.clone(),
            output: args.output.clone(),
            width: args.width,
            height: args.height,
            fps_num: args.fps_num,
            fps_den: args.fps_den,
            preview: args.preview,
        }
    }
}

impl From<StoredConfig> for DaemonArgs {
    fn from(s: StoredConfig) -> Self {
        Self {
            input: s.input,
            output: s.output,
            width: s.width,
            height: s.height,
            fps_num: s.fps_num,
            fps_den: s.fps_den,
            preview: s.preview,
        }
    }
}

fn config_path() -> Result<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .context("neither XDG_CONFIG_HOME nor HOME is set")?;
    Ok(base.join("gobcam").join("config.json"))
}

/// Load the stored config; on any error (missing file, malformed
/// JSON, missing fields) fall back silently to defaults.
pub(crate) fn load() -> StoredConfig {
    let Ok(path) = config_path() else {
        return StoredConfig::default();
    };
    let Ok(bytes) = fs::read(&path) else {
        return StoredConfig::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        warn!(path = %path.display(), error = %e, "config malformed; using defaults");
        StoredConfig::default()
    })
}

/// Atomic save. Failures are logged but never bubble up — settings
/// persistence is best-effort and shouldn't break the running app.
pub(crate) fn save(config: &StoredConfig) {
    if let Err(e) = save_inner(config) {
        warn!(error = %e, "saving config");
    }
}

fn save_inner(config: &StoredConfig) -> Result<()> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    let json = serde_json::to_vec_pretty(config).context("serializing config")?;
    let tmp = with_tmp_extension(&path);
    fs::write(&tmp, &json).with_context(|| format!("writing {}", tmp.display()))?;
    fs::rename(&tmp, &path)
        .with_context(|| format!("renaming {} -> {}", tmp.display(), path.display()))?;
    Ok(())
}

fn with_tmp_extension(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".tmp");
    PathBuf::from(s)
}
