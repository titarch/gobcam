//! Persisted UI settings: input device, capture mode, preview flag,
//! recents, and global hotkeys.
//!
//! Lives at `$XDG_CONFIG_HOME/gobcam/config.json` (default
//! `~/.config/gobcam/config.json`). The UI is the source of truth —
//! it loads on startup, hands daemon-arg fields to the daemon as CLI
//! args, and atomically saves whenever something user-facing changes
//! (settings, hotkey bindings, recents). The daemon itself is
//! stateless across launches.
//!
//! New optional fields are added with `#[serde(default)]` so old
//! config files keep loading without manual migration.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::daemon::DaemonArgs;
use crate::prefs::UiPrefs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StoredConfig {
    pub input: PathBuf,
    pub output: PathBuf,
    pub width: u32,
    pub height: u32,
    pub fps_num: u32,
    pub fps_den: u32,
    pub preview: bool,
    /// Most-recently-triggered emoji ids, head = most recent.
    #[serde(default)]
    pub recents: Vec<String>,
    /// User-pinned emoji ids (ordered, no dedup).
    #[serde(default)]
    pub favorites: Vec<String>,
    /// User-configured hotkey for "show / hide the panel".
    #[serde(default)]
    pub hotkey_toggle: Option<String>,
    /// User-configured hotkey for "re-trigger the most recent emoji".
    #[serde(default)]
    pub hotkey_repeat: Option<String>,
    /// CSS `color-scheme` value. `None` deserialises as "dark" (backward compat).
    #[serde(default)]
    pub color_scheme: Option<String>,
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
            recents: Vec::new(),
            favorites: Vec::new(),
            hotkey_toggle: None,
            hotkey_repeat: None,
            color_scheme: None,
        }
    }
}

impl StoredConfig {
    /// Build a fresh `StoredConfig` from the current daemon args + UI
    /// prefs. Used by every save call site so we never accidentally
    /// drop a field.
    pub(crate) fn from_state(args: &DaemonArgs, prefs: &UiPrefs) -> Self {
        Self {
            input: args.input.clone(),
            output: args.output.clone(),
            width: args.width,
            height: args.height,
            fps_num: args.fps_num,
            fps_den: args.fps_den,
            preview: args.preview,
            recents: prefs.recents.clone(),
            favorites: prefs.favorites.clone(),
            hotkey_toggle: prefs.hotkey_toggle.clone(),
            hotkey_repeat: prefs.hotkey_repeat.clone(),
            color_scheme: Some(prefs.color_scheme.clone()),
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
