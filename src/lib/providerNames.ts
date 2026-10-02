/**
 * How a mail provider id is shown to the user. Brand names, not translated —
 * and not derived by capitalising the id, which turned `imap` into "Imap".
 */
const MAIL_PROVIDER_NAMES: Record<string, string> = {
  gmail: 'Gmail',
  outlook: 'Outlook',
  imap: 'IMAP',
};

export function mailProviderName(provider: string): string {
  return MAIL_PROVIDER_NAMES[provider] ?? provider;
}
