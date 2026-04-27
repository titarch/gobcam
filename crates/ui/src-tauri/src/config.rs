//! Persisted UI settings at `$XDG_CONFIG_HOME/gobcam/config.json`.
//! The UI is the source of truth; the daemon receives values as CLI
//! args + IPC pushes and is stateless across launches.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use gobcam_protocol::AnimationConfig;
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
    #[serde(default = "default_slot_count")]
    pub slot_count: usize,
    #[serde(default = "default_slot_dim")]
    pub slot_dim: u32,
    #[serde(default)]
    pub animations: AnimationConfig,
    /// Most-recently-triggered emoji ids, head = most recent.
    #[serde(default)]
    pub recents: Vec<String>,
    #[serde(default)]
    pub favorites: Vec<String>,
    #[serde(default)]
    pub hotkey_toggle: Option<String>,
    #[serde(default)]
    pub hotkey_repeat: Option<String>,
    /// `None` deserialises as the default ("dark").
    #[serde(default)]
    pub color_scheme: Option<String>,
}

const fn default_slot_count() -> usize {
    48
}

const fn default_slot_dim() -> u32 {
    256
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
            slot_count: d.slot_count,
            slot_dim: d.slot_dim,
            animations: AnimationConfig::default(),
            recents: Vec::new(),
            favorites: Vec::new(),
            hotkey_toggle: None,
            hotkey_repeat: None,
            color_scheme: None,
        }
    }
}

impl StoredConfig {
    pub(crate) fn from_state(
        args: &DaemonArgs,
        prefs: &UiPrefs,
        animations: &AnimationConfig,
    ) -> Self {
        Self {
            input: args.input.clone(),
            output: args.output.clone(),
            width: args.width,
            height: args.height,
            fps_num: args.fps_num,
            fps_den: args.fps_den,
            preview: args.preview,
            slot_count: args.slot_count,
            slot_dim: args.slot_dim,
            animations: animations.clone(),
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
            slot_count: s.slot_count,
            slot_dim: s.slot_dim,
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

/// Load the stored config; on any error fall back silently to defaults.
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

/// Atomic save; failures are logged but not propagated.
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
