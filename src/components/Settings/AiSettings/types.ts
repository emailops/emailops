export interface AiConfigState {
  provider: 'llamacpp' | 'ollama' | 'openrouter';
  model: string;
  embeddingModel: string;
  monthlyBudgetUsd: number;
  hasApiKey: boolean;
  thinkingEnabled: boolean;
}

export type RoutingMode = 'always_rag' | 'auto' | 'always_tools';
export const DEFAULT_ROUTING_MODE: RoutingMode = 'always_rag';
export const ROUTING_MODES: RoutingMode[] = ['always_rag', 'auto', 'always_tools'];

export function isRoutingMode(v: string | null): v is RoutingMode {
  return v != null && (ROUTING_MODES as string[]).includes(v);
}
