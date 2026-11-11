// The start of every OpenRouter embedding-model selector: "none", then the
// recommended models labelled with the mail they suit. Settings appends
// OpenRouter's catalogue; onboarding, which has no saved key to list it with,
// appends an entry for typing another model id.

import type { SelectOption } from '@/components/shared/Select';
import type { AiModelInfo } from '@/types';
import { RECOMMENDED_OPENROUTER_EMBEDDING_MODELS } from './openRouterEmbeddingModels';

/** The translation keys the options need, whatever namespaces the caller loaded. */
type Translate = (
  key:
    | 'settings:openRouter.embeddingNone'
    | 'settings:openRouter.embeddingRecommendedMultilingual'
    | 'settings:openRouter.embeddingRecommendedEnglish',
) => string;

export function recommendedEmbeddingOptions(t: Translate): {
  none: SelectOption<string>;
  recommended: SelectOption<string>[];
} {
  return {
    none: { value: '', label: t('settings:openRouter.embeddingNone') },
    recommended: RECOMMENDED_OPENROUTER_EMBEDDING_MODELS.map((m) => ({
      value: m.id,
      label: `${m.id} — ${t(
        m.languages === 'multilingual'
          ? 'settings:openRouter.embeddingRecommendedMultilingual'
          : 'settings:openRouter.embeddingRecommendedEnglish',
      )}`,
    })),
  };
}

/**
 * Every option of an OpenRouter embedding-model selector: "none", the
 * recommended models, then the rest of OpenRouter's catalogue (`listed`), each
 * model once. `current`, when no list has it (or the list could not load),
 * stays selectable.
 */
export function openRouterEmbeddingSelectOptions(
  t: Translate,
  listed: AiModelInfo[],
  current: string,
): SelectOption<string>[] {
  const { none, recommended } = recommendedEmbeddingOptions(t);
  const rest = listed
    .filter((m) => !recommended.some((r) => r.value === m.id))
    .map((m) => ({ value: m.id, label: m.id }));
  const known = [...recommended, ...rest];
  return [
    none,
    ...(current !== '' && !known.some((o) => o.value === current) ? [{ value: current, label: current }] : []),
    ...known,
  ];
}
