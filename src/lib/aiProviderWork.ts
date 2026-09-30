// Which background AI work a change to the AI provider or to a model cuts
// across, for the dialog that asks whether to stop it or wait. What runs
// comes from `get_ai_provider_activity`. Pure.

import type { AiProviderActivity, AiWorkItem, AiWorkKind } from '@/types';

/** What a save is about to change. */
export interface AiChange {
  provider: boolean;
  model: boolean;
  embeddingModel: boolean;
}

/** Which model each kind of work uses: the chat model, the embedding model,
 *  or both (memory extraction embeds the facts it extracts). */
const KIND_USES: Record<AiWorkKind, { chat: boolean; embedding: boolean }> = {
  embeddingsRebuild: { chat: false, embedding: true },
  embeddingsGeneration: { chat: false, embedding: true },
  classification: { chat: true, embedding: false },
  memoryExtraction: { chat: true, embedding: true },
  taskExtraction: { chat: true, embedding: false },
  lensExtraction: { chat: true, embedding: false },
};

export function changesAnything(change: AiChange): boolean {
  return change.provider || change.model || change.embeddingModel;
}

/** The kinds of work that `change` cuts across. */
export function affectedKinds(change: AiChange): AiWorkKind[] {
  return (Object.keys(KIND_USES) as AiWorkKind[]).filter((kind) => {
    const uses = KIND_USES[kind];
    return change.provider || (change.model && uses.chat) || (change.embeddingModel && uses.embedding);
  });
}

/** The running and queued work that `change` cuts across. */
export function affectedWork(change: AiChange, items: AiWorkItem[]): AiWorkItem[] {
  const kinds = affectedKinds(change);
  return items.filter((item) => kinds.includes(item.kind));
}

export interface WorkLine {
  kind: AiWorkKind;
  running: boolean;
  /** Emails done out of the batch in hand, when the running task reports it. */
  progress: { current: number; total: number } | null;
  /** Tasks of this kind still waiting in the queue. */
  queued: number;
}

/** One line per kind of work, in the order the queue holds them. */
export function summarizeWork(items: AiWorkItem[]): WorkLine[] {
  const lines: WorkLine[] = [];
  for (const item of items) {
    let line = lines.find((l) => l.kind === item.kind);
    if (!line) {
      line = { kind: item.kind, running: false, progress: null, queued: 0 };
      lines.push(line);
    }
    if (item.running) {
      line.running = true;
      line.progress = item.progress;
    } else {
      line.queued += 1;
    }
  }
  return lines;
}

/** Whether the work is sending email text to OpenRouter to be embedded. */
export function sendsMailToOpenRouter(provider: AiProviderActivity['provider'], items: AiWorkItem[]): boolean {
  return (
    provider === 'openrouter' && items.some((i) => i.kind === 'embeddingsRebuild' || i.kind === 'embeddingsGeneration')
  );
}
