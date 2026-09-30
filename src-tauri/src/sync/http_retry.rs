//! Shared retry classification for the calendar HTTP clients, plus the
//! [`RetryPolicy`] every provider client applies to non-idempotent requests.
//!
//! Both `gmail_calendar` and `outlook_calendar` run the same retry loop:
//! transparent 401 refresh (once), exponential backoff on 429/5xx and transport
//! errors, give up after N attempts. They used to inline that decision as a
//! chain of match guards, which is where this bug lived:
//!
//! ```text
//! Ok(resp) if (429 || 5xx) && attempt < MAX_RETRIES => backoff,
//! Ok(resp) => return Ok(resp),      // <-- final attempt lands HERE
//! ```
//!
//! On the last attempt the guard is false, so a rate-limited or 500 response
//! fell through to the success arm and was handed to the caller as a valid API
//! result. The caller then parsed an error body as a calendar payload and saw
//! "no events" instead of an error. A second 401 (refresh already spent) took
//! the same path.
//!
//! Extracting the decision as a pure function makes every one of those cases
//! table-testable without an HTTP server, per the repo's planner/executor split.

/// What the retry loop should do with the outcome of one attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RetryDecision {
    /// Success — hand the response to the caller.
    Return,
    /// 401 and the refresh has not been spent yet: refresh the token, retry.
    RefreshAndRetry,
    /// Transient failure with attempts left: sleep, then retry.
    Backoff,
    /// Out of attempts, or a non-retryable failure. The caller must turn this
    /// into an error — never into a success.
    GiveUp,
}

/// Outcome of a single HTTP attempt, as far as the retry policy cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Attempt {
    /// The request completed and the server returned this status code.
    Status(u16),
    /// The request failed after it may have reached the server (read/write
    /// error, timeout).
    TransportError,
    /// The connection was never established, so the server never saw the
    /// request.
    ConnectError,
}

/// Whether a request may be re-sent after a failure that happened once it
/// could already have reached the server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RetryPolicy {
    /// Reads, updates by id, deletes: repeating them is harmless.
    Idempotent,
    /// Sends, creates, invitations, RSVPs: a 5xx or a dropped connection does
    /// not prove the server did nothing, and repeating one sends a duplicate.
    /// Retried only when the server provably did not act — 429, or a connect
    /// that never happened.
    NoRetryAfterSend,
}

impl RetryPolicy {
    /// May a transient failure status be retried under this policy?
    pub(crate) fn may_retry_status(self, status: u16) -> bool {
        match self {
            Self::Idempotent => true,
            Self::NoRetryAfterSend => status == 429,
        }
    }

    /// May a transport failure be retried? `connect_failed` means the request
    /// never left the client.
    pub(crate) fn may_retry_transport(self, connect_failed: bool) -> bool {
        match self {
            Self::Idempotent => true,
            Self::NoRetryAfterSend => connect_failed,
        }
    }
}

/// Decide what to do after one attempt.
///
/// `attempt` is 0-based and `max_retries` is the highest value it will take, so
/// `attempt == max_retries` is the final try.
pub(crate) fn classify_attempt(
    outcome: Attempt,
    attempt: u32,
    max_retries: u32,
    refresh_already_spent: bool,
    policy: RetryPolicy,
) -> RetryDecision {
    let attempts_remain = attempt < max_retries;

    match outcome {
        Attempt::Status(401) if !refresh_already_spent => RetryDecision::RefreshAndRetry,
        // A second 401 is a real auth failure, not something a retry fixes.
        Attempt::Status(401) => RetryDecision::GiveUp,
        Attempt::Status(status) if is_transient_status(status) => {
            if attempts_remain && policy.may_retry_status(status) {
                RetryDecision::Backoff
            } else {
                RetryDecision::GiveUp
            }
        }
        Attempt::Status(_) => RetryDecision::Return,
        Attempt::TransportError | Attempt::ConnectError => {
            if attempts_remain && policy.may_retry_transport(outcome == Attempt::ConnectError) {
                RetryDecision::Backoff
            } else {
                RetryDecision::GiveUp
            }
        }
    }
}

/// Statuses worth retrying: explicit throttling plus any server-side error.
fn is_transient_status(status: u16) -> bool {
    status == 429 || (500..600).contains(&status)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX: u32 = 3;

    /// The bug this module exists for: a throttled response on the FINAL
    /// attempt must not be reported as a success.
    #[test]
    fn throttled_response_on_the_last_attempt_gives_up_instead_of_returning() {
        assert_eq!(
            classify_attempt(Attempt::Status(429), MAX, MAX, false, RetryPolicy::Idempotent),
            RetryDecision::GiveUp
        );
    }

    /// Same shape for server errors — a 500 body is not a calendar payload.
    #[test]
    fn server_error_on_the_last_attempt_gives_up_instead_of_returning() {
        for status in [500, 502, 503, 599] {
            assert_eq!(
                classify_attempt(Attempt::Status(status), MAX, MAX, false, RetryPolicy::Idempotent),
                RetryDecision::GiveUp,
                "status {status} must not be returned as success"
            );
        }
    }

    /// The other half of the bug: once the single refresh is spent, another 401
    /// used to fall through to the success arm.
    #[test]
    fn second_unauthorized_gives_up_instead_of_returning() {
        assert_eq!(
            classify_attempt(Attempt::Status(401), 0, MAX, true, RetryPolicy::Idempotent),
            RetryDecision::GiveUp
        );
    }

    #[test]
    fn first_unauthorized_refreshes() {
        assert_eq!(
            classify_attempt(Attempt::Status(401), 0, MAX, false, RetryPolicy::Idempotent),
            RetryDecision::RefreshAndRetry
        );
    }

    #[test]
    fn transient_failures_back_off_while_attempts_remain() {
        for outcome in [Attempt::Status(429), Attempt::Status(503), Attempt::TransportError] {
            assert_eq!(
                classify_attempt(outcome, 0, MAX, false, RetryPolicy::Idempotent),
                RetryDecision::Backoff,
                "{outcome:?} should back off on the first attempt"
            );
        }
    }

    #[test]
    fn transport_error_on_the_last_attempt_gives_up() {
        assert_eq!(
            classify_attempt(Attempt::TransportError, MAX, MAX, false, RetryPolicy::Idempotent),
            RetryDecision::GiveUp
        );
    }

    #[test]
    fn success_statuses_are_returned() {
        for status in [200, 201, 204, 304] {
            assert_eq!(
                classify_attempt(Attempt::Status(status), 0, MAX, false, RetryPolicy::Idempotent),
                RetryDecision::Return,
                "status {status} should be returned to the caller"
            );
        }
    }

    /// A 4xx that is not 401/429 is the caller's problem to interpret (a 404 on
    /// a deleted event is meaningful), so it is returned rather than retried.
    #[test]
    fn non_auth_client_errors_are_returned_without_retrying() {
        for status in [400, 403, 404, 409] {
            assert_eq!(
                classify_attempt(Attempt::Status(status), 0, MAX, false, RetryPolicy::Idempotent),
                RetryDecision::Return,
                "status {status} should reach the caller unretried"
            );
        }
    }

    /// A send/create that got a 5xx may well have been carried out; retrying
    /// it sent the same mail (or invite) twice.
    #[test]
    fn a_non_idempotent_request_is_not_retried_after_a_server_error() {
        assert_eq!(
            classify_attempt(Attempt::Status(503), 0, MAX, false, RetryPolicy::NoRetryAfterSend),
            RetryDecision::GiveUp
        );
    }

    #[test]
    fn a_non_idempotent_request_is_not_retried_after_a_mid_request_transport_error() {
        assert_eq!(
            classify_attempt(Attempt::TransportError, 0, MAX, false, RetryPolicy::NoRetryAfterSend),
            RetryDecision::GiveUp
        );
    }

    /// 429 and a failed connect both mean the server never acted on it.
    #[test]
    fn a_non_idempotent_request_is_retried_when_it_was_never_processed() {
        for outcome in [Attempt::Status(429), Attempt::ConnectError] {
            assert_eq!(
                classify_attempt(outcome, 0, MAX, false, RetryPolicy::NoRetryAfterSend),
                RetryDecision::Backoff,
                "{outcome:?}"
            );
        }
    }

    #[test]
    fn a_connect_error_backs_off_for_idempotent_requests_too() {
        assert_eq!(
            classify_attempt(Attempt::ConnectError, 0, MAX, false, RetryPolicy::Idempotent),
            RetryDecision::Backoff
        );
    }

    #[test]
    fn transient_status_classification() {
        assert!(is_transient_status(429));
        assert!(is_transient_status(500));
        assert!(is_transient_status(599));
        assert!(!is_transient_status(499));
        assert!(!is_transient_status(600));
        assert!(!is_transient_status(200));
    }
}
