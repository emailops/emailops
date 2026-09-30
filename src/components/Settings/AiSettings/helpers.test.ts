import { describe, expect, it } from 'vitest';
import type { CatalogModel } from '@/types';
import {
  chatModelForProvider,
  contextBudgetFromPref,
  contextBudgetToPref,
  DEFAULT_CONTEXT_BUDGET,
  DEFAULT_OPENROUTER_CHAT_MODEL,
  embeddingModelChanged,
  embeddingModelForProvider,
  MIN_CONTEXT_BUDGET,
  needsEmbeddingProbe,
} from './helpers';
import type { AiConfigState } from './types';

describe('contextBudgetFromPref', () => {
  it('falls back to the default when the preference is unset or not a budget', () => {
    for (const raw of [null, '', '0', 'lots', '-5', '512']) {
      expect(contextBudgetFromPref(raw)).toBe(DEFAULT_CONTEXT_BUDGET);
    }
  });

  it('reads a stored budget', () => {
    expect(contextBudgetFromPref('65536')).toBe(65536);
    expect(contextBudgetFromPref(' 16384 ')).toBe(16384);
  });
});

describe('contextBudgetToPref', () => {
  it('rounds and keeps the value the backend accepts', () => {
    expect(contextBudgetToPref(65536.4)).toBe('65536');
    expect(contextBudgetToPref(100)).toBe(String(MIN_CONTEXT_BUDGET));
    expect(contextBudgetToPref(Number.NaN)).toBe(String(DEFAULT_CONTEXT_BUDGET));
  });
});

function catalogModel(id: string, over: Partial<CatalogModel> = {}): CatalogModel {
  return {
    id,
    displayName: id,
    kind: 'embedding',
    sizeBytes: 1,
    contextWindow: 2048,
    license: 'test',
    minRamGb: 1,
    recommended: false,
    supportsTools: false,
    isLocal: false,
    isLinked: false,
    ...over,
  };
}

describe('embeddingModelForProvider', () => {
  const catalog = [
    catalogModel('chat-gguf', { kind: 'chat', isLocal: true }),
    catalogModel('embed-recommended-gguf', { recommended: true }),
    catalogModel('embed-local-gguf', { isLocal: true }),
  ];
  const lists = { catalog, ollamaEmbedModels: ['ollama-embed'] };

  it('never carries the embedding model of one provider over to another', () => {
    const saved = { provider: 'llamacpp', embeddingModel: 'embed-local-gguf' } as const;
    expect(embeddingModelForProvider('openrouter', saved, lists)).toBe('');
    expect(embeddingModelForProvider('ollama', saved, lists)).toBe('ollama-embed');

    const remote = { provider: 'openrouter', embeddingModel: 'vendor/embed' } as const;
    expect(embeddingModelForProvider('llamacpp', remote, lists)).toBe('embed-local-gguf');
    expect(embeddingModelForProvider('ollama', remote, lists)).toBe('ollama-embed');
  });

  it('restores the saved model when returning to the saved provider', () => {
    const saved = { provider: 'openrouter', embeddingModel: 'vendor/embed' } as const;
    expect(embeddingModelForProvider('openrouter', saved, lists)).toBe('vendor/embed');
  });

  it('falls back to the recommended in-app model, then to none', () => {
    const remote = { provider: 'openrouter', embeddingModel: 'vendor/embed' } as const;
    const notDownloaded = { catalog: catalog.slice(0, 2), ollamaEmbedModels: [] };
    expect(embeddingModelForProvider('llamacpp', remote, notDownloaded)).toBe('embed-recommended-gguf');
    expect(embeddingModelForProvider('llamacpp', remote, { catalog: [], ollamaEmbedModels: [] })).toBe('');
    expect(embeddingModelForProvider('ollama', remote, notDownloaded)).toBe('');
  });
});

describe('chatModelForProvider', () => {
  const catalog = [
    catalogModel('chat-recommended-gguf', { kind: 'chat', recommended: true }),
    catalogModel('chat-local-gguf', { kind: 'chat', isLocal: true }),
    catalogModel('embed-local-gguf', { isLocal: true }),
  ];
  const lists = { catalog, ollamaModels: ['ollama-chat', 'ollama-chat-2'] };

  it('never carries the chat model of one provider over to another', () => {
    const saved = { provider: 'llamacpp', model: 'chat-local-gguf' } as const;
    expect(chatModelForProvider('openrouter', saved, lists)).toBe(DEFAULT_OPENROUTER_CHAT_MODEL);
    expect(chatModelForProvider('ollama', saved, lists)).toBe('ollama-chat');

    const remote = { provider: 'openrouter', model: 'vendor/model' } as const;
    expect(chatModelForProvider('llamacpp', remote, lists)).toBe('chat-local-gguf');
    expect(chatModelForProvider('ollama', remote, lists)).toBe('ollama-chat');
  });

  it('restores the saved model when returning to the saved provider', () => {
    const saved = { provider: 'openrouter', model: 'vendor/model' } as const;
    expect(chatModelForProvider('openrouter', saved, lists)).toBe('vendor/model');
  });

  it('picks nothing when the target provider has no model to run', () => {
    const remote = { provider: 'openrouter', model: 'vendor/model' } as const;
    const nothingLocal = { catalog: catalog.slice(0, 1), ollamaModels: [] };
    expect(chatModelForProvider('llamacpp', remote, nothingLocal)).toBe('');
    expect(chatModelForProvider('ollama', remote, nothingLocal)).toBe('');
  });
});

describe('embeddingModelChanged', () => {
  it('is a change only when a saved model is replaced by another, or by none', () => {
    expect(embeddingModelChanged('embed-a', 'embed-b')).toBe(true);
    expect(embeddingModelChanged('embed-a', '')).toBe(true);
    expect(embeddingModelChanged('embed-a', 'embed-a')).toBe(false);
    expect(embeddingModelChanged('', 'embed-b')).toBe(false);
  });
});

describe('needsEmbeddingProbe', () => {
  const base: AiConfigState = {
    provider: 'openrouter',
    model: 'vendor/model',
    embeddingModel: 'vendor/embed',
    embeddingModelValidated: true,
    monthlyBudgetUsd: 0,
    hasApiKey: true,
    thinkingEnabled: false,
    zeroDataRetention: false,
  };

  it('asks for a probe only for an OpenRouter model that is new or was never checked', () => {
    expect(needsEmbeddingProbe(base, 'vendor/embed')).toBe(false);
    expect(needsEmbeddingProbe(base, 'vendor/previous')).toBe(true);
    expect(needsEmbeddingProbe({ ...base, embeddingModelValidated: false }, 'vendor/embed')).toBe(true);
    expect(needsEmbeddingProbe({ ...base, embeddingModel: '' }, 'vendor/embed')).toBe(false);
    expect(needsEmbeddingProbe({ ...base, provider: 'ollama' }, 'other')).toBe(false);
    expect(needsEmbeddingProbe({ ...base, provider: 'llamacpp' }, 'other')).toBe(false);
  });
});
