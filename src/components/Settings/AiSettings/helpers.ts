import type { CatalogModel } from '@/types';
import type { AiConfigState } from './types';

export function formatBytes(bytes: number): string {
  if (bytes === 0) return '0 B';
  const gb = bytes / 1e9;
  if (gb >= 1) return `${gb.toFixed(1)} GB`;
  const mb = bytes / 1e6;
  return `${mb.toFixed(0)} MB`;
}

export function formatProgress(downloaded: number, total: number): string {
  if (total === 0) return '…';
  const pct = Math.round((downloaded / total) * 100);
  return `${pct}% · ${formatBytes(downloaded)} / ${formatBytes(total)}`;
}

/** Default prompt budget for remote (OpenRouter) models, in tokens. */
export const DEFAULT_CONTEXT_BUDGET = 32768;
/** Smallest budget the backend accepts for `chat.remote_n_ctx_budget`. */
export const MIN_CONTEXT_BUDGET = 4096;

/** The budget to show for a stored `chat.remote_n_ctx_budget` value. */
export function contextBudgetFromPref(raw: string | null): number {
  const n = raw != null && raw.trim() !== '' ? Number.parseInt(raw, 10) : Number.NaN;
  return Number.isFinite(n) && n >= MIN_CONTEXT_BUDGET ? n : DEFAULT_CONTEXT_BUDGET;
}

/** The value to store for a budget typed in Settings. */
export function contextBudgetToPref(tokens: number): string {
  if (!Number.isFinite(tokens)) return String(DEFAULT_CONTEXT_BUDGET);
  return String(Math.max(MIN_CONTEXT_BUDGET, Math.round(tokens)));
}

/**
 * The embedding model to show after switching to `next`. The preference is
 * shared by every provider, and an id only means something to the provider it
 * came from: returning to the saved provider restores the saved model, any
 * other provider gets a model it can run — or none, which for OpenRouter means
 * keyword-only search until the user picks one.
 */
export function embeddingModelForProvider(
  next: AiConfigState['provider'],
  saved: { provider: string; embeddingModel: string },
  available: { catalog: CatalogModel[]; ollamaEmbedModels: string[] },
): string {
  if (next === saved.provider) return saved.embeddingModel;
  if (next === 'ollama') return available.ollamaEmbedModels[0] ?? '';
  if (next === 'llamacpp') {
    const models = available.catalog.filter((m) => m.kind === 'embedding');
    return (models.find((m) => m.isLocal) ?? models.find((m) => m.recommended))?.id ?? '';
  }
  return '';
}

/**
 * The chat model to show after switching to `next`. Like the embedding model,
 * the preference is shared by every provider: returning to the saved provider
 * restores the saved model, Ollama and the in-app runtime get the first model
 * they can run, and OpenRouter — which has no list to pick from — gets none,
 * so its field shows the placeholder instead of another provider's id.
 */
export function chatModelForProvider(
  next: AiConfigState['provider'],
  saved: { provider: string; model: string },
  available: { catalog: CatalogModel[]; ollamaModels: string[] },
): string {
  if (next === saved.provider) return saved.model;
  if (next === 'ollama') return available.ollamaModels[0] ?? '';
  if (next === 'llamacpp') return available.catalog.find((m) => m.kind === 'chat' && m.isLocal)?.id ?? '';
  return '';
}

/**
 * Whether Save must first ask the backend to check the OpenRouter embedding
 * model: it is new, or it was saved without ever passing the check.
 */
export function needsEmbeddingProbe(config: AiConfigState, savedEmbeddingModel: string): boolean {
  if (config.provider !== 'openrouter' || config.embeddingModel === '') return false;
  return config.embeddingModel !== savedEmbeddingModel || !config.embeddingModelValidated;
}
