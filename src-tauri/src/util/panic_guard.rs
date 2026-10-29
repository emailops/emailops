//! Keep a panic inside one unit of work from taking down what runs it.
//!
//! A sync runs inside long-lived loops (the Gmail poll, the IMAP watcher, the
//! calendar poll) and behind a Tauri command the UI awaits. A panic there used
//! to end the loop for good and leave the UI waiting forever, and one message
//! that panicked on parse stopped the whole account at every retry. These
//! helpers turn the panic into an ordinary error of that unit, so the caller's
//! error path (log, `sync_state`, skip the message) runs instead.
//!
//! The panic payload never reaches the error: it can quote message text.
//! Where it happened is logged by [`install_panic_hook`].

use std::future::Future;
use std::panic::AssertUnwindSafe;

use futures::FutureExt;

use crate::models::error::{AppError, Result};

fn crashed(what: &str) -> AppError {
    AppError::SyncError(format!("{what} hit an internal error and was skipped"))
}

/// Await `fut`; a panic inside it becomes an `Err` naming `what`.
pub async fn run_isolated<T>(what: &str, fut: impl Future<Output = Result<T>>) -> Result<T> {
    AssertUnwindSafe(fut)
        .catch_unwind()
        .await
        .unwrap_or_else(|_| Err(crashed(what)))
}

/// Call `f`; a panic inside it becomes an `Err` naming `what`.
pub fn call_isolated<T>(what: &str, f: impl FnOnce() -> Result<T>) -> Result<T> {
    std::panic::catch_unwind(AssertUnwindSafe(f)).unwrap_or_else(|_| Err(crashed(what)))
}

/// The line logged for a panic: where it happened, never its message.
fn panic_report(location: Option<&std::panic::Location<'_>>) -> String {
    match location {
        Some(l) => format!("Internal error (panic) at {}:{}:{}", l.file(), l.line(), l.column()),
        None => "Internal error (panic) at an unknown location".to_string(),
    }
}

/// Also send every panic's location to the Logs panel (stderr alone is
/// invisible in an installed app). Keeps the default hook's stderr output.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        crate::services::logger::log("error", "system", panic_report(info.location()));
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn run_isolated_passes_results_through() {
        assert_eq!(run_isolated("sync", async { Ok(7) }).await.unwrap(), 7);
        let err = run_isolated::<()>("sync", async { Err(AppError::NotFound("x".into())) }).await;
        assert!(matches!(err, Err(AppError::NotFound(_))));
    }

    #[tokio::test]
    async fn run_isolated_turns_a_panic_into_an_error_without_its_message() {
        let err = run_isolated::<()>("Sync of acct-1", async { panic!("subject: private text") })
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("Sync of acct-1"), "{err}");
        assert!(!err.contains("private text"), "{err}");
    }

    #[test]
    fn call_isolated_passes_results_through_and_catches_a_panic() {
        assert_eq!(call_isolated("parse", || Ok(1)).unwrap(), 1);
        let err = call_isolated::<()>("Reading message 42", || panic!("from: private text"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("Reading message 42"), "{err}");
        assert!(!err.contains("private text"), "{err}");
    }

    #[test]
    fn the_panic_report_names_the_place_only() {
        let here = std::panic::Location::caller();
        let line = panic_report(Some(here));
        assert!(line.contains(&format!("{}:{}", here.file(), here.line())), "{line}");
        assert!(panic_report(None).contains("unknown location"));
    }
}
