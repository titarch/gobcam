//! In-memory mirror of user-facing preferences (recents, favorites,
//! hotkey bindings, color scheme). Persisted via [`crate::config`].

use crate::config::StoredConfig;

pub(crate) const RECENTS_LIMIT: usize = 12;

const DEFAULT_COLOR_SCHEME: &str = "dark";

#[derive(Debug, Clone)]
pub(crate) struct UiPrefs {
    /// Most-recently-triggered emoji ids, head = most recent.
    pub recents: Vec<String>,
    pub favorites: Vec<String>,
    pub hotkey_toggle: Option<String>,
    pub hotkey_repeat: Option<String>,
    /// CSS `color-scheme`. "dark" forces dark widgets; "light dark"
    /// follows the system preference.
    pub color_scheme: String,
    /// When `true`, the picker hides emojis flagged as suggestive or
    /// rude (see `gobcam_protocol::safe_mode`).
    pub safe_mode: bool,
}

impl Default for UiPrefs {
    fn default() -> Self {
        Self {
            recents: Vec::new(),
            favorites: Vec::new(),
            hotkey_toggle: None,
            hotkey_repeat: None,
            color_scheme: DEFAULT_COLOR_SCHEME.to_string(),
            safe_mode: false,
        }
    }
}

impl UiPrefs {
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
            safe_mode: stored.safe_mode.unwrap_or(false),
        }
    }

    /// Push `id` to the front of recents (deduped, capped). Returns
    /// `true` if the list changed.
    pub(crate) fn record(&mut self, id: &str) -> bool {
        if self.recents.first().map(String::as_str) == Some(id) {
            return false;
        }
        self.recents.retain(|x| x != id);
        self.recents.insert(0, id.to_string());
        self.recents.truncate(RECENTS_LIMIT);
        true
    }

    /// Toggle `id` in favorites. Returns the new `is_favorite` state.
    pub(crate) fn toggle_favorite(&mut self, id: &str) -> bool {
        if let Some(pos) = self.favorites.iter().position(|x| x == id) {
            self.favorites.remove(pos);
            false
        } else {
            self.favorites.push(id.to_string());
            true
        }
    }

    pub(crate) fn last(&self) -> Option<&str> {
        self.recents.first().map(String::as_str)
    }
}
