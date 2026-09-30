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

  it('restores the model remembered for the provider', () => {
    expect(embeddingModelForProvider('openrouter', 'vendor/embed', lists)).toBe('vendor/embed');
    expect(embeddingModelForProvider('ollama', 'ollama-other', lists)).toBe('ollama-other');
    expect(embeddingModelForProvider('llamacpp', 'embed-recommended-gguf', lists)).toBe('embed-recommended-gguf');
  });

  it('keeps OpenRouter on no model when that is what was chosen', () => {
    expect(embeddingModelForProvider('openrouter', '', lists)).toBe('');
  });

  it('offers a model the provider can run when none is remembered, and none for OpenRouter', () => {
    expect(embeddingModelForProvider('openrouter', null, lists)).toBe('');
    expect(embeddingModelForProvider('ollama', null, lists)).toBe('ollama-embed');
    expect(embeddingModelForProvider('llamacpp', null, lists)).toBe('embed-local-gguf');
  });

  it('falls back to the recommended in-app model, then to none', () => {
    const notDownloaded = { catalog: catalog.slice(0, 2), ollamaEmbedModels: [] };
    expect(embeddingModelForProvider('llamacpp', null, notDownloaded)).toBe('embed-recommended-gguf');
    expect(embeddingModelForProvider('llamacpp', null, { catalog: [], ollamaEmbedModels: [] })).toBe('');
    expect(embeddingModelForProvider('ollama', null, notDownloaded)).toBe('');
  });
});

describe('chatModelForProvider', () => {
  const catalog = [
    catalogModel('chat-recommended-gguf', { kind: 'chat', recommended: true }),
    catalogModel('chat-local-gguf', { kind: 'chat', isLocal: true }),
    catalogModel('embed-local-gguf', { isLocal: true }),
  ];
  const lists = { catalog, ollamaModels: ['ollama-chat', 'ollama-chat-2'] };

  it('restores the model remembered for the provider', () => {
    expect(chatModelForProvider('openrouter', 'vendor/model', lists)).toBe('vendor/model');
    expect(chatModelForProvider('ollama', 'ollama-chat-2', lists)).toBe('ollama-chat-2');
    expect(chatModelForProvider('llamacpp', 'chat-recommended-gguf', lists)).toBe('chat-recommended-gguf');
  });

  it('offers the provider default when no model is remembered', () => {
    for (const nothing of [null, '']) {
      expect(chatModelForProvider('openrouter', nothing, lists)).toBe(DEFAULT_OPENROUTER_CHAT_MODEL);
      expect(chatModelForProvider('ollama', nothing, lists)).toBe('ollama-chat');
      expect(chatModelForProvider('llamacpp', nothing, lists)).toBe('chat-local-gguf');
    }
  });

  it('picks nothing when the target provider has no model to run', () => {
    const nothingLocal = { catalog: catalog.slice(0, 1), ollamaModels: [] };
    expect(chatModelForProvider('llamacpp', null, nothingLocal)).toBe('');
    expect(chatModelForProvider('ollama', null, nothingLocal)).toBe('');
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
    monthlyBudgetUsd: 0,
    hasApiKey: true,
    thinkingEnabled: false,
    zeroDataRetention: false,
  };

  it('asks for a probe only for an OpenRouter model that has not passed one', () => {
    expect(needsEmbeddingProbe(base, 'vendor/embed')).toBe(false);
    expect(needsEmbeddingProbe(base, 'vendor/previous')).toBe(true);
    expect(needsEmbeddingProbe(base, null)).toBe(true);
    expect(needsEmbeddingProbe({ ...base, embeddingModel: '' }, null)).toBe(false);
    expect(needsEmbeddingProbe({ ...base, provider: 'ollama' }, null)).toBe(false);
    expect(needsEmbeddingProbe({ ...base, provider: 'llamacpp' }, null)).toBe(false);
  });
});
