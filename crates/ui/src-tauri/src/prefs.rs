//! In-memory mirror of the UI's user-facing preferences (recents +
//! hotkey bindings). Persisted via [`crate::config`] alongside the
//! daemon args.
//!
//! Kept separate from `DaemonSupervisor` so changing a hotkey or
//! recording a recent doesn't fight the supervisor lock used during
//! daemon respawn (which can block for ~2 s on a hot mode change).

use crate::config::StoredConfig;

/// Cap on `UiPrefs::recents`.
pub(crate) const RECENTS_LIMIT: usize = 12;

/// Default `color-scheme` sent to system widgets.
const DEFAULT_COLOR_SCHEME: &str = "dark";

#[derive(Debug, Clone)]
pub(crate) struct UiPrefs {
    /// Most-recently-triggered emoji ids, head = most recent.
    pub recents: Vec<String>,
    /// User-pinned emoji ids (order preserved, no dedup needed here).
    pub favorites: Vec<String>,
    /// User-configurable global hotkey for "show / hide the panel".
    pub hotkey_toggle: Option<String>,
    /// User-configurable global hotkey for "re-trigger the most recent emoji".
    pub hotkey_repeat: Option<String>,
    /// CSS `color-scheme` value applied to the document root.
    /// "dark" = force dark OS widgets; "light dark" = follow system preference.
    pub color_scheme: String,
}

impl Default for UiPrefs {
    fn default() -> Self {
        Self {
            recents: Vec::new(),
            favorites: Vec::new(),
            hotkey_toggle: None,
            hotkey_repeat: None,
            color_scheme: DEFAULT_COLOR_SCHEME.to_string(),
        }
    }
}

impl UiPrefs {
    /// Hydrate from a `StoredConfig` loaded off disk.
    pub(crate) fn from_stored(stored: &StoredConfig) -> Self {
        let mut recents = stored.recents.clone();
        recents.truncate(RECENTS_LIMIT);
        Self {
            recents,
            favorites: stored.favorites.clone(),
            hotkey_toggle: stored.hotkey_toggle.clone(),
            hotkey_repeat: stored.hotkey_repeat.clone(),
            color_scheme: stored
                .color_scheme
                .clone()
                .unwrap_or_else(|| DEFAULT_COLOR_SCHEME.to_string()),
        }
    }

    /// Push `id` to the front of recents, deduping any prior occurrence and
    /// trimming to `RECENTS_LIMIT`. Returns `true` if the list changed.
    pub(crate) fn record(&mut self, id: &str) -> bool {
        if self.recents.first().map(String::as_str) == Some(id) {
            return false;
        }
        self.recents.retain(|x| x != id);
        self.recents.insert(0, id.to_string());
        self.recents.truncate(RECENTS_LIMIT);
        true
    }

    /// Toggle `id` in the favorites list. Returns the new `is_favorite` state.
    pub(crate) fn toggle_favorite(&mut self, id: &str) -> bool {
        if let Some(pos) = self.favorites.iter().position(|x| x == id) {
            self.favorites.remove(pos);
            false
        } else {
            self.favorites.push(id.to_string());
            true
        }
    }

    /// The most-recently-triggered emoji id, if any. Used by the repeat-last hotkey.
    pub(crate) fn last(&self) -> Option<&str> {
        self.recents.first().map(String::as_str)
    }
}
