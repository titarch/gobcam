//! Hand-curated list of emoji ids hidden when "safe mode" is on.
//! Bundled at compile time so both the daemon and the UI agree on the
//! same set without coordinating at runtime.

use std::collections::HashSet;
use std::sync::OnceLock;

const DENYLIST_JSON: &str = include_str!("../../../assets/safe-mode-denylist.json");

/// Set of emoji ids hidden when safe mode is on.
#[must_use]
pub fn denied_ids() -> &'static HashSet<String> {
    static SET: OnceLock<HashSet<String>> = OnceLock::new();
    SET.get_or_init(|| {
        serde_json::from_str(DENYLIST_JSON).expect("bundled safe-mode-denylist.json must parse")
    })
}

/// `true` when `id` should be hidden under safe mode.
#[must_use]
pub fn is_denied(id: &str) -> bool {
    denied_ids().contains(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_denylist_parses() {
        let set = denied_ids();
        assert!(!set.is_empty(), "denylist suspiciously empty");
        assert!(
            set.contains("middle_finger"),
            "middle_finger expected in denylist"
        );
    }

    #[test]
    fn is_denied_matches_set() {
        assert!(is_denied("middle_finger"));
        assert!(!is_denied("fire"));
    }
}
