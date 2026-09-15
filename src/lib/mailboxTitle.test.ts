import { describe, expect, it } from 'vitest';
import { mailboxTitle } from './mailboxTitle';

const t = (key: string) =>
  ({ 'sidebar:inbox': 'Inbox', 'sidebar:sent': 'Sent', 'sidebar:spam': 'Spam', 'sidebar:deleted': 'Deleted' })[key] ??
  key;

describe('mailboxTitle', () => {
  it('names the mailbox the user opened, not always Inbox', () => {
    expect(mailboxTitle('sent', 'Ulises', t)).toBe('Sent — Ulises');
    expect(mailboxTitle('spam', 'Ulises', t)).toBe('Spam — Ulises');
    expect(mailboxTitle('deleted', 'Ulises', t)).toBe('Deleted — Ulises');
    expect(mailboxTitle('inbox', 'Ulises', t)).toBe('Inbox — Ulises');
  });

  it('uses the folder name for a custom IMAP folder', () => {
    expect(mailboxTitle('folder:Projects/2026', 'Ulises', t)).toBe('Projects/2026 — Ulises');
  });

  it('drops the account suffix when there is no account name', () => {
    expect(mailboxTitle('sent', undefined, t)).toBe('Sent');
  });

  it('falls back to Inbox for any other view mode', () => {
    expect(mailboxTitle('tagboard', 'Ulises', t)).toBe('Inbox — Ulises');
  });
});
