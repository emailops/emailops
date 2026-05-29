/// Typed catalogue of every background operation the app can enqueue.
///
/// Why this exists
/// ---------------
/// The raw `TaskQueue` accepts boxed futures — tests can observe only the
/// task *name* string, and an AI agent reading the codebase has to grep
/// across every `submit_named` call site to understand what background work
/// exists. This enum gives a single authoritative list so:
///
///   1. Tests can `assert_eq!(dispatcher.recorded(), vec![BackgroundTask::…])`.
///   2. New contributors (and AI agents) see the full menu at a glance.
///   3. Routing rules (ai_queue vs db_queue) live next to the enum, not
///      scattered across command handlers.
///
/// Migration status
/// ----------------
/// Call sites still use `submit_named` directly. Migrate them incrementally
/// by replacing `queue.submit_named("…", async move { … })` with
/// `dispatcher.dispatch(BackgroundTask::…)` as you touch each command.
/// The `TaskDispatcher` trait (below) makes this testable from day one.
///
/// Queue routing
/// -------------
/// See `BackgroundTask::queue` for the routing rules.
/// - `ai_queue`        — interactive AI (draft, chat). Concurrency 1.
/// - `ai_background`   — background AI (classify, embed, memory). Concurrency 1.
/// - `db_queue`        — DB-only work (lens row update, filter stats). Concurrency 4.
/// - `sync_queue`      — per-account sync. One queue per account, concurrency 1 each.
use std::future::Future;
use std::pin::Pin;
use std::sync::{PoisonError, RwLock};

use serde::{Deserialize, Serialize};

/// Every background operation the app can enqueue.
///
/// Each variant carries the minimal set of IDs needed to reconstruct the
/// work on the executor side. Do not embed large payloads — pass IDs and
/// let the executor re-fetch from the DB.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BackgroundTask {
    // ── Interactive AI ──────────────────────────────────────────────────
    /// Generate an AI draft reply for an incoming email.
    GenerateDraft { email_id: String, request_id: String },
    /// Run one turn of a chat conversation.
    SendChatMessage { conversation_id: i64, request_id: String },

    // ── Background AI ───────────────────────────────────────────────────
    /// Classify (triage / tag) a batch of emails for one account.
    ClassifyEmails { account_id: String, request_id: String },
    /// Re-classify all previously classified emails for one account.
    ReclassifyAllEmails { account_id: String, request_id: String },
    /// Generate vector embeddings for emails that don't have them yet.
    GenerateEmbeddings {
        account_id: Option<String>,
        request_id: String,
    },
    /// Regenerate all embeddings (e.g. after a model change).
    RegenerateEmbeddings { request_id: String },
    /// Run a Lens extraction pass.
    RunLens { lens_id: i64, run_id: i64 },
    /// Re-extract a single Lens row.
    ReextractLensRow { lens_id: i64, email_id: String },
    /// Extract memory facts from a batch of emails.
    BackfillMemory { account_id: String, request_id: String },
    /// Extract task items from a batch of emails.
    BackfillTasks { account_id: String, request_id: String },
    /// Download a local AI model file.
    DownloadModel { model_id: String },

    // ── Sync ────────────────────────────────────────────────────────────
    /// Sync a single email account now (user-triggered or scheduler tick).
    SyncAccount { account_id: String, request_id: String },
    /// Re-download emails whose body is missing or empty.
    RedownloadEmptyEmails { account_id: String, request_id: String },
}

/// Which queue a task should be routed to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueKind {
    /// Interactive AI requests. Concurrency 1.
    AiInteractive,
    /// Background AI work. Concurrency 1.
    AiBackground,
    /// Fast DB-only work. Concurrency 4.
    Db,
    /// Per-account sync. One queue per account.
    Sync,
}

impl BackgroundTask {
    /// Returns which queue this task should be submitted to.
    pub fn queue(&self) -> QueueKind {
        match self {
            Self::GenerateDraft { .. } | Self::SendChatMessage { .. } => QueueKind::AiInteractive,
            Self::ClassifyEmails { .. }
            | Self::ReclassifyAllEmails { .. }
            | Self::GenerateEmbeddings { .. }
            | Self::RegenerateEmbeddings { .. }
            | Self::RunLens { .. }
            | Self::ReextractLensRow { .. }
            | Self::BackfillMemory { .. }
            | Self::BackfillTasks { .. }
            | Self::DownloadModel { .. } => QueueKind::AiBackground,
            Self::SyncAccount { .. } | Self::RedownloadEmptyEmails { .. } => QueueKind::Sync,
        }
    }

    /// Human-readable label for the task panel dashboard.
    pub fn label(&self) -> String {
        match self {
            Self::GenerateDraft { email_id, .. } => format!("draft:{email_id}"),
            Self::SendChatMessage { conversation_id, .. } => format!("chat:{conversation_id}"),
            Self::ClassifyEmails { account_id, .. } => format!("classify:{account_id}"),
            Self::ReclassifyAllEmails { account_id, .. } => format!("reclassify-all:{account_id}"),
            Self::GenerateEmbeddings { account_id, .. } => {
                format!("embeddings:{}", account_id.as_deref().unwrap_or("all"))
            }
            Self::RegenerateEmbeddings { .. } => "regenerate-embeddings".to_string(),
            Self::RunLens { lens_id, run_id } => format!("lens:{lens_id}:run:{run_id}"),
            Self::ReextractLensRow { lens_id, email_id } => {
                format!("lens:{lens_id}:reextract:{email_id}")
            }
            Self::BackfillMemory { account_id, .. } => format!("memory-backfill:{account_id}"),
            Self::BackfillTasks { account_id, .. } => format!("task-backfill:{account_id}"),
            Self::DownloadModel { model_id } => format!("download-model:{model_id}"),
            Self::SyncAccount { account_id, .. } => format!("sync:{account_id}"),
            Self::RedownloadEmptyEmails { account_id, .. } => {
                format!("redownload-empty:{account_id}")
            }
        }
    }
}

// ── TaskDispatcher trait ──────────────────────────────────────────────────────

type BoxFuture<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// Abstraction over the background task queues.
///
/// The real implementation (`RealDispatcher`) routes each `BackgroundTask`
/// variant to the appropriate `TaskQueue` and executes the supplied async
/// closure. The test implementation (`FakeDispatcher`) records the task
/// without running it — tests assert on `recorded()`.
///
/// ## Usage
///
/// ```rust,ignore
/// async fn my_command(dispatcher: &dyn TaskDispatcher, email_id: String) {
///     dispatcher
///         .dispatch(BackgroundTask::GenerateDraft {
///             email_id,
///             request_id: uuid::Uuid::new_v4().to_string(),
///         }, || Box::pin(async move { /* real work */ }))
///         .await;
/// }
/// ```
pub trait TaskDispatcher: Send + Sync {
    /// Enqueue `task` for execution. The `make_fut` closure is only called by
    /// the real dispatcher — `FakeDispatcher` ignores it.
    fn dispatch<'a>(
        &'a self,
        task: BackgroundTask,
        make_fut: Box<dyn FnOnce() -> BoxFuture<'static> + Send + 'a>,
    ) -> BoxFuture<'a>;

    /// All tasks recorded so far (only meaningful on `FakeDispatcher`).
    fn recorded(&self) -> Vec<BackgroundTask> {
        vec![]
    }
}

/// Production dispatcher: routes tasks to the appropriate `TaskQueue`.
pub struct RealDispatcher {
    pub ai_queue: crate::services::task_queue::TaskQueue,
    pub ai_background: crate::services::task_queue::TaskQueue,
    pub db_queue: crate::services::task_queue::TaskQueue,
}

impl TaskDispatcher for RealDispatcher {
    fn dispatch<'a>(
        &'a self,
        task: BackgroundTask,
        make_fut: Box<dyn FnOnce() -> BoxFuture<'static> + Send + 'a>,
    ) -> BoxFuture<'a> {
        let queue = match task.queue() {
            QueueKind::AiInteractive => self.ai_queue.clone(),
            QueueKind::AiBackground | QueueKind::Db => self.ai_background.clone(),
            // Sync tasks must use per-account queues (managed by AppState);
            // callers that need sync routing should not use RealDispatcher.
            QueueKind::Sync => self.ai_background.clone(),
        };
        let label = task.label();
        Box::pin(async move {
            let fut = make_fut();
            queue.submit_named(&label, fut).await;
        })
    }
}

/// Test dispatcher: records `BackgroundTask` values, never runs futures.
///
/// ## Example
///
/// ```rust,ignore
/// let d = Arc::new(FakeDispatcher::new());
/// my_fn_under_test(&d, "email-1").await;
/// assert_eq!(d.recorded(), vec![BackgroundTask::GenerateDraft { … }]);
/// ```
#[derive(Default)]
pub struct FakeDispatcher {
    recorded: RwLock<Vec<BackgroundTask>>,
}

impl FakeDispatcher {
    pub fn new() -> Self {
        Self::default()
    }
}

impl TaskDispatcher for FakeDispatcher {
    fn dispatch<'a>(
        &'a self,
        task: BackgroundTask,
        _make_fut: Box<dyn FnOnce() -> BoxFuture<'static> + Send + 'a>,
    ) -> BoxFuture<'a> {
        self.recorded.write().unwrap_or_else(PoisonError::into_inner).push(task);
        Box::pin(async {})
    }

    fn recorded(&self) -> Vec<BackgroundTask> {
        self.recorded.read().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_routing_is_correct() {
        assert_eq!(
            BackgroundTask::GenerateDraft {
                email_id: "x".into(),
                request_id: "r".into()
            }
            .queue(),
            QueueKind::AiInteractive
        );
        assert_eq!(
            BackgroundTask::RunLens { lens_id: 1, run_id: 1 }.queue(),
            QueueKind::AiBackground
        );
        assert_eq!(
            BackgroundTask::SyncAccount {
                account_id: "a".into(),
                request_id: "r".into()
            }
            .queue(),
            QueueKind::Sync
        );
    }

    #[test]
    fn label_includes_key_ids() {
        let t = BackgroundTask::RunLens { lens_id: 7, run_id: 42 };
        assert!(t.label().contains("lens:7"));
        assert!(t.label().contains("run:42"));
    }

    #[test]
    fn task_is_serde_roundtrippable() {
        let t = BackgroundTask::GenerateDraft {
            email_id: "abc".into(),
            request_id: "req-1".into(),
        };
        let json = serde_json::to_string(&t).unwrap();
        let back: BackgroundTask = serde_json::from_str(&json).unwrap();
        assert_eq!(t, back);
    }

    // ── FakeDispatcher ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn fake_dispatcher_records_tasks_without_running_future() {
        let d = FakeDispatcher::new();

        // Dispatch two different task kinds.
        d.dispatch(
            BackgroundTask::GenerateDraft {
                email_id: "e1".into(),
                request_id: "r1".into(),
            },
            Box::new(|| Box::pin(async { panic!("should not execute") })),
        )
        .await;
        d.dispatch(
            BackgroundTask::SyncAccount {
                account_id: "acc".into(),
                request_id: "r2".into(),
            },
            Box::new(|| Box::pin(async { panic!("should not execute") })),
        )
        .await;

        let recorded = d.recorded();
        assert_eq!(recorded.len(), 2);
        assert_eq!(
            recorded[0],
            BackgroundTask::GenerateDraft {
                email_id: "e1".into(),
                request_id: "r1".into()
            }
        );
        assert_eq!(
            recorded[1],
            BackgroundTask::SyncAccount {
                account_id: "acc".into(),
                request_id: "r2".into()
            }
        );
    }

    #[tokio::test]
    async fn fake_dispatcher_starts_empty() {
        let d = FakeDispatcher::new();
        assert!(d.recorded().is_empty());
    }
}
