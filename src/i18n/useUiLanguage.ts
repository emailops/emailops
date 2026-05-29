// Reactive hook for the UI language preference.
//
// Reads/writes through the `ui_language` SQLite preference (NOT localStorage).
// Switching language calls `i18n.changeLanguage` so all `useTranslation`
// consumers re-render with the new bundle.
import { useCallback, useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';

import { getPref, setPref } from '../lib/api';

import { FALLBACK_LANGUAGE, isSupportedLanguage, type Language } from './resources';

export interface UseUiLanguageResult {
  /** Currently active UI language (one of the supported codes). */
  language: Language;
  /** Persist a new UI language and switch i18next immediately. */
  setLanguage: (next: Language) => Promise<void>;
  /** True until the initial DB read settles. */
  isLoading: boolean;
}

/**
 * Returns the active UI language and a setter that persists to SQLite. The
 * setter awaits both the DB write and i18next's language switch so callers
 * can rely on the UI being fully reskinned after `await setLanguage(...)`.
 */
export function useUiLanguage(): UseUiLanguageResult {
  const { i18n } = useTranslation();
  const initial = isSupportedLanguage(i18n.language) ? (i18n.language as Language) : FALLBACK_LANGUAGE;
  const [language, setLanguageState] = useState<Language>(initial);
  const [isLoading, setIsLoading] = useState(true);

  // On mount, reconcile with the DB in case i18next was initialised before
  // the preference row arrived (e.g. a race between bootstrap and a fresh
  // install completing onboarding).
  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const stored = await getPref('ui_language');
        if (cancelled) return;
        if (isSupportedLanguage(stored)) {
          setLanguageState(stored);
          if (i18n.language !== stored) {
            await i18n.changeLanguage(stored);
          }
        }
      } finally {
        if (!cancelled) setIsLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [i18n]);

  const setLanguage = useCallback(
    async (next: Language) => {
      await setPref('ui_language', next);
      await i18n.changeLanguage(next);
      setLanguageState(next);
    },
    [i18n],
  );

  return { language, setLanguage, isLoading };
}
