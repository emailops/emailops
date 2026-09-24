// A unified ("All accounts") search mixes results from every account; each
// row must name the account it belongs to with a readable chip, not only the
// 3px colour stripe the plain unified inbox uses.

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
  subject: 'Quarterly invoice',
  sender: 'Billing',
  senderEmail: 'billing@example.com',
  recipients: ['work@example.com'],
  cc: [],
  body: 'body',
  snippet: 'snippet',
  timestamp: 1_700_000_000,
  isRead: true,
  triageStatus: null,
  category: 'primary',
  mailbox: 'inbox',
  isSent: false,
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

describe('EmailRow account chip', () => {
  it.each([
    ['default', false],
    ['compact', true],
  ])('names the account in the %s row when the badge asks for a chip', (_layout, compact) => {
    act(() => {
      root.render(
        <EmailRow
          email={email}
          isSelected={false}
          onClick={() => {}}
          compact={compact}
          accountBadge={{ colorClass: 'bg-blue-500', label: 'work@example.com', chip: true }}
        />,
      );
    });
    expect(container.querySelector('[data-testid="account-chip"]')?.textContent).toBe('work@example.com');
  });

  it('keeps the plain stripe only when no chip is requested', () => {
    act(() => {
      root.render(
        <EmailRow
          email={email}
          isSelected={false}
          onClick={() => {}}
          accountBadge={{ colorClass: 'bg-blue-500', label: 'work@example.com', chip: false }}
        />,
      );
    });
    expect(container.querySelector('[data-testid="account-chip"]')).toBeNull();
  });
});
