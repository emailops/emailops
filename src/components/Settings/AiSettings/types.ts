/** Where the email index is computed when chat runs on an OpenAI-compatible server. */
export type EmbeddingSource = 'server' | 'inapp' | 'openrouter';

/** The OpenAI-compatible server's settings (`services::ai_server::ServerSettings`). */
export interface ServerState {
  baseUrl: string;
  embeddingSource: EmbeddingSource;
  /** Tokens; 0 reads it from the server. */
  contextWindow: number;
  hasApiKey: boolean;
  /** The remote host whose disclosure the user confirmed last. */
  disclosedHost: string | null;
}

export interface AiConfigState {
  provider: 'llamacpp' | 'ollama' | 'openrouter' | 'openai_compatible';
  model: string;
  embeddingModel: string;
  monthlyBudgetUsd: number;
  hasApiKey: boolean;
  thinkingEnabled: boolean;
  zeroDataRetention: boolean;
  /** Present once loaded; edited on the OpenAI-compatible tab. */
  server?: ServerState;
}

export type RoutingMode = 'always_rag' | 'auto' | 'always_tools';
export const DEFAULT_ROUTING_MODE: RoutingMode = 'always_rag';
export const ROUTING_MODES: RoutingMode[] = ['always_rag', 'auto', 'always_tools'];

export function isRoutingMode(v: string | null): v is RoutingMode {
  return v != null && (ROUTING_MODES as string[]).includes(v);
}
