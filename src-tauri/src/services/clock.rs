//! Clock trait seam.
//!
//! Wraps "what time is it?" so tests can advance time deterministically
//! instead of relying on `chrono::Utc::now()` / `std::time::SystemTime::now()`.
//! Most services in the app stamp DB rows with `now_secs()` — without a seam
//! those timestamps drift with wall-clock during test runs, making time-based
//! assertions (e.g. "this task was created before that one") flaky.
//!
//! ## Production
//!
//! `SystemClock` reads `chrono::Utc::now()`. It is the default backend, so
//! production code that never installs anything still works.
//!
//! ## Tests
//!
//! `FixedClock` returns a constant instant unless explicitly advanced via
//! `set_now_secs(...)` / `advance_secs(...)`. Tests call
//! [`install_for_testing(seconds)`] in setup; this swaps in a FixedClock at
//! the provided unix-seconds timestamp.
//!
//! Note: all tests share the same global backend, so tests that exercise the
//! clock should serialize via a module-level mutex if they would otherwise
//! race.
//!
//! ## Migration note
//!
//! Roughly 130 sites still call `chrono::Utc::now()` / `SystemTime::now()`
//! directly. They keep working — the seam is opt-in. CLAUDE.md states the
//! end-state goal ("never call `Utc::now()` directly inside services"); we
//! migrate sites as we touch them rather than in a single sweep.

use std::sync::{Arc, PoisonError, RwLock};

/// Source of "now" for the app.
pub trait Clock: Send + Sync {
    /// Current unix-time in seconds. Almost every caller in this codebase
    /// stores i64 unix seconds in SQLite, so the trait surfaces only that.
    /// Sub-second precision is intentionally excluded — services that need
    /// it can fall back to `std::time::Instant` (monotonic, no wall-clock
    /// semantics needed).
    fn now_secs(&self) -> i64;

    /// Seconds to add to a UTC timestamp to get the user's wall-clock time.
    /// Every date the chat shows the model or parses from it ("today", a
    /// `since` bound, a message's date) is a day in THIS zone, not UTC.
    fn utc_offset_secs(&self) -> i32 {
        0
    }
}

/// Production clock: delegates to `chrono::Utc::now()`.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_secs(&self) -> i64 {
        chrono::Utc::now().timestamp()
    }

    fn utc_offset_secs(&self) -> i32 {
        chrono::Local::now().offset().local_minus_utc()
    }
}

/// Deterministic clock for tests. Mutate via `set_now_secs` / `advance_secs`.
pub struct FixedClock {
    now: RwLock<i64>,
    offset_secs: i32,
}

impl FixedClock {
    pub fn new(now_secs: i64) -> Self {
        Self::with_offset(now_secs, 0)
    }

    /// A pinned clock in a pinned zone — the offset is what makes local-day
    /// tests deterministic on any machine.
    pub fn with_offset(now_secs: i64, offset_secs: i32) -> Self {
        Self {
            now: RwLock::new(now_secs),
            offset_secs,
        }
    }

    /// Jump the clock to an absolute timestamp.
    pub fn set_now_secs(&self, t: i64) {
        *self.now.write().unwrap_or_else(PoisonError::into_inner) = t;
    }

    /// Advance the clock by `delta` seconds (may be negative — useful for
    /// simulating clock drift, but rarely needed).
    pub fn advance_secs(&self, delta: i64) {
        let mut guard = self.now.write().unwrap_or_else(PoisonError::into_inner);
        *guard += delta;
    }
}

impl Clock for FixedClock {
    fn now_secs(&self) -> i64 {
        *self.now.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn utc_offset_secs(&self) -> i32 {
        self.offset_secs
    }
}

// ── Global registry ──────────────────────────────────────────────────────────

static CLOCK: std::sync::LazyLock<RwLock<Arc<dyn Clock>>> =
    std::sync::LazyLock::new(|| RwLock::new(Arc::new(SystemClock)));

/// Active clock. Cheap to call; clones an Arc.
pub fn current() -> Arc<dyn Clock> {
    CLOCK.read().unwrap_or_else(PoisonError::into_inner).clone()
}

/// Replace the active clock. Called by tests; production code never needs
/// to touch this (the default `SystemClock` is installed at module init).
pub fn install(backend: Arc<dyn Clock>) {
    *CLOCK.write().unwrap_or_else(PoisonError::into_inner) = backend;
}

/// Convenience wrapper for the common "I just want unix seconds" call site.
/// Replaces `chrono::Utc::now().timestamp()`.
pub fn now_secs() -> i64 {
    current().now_secs()
}

/// The installed clock's UTC offset — see [`Clock::utc_offset_secs`].
pub fn utc_offset_secs() -> i32 {
    current().utc_offset_secs()
}

#[cfg(test)]
pub fn install_for_testing(initial_now_secs: i64) -> Arc<FixedClock> {
    let clock = Arc::new(FixedClock::new(initial_now_secs));
    install(clock.clone() as Arc<dyn Clock>);
    clock
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        static M: std::sync::Mutex<()> = std::sync::Mutex::new(());
        M.lock().unwrap_or_else(PoisonError::into_inner)
    }

    #[test]
    fn system_clock_reports_the_machine_utc_offset() {
        // The user's day, not UTC's: "today" at 01:00 in Madrid is still
        // yesterday in UTC, and every date-shaped prompt value must follow
        // the machine's zone.
        let expected = chrono::Local::now().offset().local_minus_utc();
        assert_eq!(SystemClock.utc_offset_secs(), expected);
    }

    #[test]
    fn fixed_clock_offset_defaults_to_zero_and_can_be_set() {
        assert_eq!(FixedClock::new(1_000).utc_offset_secs(), 0);
        assert_eq!(FixedClock::with_offset(1_000, 7_200).utc_offset_secs(), 7_200);
    }

    #[test]
    fn fixed_clock_is_deterministic() {
        let _g = lock();
        let c = install_for_testing(1_000);
        assert_eq!(now_secs(), 1_000);
        c.advance_secs(60);
        assert_eq!(now_secs(), 1_060);
        c.set_now_secs(5_000);
        assert_eq!(now_secs(), 5_000);
        install(Arc::new(SystemClock));
    }

    #[test]
    fn system_clock_returns_real_time() {
        let _g = lock();
        install(Arc::new(SystemClock));
        let t = now_secs();
        // Should be after 2024-01-01 (1_704_067_200) and before year 2100.
        assert!(t > 1_704_067_200);
        assert!(t < 4_102_444_800);
    }
}
