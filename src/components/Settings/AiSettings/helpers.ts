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
 * The embedding model to show after switching to `next`: the one `remembered`
 * for that provider (see `AiConfig.remembered`), else a model it can run — or
 * none, which for OpenRouter means keyword-only search until the user picks
 * one. An id only means something to the provider it came from, so another
 * provider's model is never carried over.
 */
export function embeddingModelForProvider(
  next: AiConfigState['provider'],
  remembered: string | null,
  available: { catalog: CatalogModel[]; ollamaEmbedModels: string[] },
): string {
  if (remembered !== null) return remembered;
  if (next === 'ollama') return available.ollamaEmbedModels[0] ?? '';
  if (next === 'llamacpp') {
    const models = available.catalog.filter((m) => m.kind === 'embedding');
    return (models.find((m) => m.isLocal) ?? models.find((m) => m.recommended))?.id ?? '';
  }
  return '';
}

/**
 * The OpenRouter chat model offered when none was chosen yet (Settings and
 * onboarding). Picked by the developer on 30/09/2026 from the public
 * catalogue: it supports tool calls and has a zero-data-retention endpoint.
 * The user can type any other model id.
 */
export const DEFAULT_OPENROUTER_CHAT_MODEL = 'google/gemini-3.5-flash-lite';

/**
 * The chat model to show after switching to `next`: the one `remembered` for
 * that provider, else the first model Ollama or the in-app runtime can run,
 * and for OpenRouter — which has no list to pick from — its default, never
 * another provider's id.
 */
export function chatModelForProvider(
  next: AiConfigState['provider'],
  remembered: string | null,
  available: { catalog: CatalogModel[]; ollamaModels: string[] },
): string {
  if (remembered) return remembered;
  if (next === 'ollama') return available.ollamaModels[0] ?? '';
  if (next === 'llamacpp') return available.catalog.find((m) => m.kind === 'chat' && m.isLocal)?.id ?? '';
  return DEFAULT_OPENROUTER_CHAT_MODEL;
}

/**
 * Whether saving `next` replaces the email index: every vector made with the
 * `saved` model is deleted and rebuilt. With no saved model there is nothing
 * to replace.
 */
export function embeddingModelChanged(saved: string, next: string): boolean {
  return saved !== '' && saved !== next;
}

/**
 * Whether Save must first ask the backend to check the OpenRouter embedding
 * model: any model other than `validatedModel`, the one that already passed.
 */
export function needsEmbeddingProbe(config: AiConfigState, validatedModel: string | null): boolean {
  return config.provider === 'openrouter' && config.embeddingModel !== '' && config.embeddingModel !== validatedModel;
}
