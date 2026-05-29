// i18next initialisation.
//
// The boot sequence in `main.tsx` is:
//   1. Read `ui_language` from SQLite via `getPref('ui_language')`.
//   2. If unset, ask the backend for the OS locale via `getSystemLocale()`.
//   3. Fall back to "en" if both are unavailable / unsupported.
//   4. Call `initI18n(lang)` to wire up i18next, then mount React inside
//      `<I18nextProvider>`.
//
// Persisted preferences are **never** read from localStorage — see the user
// memory rule "No localStorage for preferences".
import i18n from 'i18next';
import { initReactI18next } from 'react-i18next';

import { defaultNS, FALLBACK_LANGUAGE, isSupportedLanguage, type Language, NAMESPACES, resources } from './resources';

let initialised = false;

/**
 * Initialise i18next with the resolved UI language. Idempotent — repeat calls
 * just switch the active language. Returns the i18n instance for callers that
 * want to await the underlying init promise (mostly tests).
 */
export async function initI18n(language: string | null | undefined): Promise<typeof i18n> {
  const lng: Language = isSupportedLanguage(language) ? language : FALLBACK_LANGUAGE;

  if (!initialised) {
    await i18n.use(initReactI18next).init({
      resources,
      lng,
      fallbackLng: FALLBACK_LANGUAGE,
      defaultNS,
      ns: NAMESPACES as unknown as string[],
      interpolation: {
        // React already escapes by default, so i18next can stay out of the way.
        escapeValue: false,
      },
      returnNull: false,
    });
    initialised = true;
  } else if (i18n.language !== lng) {
    await i18n.changeLanguage(lng);
  }

  return i18n;
}

/** Re-export the configured i18n singleton for direct access where needed. */
export { default as i18n } from 'i18next';
export type { Language, Namespace } from './resources';
export {
  FALLBACK_LANGUAGE,
  isSupportedLanguage,
  NAMESPACES,
  NATIVE_NAMES,
  SUPPORTED_LANGUAGES,
} from './resources';
export { useUiLanguage } from './useUiLanguage';
