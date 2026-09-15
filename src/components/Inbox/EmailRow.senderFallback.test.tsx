// Regression test: a message whose From header has no display name (e.g. a
// bare `From: info@example.com` sent over SMTP) syncs with `sender: ''`. The
// row must show the address instead of an empty name column.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { Email } from '@/types';
import { EmailRow } from './EmailRow';

const email: Email = {
  id: 'email-1',
  accountId: 'acct-1',
  threadId: 'thread-1',
  messageId: null,
  subject: 'Collaboration proposal',
  sender: '',
  senderEmail: 'info@example.com',
  recipients: ['client@example.com'],
  cc: [],
  body: 'body',
  snippet: 'snippet',
  timestamp: 1_700_000_000,
  isRead: true,
  triageStatus: null,
  category: 'primary',
  mailbox: 'sent',
  isSent: true,
};

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

describe('EmailRow sender without a display name', () => {
  it.each([
    ['default', false],
    ['compact', true],
  ])('shows the sender address in the %s row', (_layout, compact) => {
    act(() => {
      root.render(<EmailRow email={email} isSelected={false} onClick={() => {}} compact={compact} />);
    });
    const nameCell = container.querySelector('span.truncate');
    expect(nameCell?.textContent).toBe('info@example.com');
  });
});
