// The Tag Board verification (`.claude/skills/verify-emailops`) compares what
// the board renders against the database. Cards and columns carry stable data
// attributes so the driver can read thread ids and counts instead of parsing
// visible text that changes with language and truncation.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Email } from '@/types';
import { TagEmailCard } from './TagEmailCard';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));
vi.mock('@/hooks/useFormatters', () => ({
  useFormatters: () => ({
    relativeTime: () => 'today',
    formatDate: () => 'today',
    formatTime: () => '',
    formatNumber: (n: number) => String(n),
  }),
}));

const email = {
  id: 'email-1',
  accountId: 'acct-1',
  threadId: 'thread-1',
  messageId: null,
  subject: 'Quarterly report',
  sender: 'Ada Example',
  senderEmail: 'ada@example.com',
  recipients: ['me@example.com'],
  cc: [],
  body: 'body',
  snippet: 'snippet',
  timestamp: 1_700_000_000,
  isRead: true,
  triageStatus: null,
  category: 'primary',
  mailbox: 'inbox',
  isDeleted: false,
} as unknown as Email;

let container: HTMLDivElement;
let root: Root;
beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe('TagEmailCard test hooks', () => {
  it('exposes the thread and account ids as data attributes', () => {
    act(() =>
      root.render(<TagEmailCard email={email} ownerEmail="me@example.com" isSelected={false} onSelect={() => {}} />),
    );
    const card = container.querySelector('[data-testid="tag-card"]');
    expect(card).not.toBeNull();
    expect(card?.getAttribute('data-thread-id')).toBe('thread-1');
    expect(card?.getAttribute('data-account-id')).toBe('acct-1');
  });
});
