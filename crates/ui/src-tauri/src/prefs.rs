//! In-memory mirror of the UI's user-facing preferences (recents +
//! hotkey bindings). Persisted via [`crate::config`] alongside the
//! daemon args.
//!
//! Kept separate from `DaemonSupervisor` so changing a hotkey or
//! recording a recent doesn't fight the supervisor lock used during
//! daemon respawn (which can block for ~2 s on a hot mode change).

use crate::config::StoredConfig;

/// Cap on `UiPrefs::recents`. Twelve fits a `grid-cols-4` row of three
/// rows in the panel (which is what the UI renders) — small enough to
/// scan, large enough to cover a meeting's worth of reactions.
pub(crate) const RECENTS_LIMIT: usize = 12;

#[derive(Debug, Default, Clone)]
pub(crate) struct UiPrefs {
    /// Most-recently-triggered emoji ids, head = most recent.
    pub recents: Vec<String>,
    /// User-configurable global hotkey for "show / hide the panel".
    /// `None` = unbound (no shortcut registered).
    pub hotkey_toggle: Option<String>,
    /// User-configurable global hotkey for "re-trigger the most
    /// recent emoji". `None` = unbound.
    pub hotkey_repeat: Option<String>,
}

impl UiPrefs {
    /// Hydrate from a `StoredConfig` loaded off disk. Recents are
    /// truncated to [`RECENTS_LIMIT`] to defend against a hand-edited
    /// or older-version config that exceeded the cap.
    pub(crate) fn from_stored(stored: &StoredConfig) -> Self {
        let mut recents = stored.recents.clone();
        recents.truncate(RECENTS_LIMIT);
        Self {
            recents,
            hotkey_toggle: stored.hotkey_toggle.clone(),
            hotkey_repeat: stored.hotkey_repeat.clone(),
        }
    }

    /// Push `id` to the front of recents, deduping any prior
    /// occurrence and trimming to `RECENTS_LIMIT`. Returns `true` if
    /// the recents list actually changed (caller decides whether to
    /// persist).
    pub(crate) fn record(&mut self, id: &str) -> bool {
        if self.recents.first().map(String::as_str) == Some(id) {
            return false;
        }
        self.recents.retain(|x| x != id);
        self.recents.insert(0, id.to_string());
        self.recents.truncate(RECENTS_LIMIT);
        true
    }

    /// The most-recently-triggered emoji id, if any. Used by the
    /// "repeat last" hotkey.
    pub(crate) fn last(&self) -> Option<&str> {
        self.recents.first().map(String::as_str)
    }
}
