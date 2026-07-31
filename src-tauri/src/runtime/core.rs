//! `AppCore` — the transport-agnostic application state.
//!
//! Everything `AppState` used to hold *except* the Tauri background scheduler: the
//! database handle, the data directory, the task queues, the dispatcher and the chat
//! tool registry. None of it knows about Tauri, so it can back a desktop window, a
//! CLI invocation, or one user's session inside a server process.
//!
//! `AppState` (desktop only) now wraps an `Arc<AppCore>` and `Deref`s to it, so the
//! ~200 command handlers that reach for `state.db` / `state.ai_background` /
//! `state.dispatcher` keep compiling untouched.
//!
//! ## Scope
//!
//! One `AppCore` == one user's mailbox universe. On the desktop there is exactly one,
//! built at startup. In a server there is one per signed-in user, opened lazily and
//! held in an LRU — which is why the data directory and the database are fields here
//! rather than process globals.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, PoisonError};

use crate::db::Database;
use crate::services;
use crate::services::task_queue::{QueueStateSnapshot, TaskQueue};

/// Transport-agnostic application state for a single user's mailbox.
pub struct AppCore {
    pub db: Arc<Database>,
    pub app_data_dir: PathBuf,
    /// Queue for interactive AI tasks (chat, draft generation). Concurrency 1
    /// so a user-facing request always gets full model throughput.
    pub ai_queue: TaskQueue,
    /// Queue for background AI tasks (classification, embeddings after sync).
    /// Concurrency 1 so background work doesn't compete with interactive requests.
    pub ai_background: TaskQueue,
    /// Queue for fast DB-only background tasks. Higher concurrency since there is no
    /// shared GPU/CPU bottleneck.
    pub db_queue: TaskQueue,
    /// Per-account sync queues. Each account gets its own concurrency-1
    /// `TaskQueue` lazily on first manual sync, so one slow provider can't
    /// stall manual syncs of other accounts. A single account is still
    /// serialized inside `sync_account` by `sync_locks` (try_lock); the
    /// per-account queue exists for dashboard visibility and so multiple
    /// rapid clicks on the same account land in a FIFO instead of bailing
    /// out via the lock.
    pub sync_queues: Arc<Mutex<HashMap<String, TaskQueue>>>,
    /// Per-account abort flags. Set to `true` before deleting an account so any
    /// in-progress sync exits cleanly at the next batch boundary.
    pub sync_abort_flags: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    /// Per-account sync mutex. Held for the duration of a sync so concurrent
    /// calls (UI trigger + scheduler tick) bail out immediately via try_lock.
    pub sync_locks: Arc<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    /// Background internet-connectivity probe. Exposes a cached boolean used
    /// by the `is_online` command and emits `app-connectivity-changed` events
    /// on state transitions.
    pub connectivity: services::connectivity::ConnectivityMonitor,
    /// Typed dispatcher for background tasks.
    ///
    /// Use `core.dispatcher.dispatch(BackgroundTask::…, || Box::pin(async { … }))`
    /// in handlers instead of calling `queue.submit_named` directly. This makes it
    /// possible to swap in a `FakeDispatcher` in tests and assert exactly which
    /// tasks were enqueued, without running the underlying futures.
    pub dispatcher: Arc<dyn services::background_tasks::TaskDispatcher>,
    /// Chat tool registry. Built once at startup with every production tool
    /// pre-registered (see `services::chat::tools::default_registry`).
    /// `run_chat_turn` consults it both for the LLM's tool-definitions array
    /// (filtered by Settings feature flags) and for dispatching tool calls.
    pub tool_registry: Arc<services::chat::tools::ToolRegistry>,
}

impl AppCore {
    /// Construct an `AppCore` suitable for unit/integration tests.
    ///
    /// * Uses the provided in-memory database (from `Database::new_for_testing()`).
    /// * Wires a stub connectivity monitor (no real I/O).
    /// * Task queues are created with minimal concurrency.
    /// * All hash-maps start empty.
    #[cfg(test)]
    pub fn for_testing(db: Arc<Database>) -> Self {
        Self {
            db,
            // `std::env::temp_dir()` rather than a literal "/tmp/..." — the
            // latter is not a valid path on Windows.
            app_data_dir: std::env::temp_dir().join("emailops-test"),
            ai_queue: TaskQueue::new(1, "ai"),
            ai_background: TaskQueue::new(1, "ai_bg"),
            db_queue: TaskQueue::new(1, "db"),
            sync_queues: Arc::new(Mutex::new(HashMap::new())),
            sync_abort_flags: Arc::new(Mutex::new(HashMap::new())),
            sync_locks: Arc::new(Mutex::new(HashMap::new())),
            connectivity: services::connectivity::ConnectivityMonitor::stub(),
            dispatcher: Arc::new(services::background_tasks::FakeDispatcher::new()),
            tool_registry: Arc::new(services::chat::tools::default_registry()),
        }
    }

    /// Get-or-create the dedicated sync queue for `account_id`. Each queue
    /// is concurrency 1 so the same account never has two downloads
    /// in-flight via the queue path; different accounts run on independent
    /// queues so a slow provider for one account cannot block another.
    pub fn sync_queue_for(&self, account_id: &str) -> TaskQueue {
        let mut map = self.sync_queues.lock().unwrap_or_else(PoisonError::into_inner);
        map.entry(account_id.to_string())
            .or_insert_with(|| TaskQueue::new(1, "sync"))
            .clone()
    }

    /// Drop every per-account map entry for `account_id`.
    ///
    /// `sync_queues`, `sync_abort_flags` and `sync_locks` are keyed by account and
    /// were previously never pruned, so deleting and re-adding accounts grew them
    /// without bound. Harmless-ish for a single desktop session; a genuine leak for
    /// a long-lived process serving many users.
    pub fn forget_account(&self, account_id: &str) {
        self.sync_queues
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(account_id);
        self.sync_abort_flags
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(account_id);
        self.sync_locks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(account_id);
    }

    /// Aggregate snapshot of every per-account sync queue, presented to the
    /// dashboard as a single "sync" queue. Running/pending tasks are merged
    /// as-is (their `name` already carries the account id via
    /// `submit_named`); history is merged across accounts and truncated to
    /// the most recent 5 globally so the panel stays compact. Effective
    /// concurrency is reported as the number of distinct per-account queues
    /// (each contributes one parallel slot).
    pub fn sync_queue_snapshot(&self) -> QueueStateSnapshot {
        let queues: Vec<TaskQueue> = {
            let map = self.sync_queues.lock().unwrap_or_else(PoisonError::into_inner);
            map.values().cloned().collect()
        };
        let mut running = Vec::new();
        let mut pending = Vec::new();
        let mut history = Vec::new();
        for q in &queues {
            let snap = q.snapshot();
            running.extend(snap.running);
            pending.extend(snap.pending);
            history.extend(snap.history);
        }
        history.sort_by_key(|h| std::cmp::Reverse(h.finished_at));
        history.truncate(5);
        QueueStateSnapshot {
            name: "sync".to_string(),
            concurrency: queues.len().max(1),
            running,
            pending,
            history,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core() -> AppCore {
        #[allow(clippy::expect_used)] // test-only: an in-memory DB cannot fail to open
        let db = Arc::new(Database::new_for_testing().expect("in-memory db"));
        AppCore::for_testing(db)
    }

    #[test]
    fn sync_queue_for_is_stable_across_calls() {
        let core = core();
        let _ = core.sync_queue_for("acct-a");
        let _ = core.sync_queue_for("acct-a");
        let map = core.sync_queues.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(map.len(), 1, "same account must reuse one queue");
    }

    #[test]
    fn distinct_accounts_get_distinct_queues() {
        let core = core();
        let _ = core.sync_queue_for("acct-a");
        let _ = core.sync_queue_for("acct-b");
        let map = core.sync_queues.lock().unwrap_or_else(PoisonError::into_inner);
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn forget_account_prunes_every_per_account_map() {
        let core = core();
        let _ = core.sync_queue_for("acct-a");
        let _ = core.sync_queue_for("acct-b");
        core.sync_abort_flags
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert("acct-a".to_string(), Arc::new(AtomicBool::new(false)));
        core.sync_locks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert("acct-a".to_string(), Arc::new(tokio::sync::Mutex::new(())));

        core.forget_account("acct-a");

        assert!(!core
            .sync_queues
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key("acct-a"));
        assert!(!core
            .sync_abort_flags
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key("acct-a"));
        assert!(!core
            .sync_locks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains_key("acct-a"));
        assert!(
            core.sync_queues
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .contains_key("acct-b"),
            "pruning one account must not disturb another"
        );
    }

    #[test]
    fn snapshot_reports_one_slot_per_account_queue() {
        let core = core();
        let _ = core.sync_queue_for("acct-a");
        let _ = core.sync_queue_for("acct-b");
        assert_eq!(core.sync_queue_snapshot().concurrency, 2);
    }

    #[test]
    fn snapshot_of_an_idle_core_reports_a_single_slot() {
        // `.max(1)` keeps the dashboard from rendering a zero-width queue.
        assert_eq!(core().sync_queue_snapshot().concurrency, 1);
    }
}
