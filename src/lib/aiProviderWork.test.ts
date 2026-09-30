import { describe, expect, it } from 'vitest';
import type { AiWorkItem, AiWorkKind } from '@/types';
import { affectedWork, changesAnything, sendsMailToOpenRouter, summarizeWork } from './aiProviderWork';

const item = (kind: AiWorkKind, over: Partial<AiWorkItem> = {}): AiWorkItem => ({
  kind,
  running: false,
  stopping: false,
  progress: null,
  ...over,
});

const ALL: AiWorkKind[] = [
  'embeddingsRebuild',
  'embeddingsGeneration',
  'classification',
  'memoryExtraction',
  'taskExtraction',
  'lensExtraction',
];
const items = ALL.map((k) => item(k));
const kinds = (work: AiWorkItem[]) => work.map((w) => w.kind);

describe('affectedWork', () => {
  it('is empty when nothing about the provider or the models changes', () => {
    const change = { provider: false, model: false, embeddingModel: false };
    expect(changesAnything(change)).toBe(false);
    expect(affectedWork(change, items)).toEqual([]);
  });

  it('is every kind of work when the provider changes', () => {
    expect(kinds(affectedWork({ provider: true, model: false, embeddingModel: false }, items))).toEqual(ALL);
  });

  it('is the work that writes with the chat model when only that changes', () => {
    expect(kinds(affectedWork({ provider: false, model: true, embeddingModel: false }, items))).toEqual([
      'classification',
      'memoryExtraction',
      'taskExtraction',
      'lensExtraction',
    ]);
  });

  it('is the work that embeds when only the embedding model changes', () => {
    // Memory extraction embeds the facts it extracts.
    expect(kinds(affectedWork({ provider: false, model: false, embeddingModel: true }, items))).toEqual([
      'embeddingsRebuild',
      'embeddingsGeneration',
      'memoryExtraction',
    ]);
  });

  it('is empty when the queue holds nothing the change touches', () => {
    const change = { provider: false, model: true, embeddingModel: false };
    expect(changesAnything(change)).toBe(true);
    expect(affectedWork(change, [item('embeddingsRebuild', { running: true })])).toEqual([]);
  });
});

describe('summarizeWork', () => {
  it('gives one line per kind: the running task with its progress, and how many more are queued', () => {
    const lines = summarizeWork([
      item('embeddingsGeneration', { running: true, progress: { current: 12, total: 50 } }),
      item('classification'),
      item('embeddingsGeneration'),
      item('classification', { stopping: true }),
      item('embeddingsGeneration'),
    ]);
    expect(lines).toEqual([
      { kind: 'embeddingsGeneration', running: true, progress: { current: 12, total: 50 }, queued: 2 },
      { kind: 'classification', running: false, progress: null, queued: 2 },
    ]);
  });
});

describe('sendsMailToOpenRouter', () => {
  it('is true only for Embeddings work while OpenRouter is the provider in use', () => {
    expect(sendsMailToOpenRouter('openrouter', [item('embeddingsRebuild')])).toBe(true);
    expect(sendsMailToOpenRouter('openrouter', [item('embeddingsGeneration')])).toBe(true);
    expect(sendsMailToOpenRouter('openrouter', [item('classification')])).toBe(false);
    expect(sendsMailToOpenRouter('ollama', [item('embeddingsRebuild')])).toBe(false);
    expect(sendsMailToOpenRouter('openrouter', [])).toBe(false);
  });
});
