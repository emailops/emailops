// Localized privacy policy pages on getemailops.com. Spanish is the site's
// root locale, so it has no path prefix.
import type { Language } from '../i18n/resources';

const PRIVACY_POLICY_URLS: Record<Language, string> = {
  en: 'https://getemailops.com/en/privacy/',
  es: 'https://getemailops.com/privacy/',
  fr: 'https://getemailops.com/fr/privacy/',
  de: 'https://getemailops.com/de/privacy/',
};

export function privacyPolicyUrl(language: Language): string {
  return PRIVACY_POLICY_URLS[language];
}
