// Contract: the AI provider and model commands are called from api.ts with the argument
// names their Rust signatures declare (see ./contract.ts).
import { describe, expect, it } from 'vitest';
import {
  itMatchesRustArguments,
  rustEnumVariants,
  rustJsonKeys,
  rustStructFields,
  tsInterfaceFields,
  tsStringUnion,
} from './contract';

describe('api.ts ↔ Rust: AI provider and model commands', () => {
  itMatchesRustArguments(['ai_config.rs', 'ai_models.rs']);
});

describe('AI provider responses have the shape the frontend types declare', () => {
  it('get_ai_config answers every AiConfig field, `remembered` and the validated OpenRouter model included', () => {
    expect(rustJsonKeys('ai_config.rs', 'get_ai_config')).toEqual(tsInterfaceFields('AiConfig'));
    expect(tsInterfaceFields('AiConfig')).toEqual(
      expect.arrayContaining(['remembered', 'openRouterValidatedEmbeddingModel']),
    );
  });

  it('get_ai_provider_activity answers AiProviderActivity with AiWorkItem rows', () => {
    expect(rustStructFields('commands/ai_config.rs', 'AiProviderActivity')).toEqual(
      tsInterfaceFields('AiProviderActivity'),
    );
    expect(rustStructFields('services/ai_activity.rs', 'AiWorkItem')).toEqual(tsInterfaceFields('AiWorkItem'));
  });

  it('AiWorkKind is the Rust enum minus the kinds that never reach the provider', () => {
    const rust = rustEnumVariants('services/ai_activity.rs', 'AiWorkKind');
    expect(tsStringUnion('AiWorkKind')).toEqual(rust.filter((k) => k !== 'junkScoring' && k !== 'other'));
  });

  it('model lists answer AiModelInfo', () => {
    expect(rustStructFields('ai/provider.rs', 'ModelInfo')).toEqual(tsInterfaceFields('AiModelInfo'));
  });
});
