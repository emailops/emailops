// Which long-running AI processes are running now, and how far along they
// are — for the status bar. What runs comes from the task-queue snapshot
// (`get_queue_state`), whose task labels name the process; the numbers come
// from each process's own status API or progress event. Pure.

import type { AllQueuesState } from '@/types';

export type ActivityKind =
  | 'lensBackfill'
  | 'lensRun'
  | 'memoryBackfill'
  | 'memoryExtract'
  | 'tasksBackfill'
  | 'tasksExtract'
  | 'classification'
  | 'embeddings'
  | 'junk'
  | 'modelDownload';

export interface ActivityProgress {
  current: number;
  /** `null` when only a remaining count is known (memory / task backfills). */
  total: number | null;
  unit?: 'items' | 'bytes' | 'remaining';
}

export interface Activity {
  key: string;
  kind: ActivityKind;
  /** The lens, account or model the process works on. */
  target: string | null;
  progress: ActivityProgress | null;
}

/** Kinds whose progress is keyed by the process alone (one runs at a time). */
const KIND_KEYED: ReadonlySet<ActivityKind> = new Set(['classification', 'embeddings']);

/** The progress-map key for a process. */
export function progressKey(kind: ActivityKind, target: string | null): string {
  return KIND_KEYED.has(kind) ? kind : `${kind}:${target ?? ''}`;
}

/** The expensive AI process a queue task label stands for, or null for
 *  short or non-AI work (chat turns, drafts, translations, sync). */
export function classifyTask(name: string): { kind: ActivityKind; target: string | null } | null {
  const parts = name.split(':');
  const [head, sub] = parts;
  const at = (i: number) => parts[i] ?? null;
  switch (head) {
    case 'lens':
      return { kind: sub === 'backfill' ? 'lensBackfill' : 'lensRun', target: at(2) };
    case 'memory':
      return { kind: sub === 'backfill' ? 'memoryBackfill' : 'memoryExtract', target: at(2) };
    case 'tasks':
      return { kind: sub === 'backfill' ? 'tasksBackfill' : 'tasksExtract', target: at(2) };
    case 'classify':
    case 'reclassify':
      return { kind: 'classification', target: at(2) };
    case 'embeddings':
      return { kind: 'embeddings', target: at(2) };
    case 'junk':
      return { kind: 'junk', target: at(2) };
    case 'model_download':
      return { kind: 'modelDownload', target: at(1) };
    default:
      return null;
  }
}

/** The running expensive processes, one per process, with their progress. */
export function summarizeActivities(queues: AllQueuesState, progress: Record<string, ActivityProgress>): Activity[] {
  const out: Activity[] = [];
  const seen = new Set<string>();
  for (const q of [queues.ai, queues.aiBackground, queues.db]) {
    for (const task of q.running) {
      const c = classifyTask(task.name);
      if (!c) continue;
      const key = progressKey(c.kind, c.target);
      if (seen.has(key)) continue;
      seen.add(key);
      out.push({ key, kind: c.kind, target: c.target, progress: progress[key] ?? null });
    }
  }
  return out;
}
