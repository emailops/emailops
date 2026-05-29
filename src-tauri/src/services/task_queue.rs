use serde::Serialize;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, Semaphore};

/// How many recently-completed tasks each queue retains for the dashboard.
const HISTORY_LIMIT: usize = 5;

type BoxFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

struct QueuedTask {
    id: u64,
    name: String,
    fut: BoxFuture,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskInfo {
    pub id: u64,
    pub name: String,
    /// Unix timestamp (seconds) when the task started running. For pending
    /// tasks this is when it was submitted to the queue.
    pub started_at: i64,
}

/// One entry in a queue's recent-completions ring buffer. Surfaces in the
/// dashboard as the "Past 5" list per queue so the user can see what just
/// finished and whether it succeeded.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskHistoryEntry {
    pub id: u64,
    pub name: String,
    /// Unix seconds when the task started executing.
    pub started_at: i64,
    /// Unix seconds when the task finished.
    pub finished_at: i64,
    /// Wall-clock seconds the task spent running. Computed from
    /// `finished_at - started_at` and clamped to 0 to avoid negative values
    /// from clock skew.
    pub duration_secs: i64,
    /// "ok" if the future ran to completion, "ko" if it panicked. Today the
    /// queue can't observe inner Result-typed failures because tasks return
    /// `()`; "ok" therefore only means "no panic".
    pub status: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueStateSnapshot {
    pub name: String,
    pub concurrency: usize,
    pub running: Vec<TaskInfo>,
    pub pending: Vec<TaskInfo>,
    /// Most-recent first, capped at `HISTORY_LIMIT` entries.
    pub history: Vec<TaskHistoryEntry>,
}

/// Shared inspection state for a queue. Updated by the consumer as tasks
/// move from "pending" → "running" → "completed".
#[derive(Default)]
struct QueueState {
    running: Vec<TaskInfo>,
    pending: Vec<TaskInfo>,
    history: VecDeque<TaskHistoryEntry>,
}

/// An unbounded task queue that limits concurrent background operations via a semaphore.
///
/// - Tasks are never dropped due to queue capacity (unbounded channel).
/// - The only failure mode is a dropped receiver (app shutting down), which is logged.
/// - Lazily starts the consumer on first submit (avoids needing a Tokio runtime at construction).
/// - Tracks a name + start time for every running and pending task so the
///   dashboard can show what's happening in real time.
#[derive(Clone)]
pub struct TaskQueue {
    sender: mpsc::UnboundedSender<QueuedTask>,
    started: Arc<Mutex<bool>>,
    concurrency: usize,
    receiver: Arc<Mutex<Option<mpsc::UnboundedReceiver<QueuedTask>>>>,
    name: &'static str,
    state: Arc<std::sync::Mutex<QueueState>>,
    next_id: Arc<AtomicU64>,
}

impl TaskQueue {
    /// Create a new task queue. The background consumer starts on first `submit()`.
    ///
    /// - `concurrency`: max number of tasks running simultaneously.
    /// - `name`: used in log messages to identify the queue (e.g. "ai", "db").
    pub fn new(concurrency: usize, name: &'static str) -> Self {
        let (sender, receiver) = mpsc::unbounded_channel::<QueuedTask>();
        Self {
            sender,
            started: Arc::new(Mutex::new(false)),
            concurrency,
            receiver: Arc::new(Mutex::new(Some(receiver))),
            name,
            state: Arc::new(std::sync::Mutex::new(QueueState::default())),
            next_id: Arc::new(AtomicU64::new(1)),
        }
    }

    /// Submit an unnamed task. Prefer `submit_named` so the dashboard shows
    /// something meaningful instead of "unnamed".
    pub async fn submit<F>(&self, task: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.submit_named("unnamed", task).await
    }

    /// Submit a task with a human-readable name. The name appears in the
    /// background-task panel of the dashboard while the task is queued and
    /// while it's running.
    pub async fn submit_named<F>(&self, name: &str, task: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        // Lazy-start the consumer
        {
            let mut started = self.started.lock().await;
            if !*started {
                *started = true;
                let mut guard = self.receiver.lock().await;
                if let Some(receiver) = guard.take() {
                    let concurrency = self.concurrency;
                    let state = self.state.clone();
                    let queue_name = self.name;
                    tokio::spawn(run_consumer(receiver, concurrency, state, queue_name));
                }
            }
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let info = TaskInfo {
            id,
            name: name.to_string(),
            started_at: chrono::Utc::now().timestamp(),
        };

        // Record as pending before sending so a dashboard read between
        // send() and consumer pickup still sees the task.
        if let Ok(mut s) = self.state.lock() {
            s.pending.push(info.clone());
        }

        let queued = QueuedTask {
            id,
            name: name.to_string(),
            fut: Box::pin(task),
        };

        if self.sender.send(queued).is_err() {
            // Receiver dropped — undo the pending entry we just added.
            if let Ok(mut s) = self.state.lock() {
                s.pending.retain(|t| t.id != id);
            }
            crate::services::logger::log(
                "error",
                "system",
                format!(
                    "[task_queue:{}] failed to enqueue task '{}': receiver dropped (app shutting down?)",
                    self.name, name
                ),
            );
        }
    }

    /// Snapshot of the queue's current running + pending tasks plus the most
    /// recent history. History is returned newest-first.
    pub fn snapshot(&self) -> QueueStateSnapshot {
        let (running, pending, history) = match self.state.lock() {
            Ok(s) => (
                s.running.clone(),
                s.pending.clone(),
                s.history.iter().rev().cloned().collect(),
            ),
            Err(poisoned) => {
                let s = poisoned.into_inner();
                (
                    s.running.clone(),
                    s.pending.clone(),
                    s.history.iter().rev().cloned().collect(),
                )
            }
        };
        QueueStateSnapshot {
            name: self.name.to_string(),
            concurrency: self.concurrency,
            running,
            pending,
            history,
        }
    }
}

async fn run_consumer(
    mut receiver: mpsc::UnboundedReceiver<QueuedTask>,
    concurrency: usize,
    state: Arc<std::sync::Mutex<QueueState>>,
    queue_name: &'static str,
) {
    let semaphore = Arc::new(Semaphore::new(concurrency));
    while let Some(queued) = receiver.recv().await {
        let permit = semaphore.clone().acquire_owned().await;
        let task_state = state.clone();
        let id = queued.id;
        let name = queued.name;
        let fut = queued.fut;
        tokio::spawn(async move {
            // Move from pending → running, refreshing started_at to the
            // moment execution actually begins.
            let started_at = chrono::Utc::now().timestamp();
            let info = TaskInfo {
                id,
                name: name.clone(),
                started_at,
            };
            if let Ok(mut s) = task_state.lock() {
                s.pending.retain(|t| t.id != id);
                s.running.push(info);
            }

            // Catch panics so a misbehaving task can't poison the state mutex
            // and leave the queue stuck. A panicking task is logged and the
            // state entry is cleaned up just like a normal completion.
            let result = std::panic::AssertUnwindSafe(fut);
            let outcome = futures::FutureExt::catch_unwind(result).await;
            let panicked = outcome.is_err();
            if panicked {
                eprintln!("[task_queue:{}] task '{}' (id={}) panicked", queue_name, name, id);
            }

            let finished_at = chrono::Utc::now().timestamp();
            let entry = TaskHistoryEntry {
                id,
                name: name.clone(),
                started_at,
                finished_at,
                duration_secs: (finished_at - started_at).max(0),
                status: if panicked { "ko".to_string() } else { "ok".to_string() },
            };
            if let Ok(mut s) = task_state.lock() {
                s.running.retain(|t| t.id != id);
                s.history.push_back(entry);
                while s.history.len() > HISTORY_LIMIT {
                    s.history.pop_front();
                }
            }
            drop(permit);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn snapshot_tracks_running_and_pending() {
        let q = TaskQueue::new(1, "test");

        let (gate_tx, gate_rx) = tokio::sync::oneshot::channel::<()>();
        let gate_rx = Arc::new(Mutex::new(Some(gate_rx)));

        // Task 1 blocks on the gate so it stays "running" while we inspect.
        let gate_clone = gate_rx.clone();
        q.submit_named("blocker", async move {
            let rx = gate_clone.lock().await.take().unwrap();
            let _ = rx.await;
        })
        .await;

        // Tasks 2..4 sit in "pending" behind it.
        for i in 0..3 {
            let label = format!("pending-{}", i);
            q.submit_named(&label, async {}).await;
        }

        // Give the consumer a tick to pick up task 1.
        tokio::time::sleep(Duration::from_millis(50)).await;

        let snap = q.snapshot();
        assert_eq!(snap.name, "test");
        assert_eq!(snap.concurrency, 1);
        assert_eq!(snap.running.len(), 1, "expected blocker to be running");
        assert_eq!(snap.running[0].name, "blocker");
        assert_eq!(snap.pending.len(), 3, "expected 3 tasks pending");

        // Release the gate; everything should drain.
        let _ = gate_tx.send(());
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let s = q.snapshot();
            if s.running.is_empty() && s.pending.is_empty() {
                return;
            }
        }
        panic!("queue did not drain: {:?}", q.snapshot());
    }

    #[tokio::test]
    async fn history_records_recent_completions_with_status() {
        let q = TaskQueue::new(1, "test_history");

        // Three OK tasks and one KO (panicking) task, in deterministic order.
        q.submit_named("ok-1", async {}).await;
        q.submit_named("boom", async {
            panic!("intentional");
        })
        .await;
        q.submit_named("ok-2", async {}).await;
        q.submit_named("ok-3", async {}).await;

        // Wait for the queue to drain.
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let s = q.snapshot();
            if s.running.is_empty() && s.pending.is_empty() && s.history.len() == 4 {
                break;
            }
        }

        let snap = q.snapshot();
        assert_eq!(
            snap.history.len(),
            4,
            "expected all four completions, got {:?}",
            snap.history
        );

        // Newest-first ordering: ok-3 should be the most recent entry.
        let names: Vec<&str> = snap.history.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, vec!["ok-3", "ok-2", "boom", "ok-1"]);

        // Status: only the panicking task should be "ko".
        let boom = snap.history.iter().find(|e| e.name == "boom").unwrap();
        assert_eq!(boom.status, "ko");
        for entry in snap.history.iter().filter(|e| e.name != "boom") {
            assert_eq!(entry.status, "ok", "{} should be ok", entry.name);
        }
    }

    #[tokio::test]
    async fn history_is_capped_to_limit() {
        let q = TaskQueue::new(2, "test_cap");
        // Submit double the limit; only the most recent HISTORY_LIMIT survive.
        for i in 0..(HISTORY_LIMIT * 2) {
            let label = format!("t{i}");
            q.submit_named(&label, async {}).await;
        }
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let s = q.snapshot();
            if s.running.is_empty() && s.pending.is_empty() && s.history.len() == HISTORY_LIMIT {
                return;
            }
        }
        panic!("history did not stabilise: {:?}", q.snapshot().history);
    }

    #[tokio::test]
    async fn panicking_task_does_not_block_queue() {
        let q = TaskQueue::new(1, "test_panic");
        q.submit_named("boom", async {
            panic!("intentional");
        })
        .await;

        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        q.submit_named("after", async move {
            let _ = tx.send(());
        })
        .await;

        tokio::time::timeout(Duration::from_secs(2), rx)
            .await
            .expect("queue stuck after panicking task")
            .expect("after-task channel closed");

        let snap = q.snapshot();
        assert!(snap.running.is_empty());
        assert!(snap.pending.is_empty());
    }
}
