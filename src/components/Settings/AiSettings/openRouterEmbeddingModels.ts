// OpenRouter embedding models EmailOps recommends in Settings.
//
// Measured, not assumed: on 30/09/2026 `scripts/probe_openrouter_embeddings.sh`
// asked every model in OpenRouter's embedding catalogue for a vector under the
// routing the app always sends (`data_collection: "deny"` — no provider that
// trains on or stores prompts). Each model below returned the 768 dimensions
// the vector tables hold, and still answered with zero data retention on.
// Re-run the script before changing this list. Any other model stays
// selectable and is checked the same way when it is saved.

export interface RecommendedEmbeddingModel {
  id: string;
  /** What mail it suits: most of these handle many languages, some only English. */
  languages: 'multilingual' | 'english';
}

export const RECOMMENDED_OPENROUTER_EMBEDDING_MODELS: readonly RecommendedEmbeddingModel[] = [
  { id: 'qwen/qwen3-embedding-8b', languages: 'multilingual' },
  { id: 'openai/text-embedding-3-small', languages: 'multilingual' },
  { id: 'google/gemini-embedding-001', languages: 'multilingual' },
  { id: 'qwen/qwen3-embedding-4b', languages: 'multilingual' },
  { id: 'perplexity/pplx-embed-v1-0.6b', languages: 'multilingual' },
  { id: 'thenlper/gte-base', languages: 'english' },
];
