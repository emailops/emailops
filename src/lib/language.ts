/**
 * Localized display name for an ISO 639-1 language code, in the UI locale
 * (e.g. `languageDisplayName('es', 'en')` → "Spanish", `('es', 'es')` →
 * "español"). Falls back to the raw code for unknown codes / environments
 * without `Intl.DisplayNames`.
 */
export function languageDisplayName(code: string, uiLocale: string): string {
  if (!code || code === 'und') return code;
  try {
    return new Intl.DisplayNames([uiLocale], { type: 'language' }).of(code) ?? code;
  } catch {
    return code;
  }
}
