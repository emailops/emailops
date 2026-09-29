//! One constructor for the provider HTTP clients (Gmail, Graph, both
//! calendars, OAuth token endpoints). `reqwest::Client::new()` has no timeout
//! at all, so a half-open connection stalled a sync or a send forever.

use std::time::Duration;

/// How long to wait for the TCP/TLS connection itself.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Whole-request budget for API calls that move small JSON payloads
/// (calendar, OAuth token exchange/refresh/revoke).
pub(crate) const API_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);

/// Whole-request budget for the mail clients. They share one client for
/// sends with attachments and attachment downloads, which can legitimately
/// take minutes on a slow link.
pub(crate) const MAIL_REQUEST_TIMEOUT: Duration = Duration::from_secs(600);

/// A `reqwest::Client` with a connect timeout and a whole-request timeout.
pub(crate) fn provider_http_client(request_timeout: Duration) -> reqwest::Client {
    match reqwest::Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(request_timeout)
        .build()
    {
        Ok(client) => client,
        Err(e) => {
            // Only fails when the TLS backend cannot initialise, which the
            // default client would hit too; keep going but leave a trace.
            crate::services::logger::log(
                "error",
                "system",
                format!("HTTP client with timeouts could not be built, using defaults: {e}"),
            );
            reqwest::Client::new()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// A server that accepts the connection and then never answers.
    fn silent_server() -> (String, std::net::TcpListener) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/", listener.local_addr().unwrap());
        (url, listener)
    }

    #[tokio::test]
    async fn a_request_to_a_server_that_never_answers_times_out() {
        let (url, _listener) = silent_server();
        let client = provider_http_client(Duration::from_millis(300));

        let started = Instant::now();
        let err = client.get(&url).send().await.expect_err("must not hang");

        assert!(err.is_timeout(), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}
