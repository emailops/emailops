//! Process-wide state a research run shares with the commands around it: the
//! stop flag the chat's "Stop" button raises, and the estimate the user
//! confirmed, so the run reads exactly the emails the estimate counted instead
//! of planning the question a second time.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use super::Prepared;

/// How long a confirmed-but-unsent estimate stays usable. Past it the run
/// plans again — the mailbox may have changed under an old count.
const ESTIMATE_TTL: Duration = Duration::from_secs(30 * 60);

fn stop_flags() -> &'static Mutex<HashMap<String, Arc<AtomicBool>>> {
    static FLAGS: OnceLock<Mutex<HashMap<String, Arc<AtomicBool>>>> = OnceLock::new();
    FLAGS.get_or_init(|| Mutex::new(HashMap::new()))
}

struct CachedEstimate {
    account_id: String,
    question: String,
    prepared: Prepared,
    created: Instant,
}

fn estimates() -> &'static Mutex<HashMap<String, CachedEstimate>> {
    static ESTIMATES: OnceLock<Mutex<HashMap<String, CachedEstimate>>> = OnceLock::new();
    ESTIMATES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Registers the run for `message_id` and returns its stop flag. Dropping the
/// guard unregisters it, so a finished run can no longer be "stopped".
pub(crate) struct RunGuard {
    message_id: String,
    pub flag: Arc<AtomicBool>,
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        stop_flags()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.message_id);
    }
}

pub(crate) fn register_run(message_id: &str) -> RunGuard {
    let flag = Arc::new(AtomicBool::new(false));
    stop_flags()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(message_id.to_string(), Arc::clone(&flag));
    RunGuard {
        message_id: message_id.to_string(),
        flag,
    }
}

/// Ask the run answering `message_id` to stop reading and write its report
/// from what it has read. `false` when no such run is live.
pub fn request_stop(message_id: &str) -> bool {
    match stop_flags()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(message_id)
    {
        Some(flag) => {
            flag.store(true, Ordering::Relaxed);
            true
        }
        None => false,
    }
}

/// Keep a prepared run for the user to confirm; returns its id.
pub(crate) fn store_estimate(account_id: &str, question: &str, prepared: Prepared) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let mut map = estimates().lock().unwrap_or_else(PoisonError::into_inner);
    map.retain(|_, e| e.created.elapsed() < ESTIMATE_TTL);
    map.insert(
        id.clone(),
        CachedEstimate {
            account_id: account_id.to_string(),
            question: question.to_string(),
            prepared,
            created: Instant::now(),
        },
    );
    id
}

/// The prepared run behind a confirmed estimate — only for the same account
/// and question, and only once.
pub(crate) fn take_estimate(id: &str, account_id: &str, question: &str) -> Option<Prepared> {
    let mut map = estimates().lock().unwrap_or_else(PoisonError::into_inner);
    let cached = map.remove(id)?;
    let usable = cached.account_id == account_id
        && cached.question.trim() == question.trim()
        && cached.created.elapsed() < ESTIMATE_TTL;
    usable.then_some(cached.prepared)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prepared(ids: &[&str]) -> Prepared {
        Prepared {
            email_ids: ids.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn stop_reaches_a_live_run_only() {
        let guard = register_run("msg-stop-1");
        assert!(request_stop("msg-stop-1"));
        assert!(guard.flag.load(Ordering::Relaxed));
        drop(guard);
        assert!(!request_stop("msg-stop-1"), "a finished run cannot be stopped");
    }

    #[test]
    fn an_estimate_is_taken_once_for_the_same_question() {
        let id = store_estimate("acct", "¿qué facturas?", prepared(&["e1"]));
        assert!(take_estimate(&id, "acct", "¿qué facturas? ").is_some());
        assert!(take_estimate(&id, "acct", "¿qué facturas?").is_none(), "used up");
    }

    #[test]
    fn an_estimate_does_not_serve_another_account_or_question() {
        let id = store_estimate("acct", "q1", prepared(&["e1"]));
        assert!(take_estimate(&id, "other", "q1").is_none());
        let id = store_estimate("acct", "q1", prepared(&["e1"]));
        assert!(take_estimate(&id, "acct", "q2").is_none());
    }
}
