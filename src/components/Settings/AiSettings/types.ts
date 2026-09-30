export interface AiConfigState {
  provider: 'llamacpp' | 'ollama' | 'openrouter';
  model: string;
  embeddingModel: string;
  /** Whether the saved embedding model may be used as it stands. False only
   *  for an OpenRouter model that never passed the dimension check. */
  embeddingModelValidated: boolean;
  monthlyBudgetUsd: number;
  hasApiKey: boolean;
  thinkingEnabled: boolean;
  zeroDataRetention: boolean;
}

export type RoutingMode = 'always_rag' | 'auto' | 'always_tools';
export const DEFAULT_ROUTING_MODE: RoutingMode = 'always_rag';
export const ROUTING_MODES: RoutingMode[] = ['always_rag', 'auto', 'always_tools'];

export function isRoutingMode(v: string | null): v is RoutingMode {
  return v != null && (ROUTING_MODES as string[]).includes(v);
}
