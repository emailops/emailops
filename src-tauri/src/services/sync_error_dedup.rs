//! In-memory per-account dedup for repeated sync-error log lines.
//!
//! Extracted from `services::sync_scheduler` so that callers which are not part of the
//! desktop scheduler — `services::accounts` clears it after re-authentication — do not
//! have to depend on a module that is compiled only for the Tauri build.
//!
//! Stores the last error message emitted to the output panel per key, suppressing
//! repeated identical "auto-sync failed" logs every 60 s when the underlying state
//! cannot change without user action (e.g. `NeedsReauth` — keyring entry missing).
//! Cleared on the first successful sync after recovery, or explicitly after
//! re-authentication.
//!
//! Keys are either a bare `account_id` (email sync) or `calendar:{account_id}` so the
//! two streams dedup independently.
//!
//! Process-global by nature. In a multi-user server this is shared across users, which
//! is acceptable — the values are error strings keyed by account id, and account ids are
//! unique per user store. It is deliberately *not* part of the per-user context.

use std::collections::HashMap;
use std::sync::{PoisonError, RwLock};

pub(crate) static LAST_SYNC_ERROR: std::sync::LazyLock<RwLock<HashMap<String, String>>> =
    std::sync::LazyLock::new(|| RwLock::new(HashMap::new()));

/// Drop the dedup state for an account so the next sync attempt logs/emits
/// regardless of prior error history. Call after re-authentication so the
/// user sees a fresh log line if a different problem now surfaces.
pub fn clear_sync_error_dedup(account_id: &str) {
    let mut guard = LAST_SYNC_ERROR.write().unwrap_or_else(PoisonError::into_inner);
    guard.remove(account_id);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialises the tests in this module — they all mutate the same process-global map.
    fn test_lock() -> std::sync::MutexGuard<'static, ()> {
        static M: std::sync::Mutex<()> = std::sync::Mutex::new(());
        M.lock().unwrap_or_else(PoisonError::into_inner)
    }

    #[test]
    fn clear_removes_only_the_named_account() {
        let _guard = test_lock();
        {
            let mut map = LAST_SYNC_ERROR.write().unwrap_or_else(PoisonError::into_inner);
            map.insert("acct-a".to_string(), "boom".to_string());
            map.insert("acct-b".to_string(), "bang".to_string());
        }

        clear_sync_error_dedup("acct-a");

        let map = LAST_SYNC_ERROR.read().unwrap_or_else(PoisonError::into_inner);
        assert!(!map.contains_key("acct-a"));
        assert_eq!(map.get("acct-b").map(String::as_str), Some("bang"));
    }

    #[test]
    fn clear_is_a_no_op_for_an_unknown_account() {
        let _guard = test_lock();
        clear_sync_error_dedup("never-seen");
        // Nothing to assert beyond "did not panic"; the map simply has no such key.
        let map = LAST_SYNC_ERROR.read().unwrap_or_else(PoisonError::into_inner);
        assert!(!map.contains_key("never-seen"));
    }
}
