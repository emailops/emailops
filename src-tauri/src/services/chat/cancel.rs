//! Cancelling a chat turn while it runs.
//!
//! Each running turn registers a flag under its assistant message id. The
//! chat's Cancel button raises it (`request_cancel`); the turn checks it
//! between tool rounds and from the token callback, which stops generation
//! mid-reply. A cancelled turn makes no further model call and keeps what the
//! user already saw, followed by a note.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

fn flags() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static FLAGS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    FLAGS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// A turn's cancel flag, registered while the guard lives: dropping it
/// unregisters the turn, so a finished turn can no longer be cancelled.
pub(crate) struct TurnGuard {
    message_id: String,
    pub flag: Arc<AtomicBool>,
}

impl TurnGuard {
    pub(crate) fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }
}

impl Drop for TurnGuard {
    fn drop(&mut self) {
        flags()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.message_id);
    }
}

/// Register the turn answering `message_id`.
pub(crate) fn register_turn(message_id: &str) -> TurnGuard {
    let flag = Arc::new(AtomicBool::new(false));
    flags()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(message_id.to_string(), Arc::clone(&flag));
    TurnGuard {
        message_id: message_id.to_string(),
        flag,
    }
}

/// Cancel the turn answering `message_id` — a research run included, which
/// stops at its next batch. `false` when no such turn is running.
pub fn request_cancel(message_id: &str) -> bool {
    let turn = match flags().lock().unwrap_or_else(PoisonError::into_inner).get(message_id) {
        Some(flag) => {
            flag.store(true, Ordering::Relaxed);
            true
        }
        None => false,
    };
    let research = super::research::request_stop(message_id);
    turn || research
}

/// The answer a cancelled turn keeps: what the user already saw (`partial`,
/// possibly empty), then a note in the reply language. Pure.
pub(crate) fn cancelled_answer(partial: &str, language_code: &str) -> String {
    let note = match language_code {
        "es" => "_Cancelado por el usuario._",
        "fr" => "_Annulé par l'utilisateur._",
        "de" => "_Vom Benutzer abgebrochen._",
        _ => "_Cancelled by the user._",
    };
    let partial = partial.trim_end();
    if partial.is_empty() {
        note.to_string()
    } else {
        format!("{partial}\n\n{note}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_reaches_a_running_turn_only() {
        let guard = register_turn("msg-cancel-1");
        assert!(request_cancel("msg-cancel-1"));
        assert!(guard.is_cancelled());
        drop(guard);
        assert!(!request_cancel("msg-cancel-1"), "a finished turn cannot be cancelled");
    }

    #[test]
    fn a_cancelled_answer_keeps_what_was_shown_then_says_so() {
        assert_eq!(
            cancelled_answer("You have three invoices", "en"),
            "You have three invoices\n\n_Cancelled by the user._"
        );
        assert_eq!(cancelled_answer("  ", "es"), "_Cancelado por el usuario._");
    }
}
