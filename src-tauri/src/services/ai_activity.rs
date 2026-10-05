//! What the AI background queue is doing with the configured AI provider, and
//! how to stop it — for the dialog shown before the provider or a model is
//! changed.
//!
//! The queue names every task (`embeddings:rebuild:<account>`,
//! `classify:new_emails:<account>:<phase>`, …). [`work_kind`] maps a name to a
//! small stable set of kinds the UI translates; only kinds that send work to
//! the AI provider are reported or stopped.

use serde::{Deserialize, Serialize};

use crate::services::task_queue::{QueueStateSnapshot, TaskInfo, TaskProgress, TaskQueue};

/// A kind of work on the AI background queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AiWorkKind {
    EmbeddingsRebuild,
    EmbeddingsGeneration,
    Classification,
    JunkScoring,
    MemoryExtraction,
    TaskExtraction,
    LensExtraction,
    /// The email agent evaluating new mail, events or a panel.
    AgentRules,
    Other,
}

impl AiWorkKind {
    /// Whether this work calls the configured AI provider. Junk scoring is
    /// deterministic and model-free; `Other` is work this module does not
    /// know (a language detection, a memory consolidation), which is short
    /// and left to finish.
    pub fn uses_ai_provider(self) -> bool {
        !matches!(self, Self::JunkScoring | Self::Other)
    }
}

/// The kind of work a queue task name stands for.
pub fn work_kind(task_name: &str) -> AiWorkKind {
    let mut parts = task_name.split(':');
    match (parts.next().unwrap_or_default(), parts.next().unwrap_or_default()) {
        ("embeddings", "rebuild") => AiWorkKind::EmbeddingsRebuild,
        ("embeddings", _) => AiWorkKind::EmbeddingsGeneration,
        ("classify" | "reclassify", _) => AiWorkKind::Classification,
        ("junk", _) => AiWorkKind::JunkScoring,
        // Consolidation alone merges stored facts without the provider.
        ("memory", "consolidate") => AiWorkKind::Other,
        ("memory", _) => AiWorkKind::MemoryExtraction,
        ("tasks", _) => AiWorkKind::TaskExtraction,
        ("lens", _) => AiWorkKind::LensExtraction,
        ("agent", _) => AiWorkKind::AgentRules,
        _ => AiWorkKind::Other,
    }
}

/// One running or queued task that uses the AI provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiWorkItem {
    pub kind: AiWorkKind,
    /// `false` while the task waits in the queue.
    pub running: bool,
    /// Asked to stop and not finished yet.
    pub stopping: bool,
    /// Emails done out of the batch in hand, for the kinds that report it.
    pub progress: Option<TaskProgress>,
}

/// The tasks of `queue` that use the AI provider: running first, then queued.
pub fn provider_work(queue: &QueueStateSnapshot) -> Vec<AiWorkItem> {
    let item = |task: &TaskInfo, running: bool| {
        let kind = work_kind(&task.name);
        kind.uses_ai_provider().then(|| AiWorkItem {
            kind,
            running,
            stopping: task.cancel_requested(),
            progress: task.progress(),
        })
    };
    let running = queue.running.iter().filter_map(|task| item(task, true));
    let queued = queue.pending.iter().filter_map(|task| item(task, false));
    running.chain(queued).collect()
}

/// Ask the running and queued work of the given kinds to stop, and return how
/// many tasks were asked. Kinds that do not use the AI provider are ignored.
pub fn cancel_provider_work(queue: &TaskQueue, kinds: &[AiWorkKind]) -> usize {
    queue.cancel_matching(|name| {
        let kind = work_kind(name);
        kind.uses_ai_provider() && kinds.contains(&kind)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_task_name_maps_to_its_kind() {
        use AiWorkKind::*;
        // Every name the AI background queue is given today.
        let cases = [
            ("embeddings:rebuild:all", EmbeddingsRebuild),
            ("embeddings:rebuild:acc-1", EmbeddingsRebuild),
            ("embeddings:generate:all", EmbeddingsGeneration),
            ("embeddings:after_sync:acc-1:final", EmbeddingsGeneration),
            ("classify:new_emails:acc-1:early", Classification),
            ("classify:account:acc-1", Classification),
            ("reclassify:account:acc-1", Classification),
            ("reclassify:rule_update:7", Classification),
            ("agent:emails:acc-1:final", AgentRules),
            ("agent:events", AgentRules),
            ("agent:panel:p-1", AgentRules),
            ("agent:action:a-1", AgentRules),
            ("junk:score:acc-1:final", JunkScoring),
            ("junk:backfill:acc-1", JunkScoring),
            ("memory:extract:acc-1:final", MemoryExtraction),
            ("memory:extract+consolidate:acc-1:final", MemoryExtraction),
            ("memory:backfill:acc-1", MemoryExtraction),
            ("memory:consolidate:acc-1:final", Other),
            ("tasks:extract:acc-1:final", TaskExtraction),
            ("tasks:backfill:acc-1", TaskExtraction),
            ("lens:backfill:lens-1", LensExtraction),
            ("lens:incremental:acc-1:12", LensExtraction),
            ("lens:single:lens-1", LensExtraction),
            ("detect-lang:email-1", Other),
            ("unnamed", Other),
            ("", Other),
        ];
        for (name, kind) in cases {
            assert_eq!(work_kind(name), kind, "{name}");
        }
    }

    #[test]
    fn only_junk_scoring_and_unknown_work_run_without_the_provider() {
        use AiWorkKind::*;
        for kind in [
            EmbeddingsRebuild,
            EmbeddingsGeneration,
            Classification,
            MemoryExtraction,
            TaskExtraction,
            LensExtraction,
        ] {
            assert!(kind.uses_ai_provider(), "{kind:?}");
        }
        for kind in [JunkScoring, Other] {
            assert!(!kind.uses_ai_provider(), "{kind:?}");
        }
    }

    async fn drain(queue: &TaskQueue) {
        for _ in 0..100 {
            tokio::time::sleep(Duration::from_millis(20)).await;
            let s = queue.snapshot();
            if s.running.is_empty() && s.pending.is_empty() {
                return;
            }
        }
        panic!("queue did not drain: {:?}", queue.snapshot());
    }

    #[tokio::test]
    async fn provider_work_lists_running_then_queued_and_stops_only_the_kinds_asked() {
        use crate::services::task_queue::{cancel_requested, report_progress};
        let queue = TaskQueue::new(1, "test_ai_activity");
        queue
            .submit_named("embeddings:rebuild:all", async {
                report_progress(4, 500);
                while !cancel_requested() {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        for name in ["junk:score:a:final", "classify:new_emails:a:final", "detect-lang:e1"] {
            queue
                .submit_named(name, async {
                    while !cancel_requested() {
                        tokio::time::sleep(Duration::from_millis(5)).await;
                    }
                })
                .await;
        }

        let item = |kind, running, stopping, progress| AiWorkItem {
            kind,
            running,
            stopping,
            progress,
        };
        let rebuild_progress = Some(TaskProgress { current: 4, total: 500 });
        assert_eq!(
            provider_work(&queue.snapshot()),
            vec![
                item(AiWorkKind::EmbeddingsRebuild, true, false, rebuild_progress),
                item(AiWorkKind::Classification, false, false, None),
            ]
        );

        // Junk scoring is never stopped, even when asked for by name.
        let asked = cancel_provider_work(&queue, &[AiWorkKind::EmbeddingsRebuild, AiWorkKind::JunkScoring]);
        assert_eq!(asked, 1);
        assert_eq!(
            provider_work(&queue.snapshot()),
            vec![
                item(AiWorkKind::EmbeddingsRebuild, true, true, rebuild_progress),
                item(AiWorkKind::Classification, false, false, None),
            ]
        );

        // Let everything else go so the queue drains.
        queue.cancel_matching(|_| true);
        drain(&queue).await;
        assert!(provider_work(&queue.snapshot()).is_empty());
    }
}
