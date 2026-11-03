//! One IMAP session reused across the operations of an `ImapClient`, and the
//! retry plan for opening one.
//!
//! Generic over the session type so the reuse and retry rules are testable
//! without a server; `imap.rs` plugs in the real session, `NOOP` and
//! `connect_sync`.

use std::sync::{Mutex, PoisonError};
use std::time::Duration;

/// Waits between connect attempts: three attempts in all.
pub(crate) const CONNECT_RETRY_DELAYS: [Duration; 2] = [Duration::from_secs(1), Duration::from_secs(2)];

/// Where opening a session failed.
#[derive(Debug)]
pub(crate) enum ConnectStage {
    /// TCP connect or TLS handshake.
    Stream,
    /// LOGIN, on an established stream.
    Login,
}

/// Whether a failed connect is worth another attempt.
///
/// Anything that stops the stream from opening (TCP connect, TLS handshake)
/// is retried. At LOGIN, only a dropped connection or the RFC 5530
/// `[UNAVAILABLE]` code (the server throttling logins) is: any other NO or
/// BAD is the server rejecting the credentials, and asking again only costs
/// the user a lockout.
pub(crate) fn is_transient_connect_failure(stage: &ConnectStage, error: &imap::Error) -> bool {
    match (stage, error) {
        (ConnectStage::Stream, _) => true,
        (ConnectStage::Login, imap::Error::Io(_) | imap::Error::ConnectionLost) => true,
        (ConnectStage::Login, imap::Error::No(message)) => message.contains("[UNAVAILABLE]"),
        (ConnectStage::Login, _) => false,
    }
}

/// Run `connect` until it succeeds, a failure is not transient, or the
/// attempts in [`CONNECT_RETRY_DELAYS`] run out.
pub(crate) fn connect_with_retry<S>(
    mut connect: impl FnMut() -> Result<S, (ConnectStage, imap::Error)>,
    mut sleep: impl FnMut(Duration),
) -> Result<S, imap::Error> {
    let mut delays = CONNECT_RETRY_DELAYS.iter();
    loop {
        match connect() {
            Ok(session) => return Ok(session),
            Err((stage, error)) => match delays.next() {
                Some(delay) if is_transient_connect_failure(&stage, &error) => sleep(*delay),
                _ => return Err(error),
            },
        }
    }
}

/// Idle sessions a [`SessionSlot`] keeps. Two, because a sync downloads the
/// next chunk while the current one is processed, and processing can need the
/// server too (attachments, moves); one more would only hold logins open.
const MAX_IDLE_SESSIONS: usize = 2;

/// The sessions an `ImapClient` keeps between operations.
pub(crate) struct SessionSlot<S> {
    sessions: Mutex<Vec<S>>,
}

impl<S> SessionSlot<S> {
    pub(crate) fn new() -> Self {
        Self {
            sessions: Mutex::new(Vec::new()),
        }
    }

    /// Take a kept session that `alive` confirms still works, otherwise open a
    /// new one with `connect`. A kept session that fails the check is closed.
    pub(crate) fn checkout<E>(
        &self,
        mut alive: impl FnMut(&mut S) -> bool,
        connect: impl FnOnce() -> Result<S, E>,
    ) -> Result<S, E> {
        loop {
            let kept = self.sessions.lock().unwrap_or_else(PoisonError::into_inner).pop();
            let Some(mut session) = kept else {
                return connect();
            };
            if alive(&mut session) {
                return Ok(session);
            }
        }
    }

    /// Keep `session` for the next operation, unless [`MAX_IDLE_SESSIONS`]
    /// are kept already; then it is closed.
    ///
    /// An operation that fails never checks its session back in, so a session
    /// in an unknown state is dropped rather than reused.
    pub(crate) fn checkin(&self, session: S) {
        let mut sessions = self.sessions.lock().unwrap_or_else(PoisonError::into_inner);
        if sessions.len() < MAX_IDLE_SESSIONS {
            sessions.push(session);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    /// A fake session: its id says which connect opened it.
    #[derive(Debug, PartialEq)]
    struct Fake(u32);

    fn counting_connect(count: &Cell<u32>) -> impl FnOnce() -> Result<Fake, ()> + '_ {
        move || {
            count.set(count.get() + 1);
            Ok(Fake(count.get()))
        }
    }

    #[test]
    fn operations_that_succeed_share_one_connection() {
        let slot = SessionSlot::new();
        let connects = Cell::new(0);
        for _ in 0..3 {
            let session = slot.checkout(|_| true, counting_connect(&connects)).unwrap();
            slot.checkin(session);
        }
        assert_eq!(connects.get(), 1);
    }

    #[test]
    fn two_overlapping_operations_keep_their_two_connections() {
        // A sync downloads the next chunk while the current one is processed,
        // so two operations can hold a session at once. Both are kept, or
        // every chunk would log in again.
        let slot = SessionSlot::new();
        let connects = Cell::new(0);
        for _ in 0..3 {
            let download = slot.checkout(|_| true, counting_connect(&connects)).unwrap();
            let processing = slot.checkout(|_| true, counting_connect(&connects)).unwrap();
            slot.checkin(processing);
            slot.checkin(download);
        }
        assert_eq!(connects.get(), 2);
    }

    #[test]
    fn at_most_two_idle_sessions_are_kept() {
        let slot = SessionSlot::new();
        let connects = Cell::new(0);
        let sessions: Vec<Fake> = (0..3)
            .map(|_| slot.checkout(|_| true, counting_connect(&connects)).unwrap())
            .collect();
        for session in sessions {
            slot.checkin(session);
        }
        for _ in 0..3 {
            let _held = slot.checkout(|_| true, counting_connect(&connects)).unwrap();
        }
        assert_eq!(connects.get(), 4, "the third idle session is closed, not kept");
    }

    #[test]
    fn a_session_not_checked_in_is_replaced_by_a_new_connection() {
        let slot = SessionSlot::new();
        let connects = Cell::new(0);
        let _failed_operation = slot.checkout(|_| true, counting_connect(&connects)).unwrap();
        let next = slot.checkout(|_| true, counting_connect(&connects)).unwrap();
        assert_eq!(next, Fake(2));
    }

    #[test]
    fn a_session_that_fails_the_liveness_check_is_replaced() {
        let slot = SessionSlot::new();
        let connects = Cell::new(0);
        let first = slot.checkout(|_| true, counting_connect(&connects)).unwrap();
        slot.checkin(first);
        let next = slot.checkout(|_| false, counting_connect(&connects)).unwrap();
        assert_eq!(next, Fake(2));
    }

    #[test]
    fn the_liveness_check_is_skipped_when_nothing_is_kept() {
        let slot = SessionSlot::new();
        let connects = Cell::new(0);
        let checked = Cell::new(false);
        slot.checkout(
            |_| {
                checked.set(true);
                true
            },
            counting_connect(&connects),
        )
        .unwrap();
        assert!(!checked.get());
    }

    fn timeout() -> imap::Error {
        imap::Error::Io(std::io::Error::from(std::io::ErrorKind::TimedOut))
    }

    /// Fails with each error in turn, then succeeds.
    fn run_retry(mut failures: Vec<(ConnectStage, imap::Error)>) -> (Result<u32, imap::Error>, u32, Vec<Duration>) {
        failures.reverse();
        let attempts = Cell::new(0);
        let mut slept = Vec::new();
        let result = connect_with_retry(
            || {
                attempts.set(attempts.get() + 1);
                match failures.pop() {
                    Some(failure) => Err(failure),
                    None => Ok(attempts.get()),
                }
            },
            |delay| slept.push(delay),
        );
        (result, attempts.get(), slept)
    }

    #[test]
    fn a_transient_failure_is_retried_after_the_planned_waits() {
        let (result, attempts, slept) = run_retry(vec![
            (ConnectStage::Stream, timeout()),
            (ConnectStage::Stream, timeout()),
        ]);
        assert_eq!(result.unwrap(), 3);
        assert_eq!(attempts, 3);
        assert_eq!(slept, CONNECT_RETRY_DELAYS.to_vec());
    }

    #[test]
    fn retries_stop_after_three_attempts_with_the_last_error() {
        let failures = (0..5).map(|_| (ConnectStage::Stream, timeout())).collect();
        let (result, attempts, slept) = run_retry(failures);
        assert!(matches!(result, Err(imap::Error::Io(_))));
        assert_eq!(attempts, 3);
        assert_eq!(slept.len(), 2);
    }

    #[test]
    fn a_rejected_login_is_not_retried() {
        let rejected = imap::Error::No("[AUTHENTICATIONFAILED] Invalid credentials".to_string());
        let (result, attempts, slept) = run_retry(vec![(ConnectStage::Login, rejected)]);
        assert!(matches!(result, Err(imap::Error::No(_))));
        assert_eq!(attempts, 1);
        assert!(slept.is_empty());
    }

    #[test]
    fn transient_and_permanent_failures_are_told_apart() {
        let cases: Vec<(ConnectStage, imap::Error, bool)> = vec![
            (ConnectStage::Stream, timeout(), true),
            (
                ConnectStage::Stream,
                imap::Error::Bad("TLS handshake failed: timed out".to_string()),
                true,
            ),
            (ConnectStage::Login, timeout(), true),
            (ConnectStage::Login, imap::Error::ConnectionLost, true),
            (
                ConnectStage::Login,
                imap::Error::No("[UNAVAILABLE] Try again later".to_string()),
                true,
            ),
            (
                ConnectStage::Login,
                imap::Error::No("[AUTHENTICATIONFAILED] Invalid credentials".to_string()),
                false,
            ),
            (ConnectStage::Login, imap::Error::No("LOGIN failed".to_string()), false),
            (
                ConnectStage::Login,
                imap::Error::Bad("Command unknown".to_string()),
                false,
            ),
        ];
        for (stage, error, expected) in cases {
            assert_eq!(
                is_transient_connect_failure(&stage, &error),
                expected,
                "{stage:?} {error:?}"
            );
        }
    }
}
