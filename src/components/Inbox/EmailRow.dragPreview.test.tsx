// Dragging an email row toward a sidebar folder shows a preview card under
// the pointer (sender + subject) instead of nothing, in both row layouts.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { EMAIL_DRAG_MIME } from '@/lib/emailDrag';
import { useAccountStore } from '@/stores/accountStore';
import type { Account, Email } from '@/types';
import { EmailRow } from './EmailRow';

const email: Email = {
  id: 'email-1',
  accountId: 'acct-1',
  threadId: 'thread-1',
  messageId: null,
  subject: 'Invoice 42',
  sender: 'Alice Martin',
  senderEmail: 'alice@example.com',
  recipients: ['me@example.com'],
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
  // Move-to-folder (and so dragging) is offered for IMAP inbox messages.
  useAccountStore.setState({
    accounts: [{ id: 'acct-1', email: 'me@example.com', provider: 'imap' } as Account],
  } as never);
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function fakeDataTransfer() {
  const data: Record<string, string> = {};
  return {
    data,
    setData: (type: string, value: string) => {
      data[type] = value;
    },
    setDragImage: vi.fn(),
    effectAllowed: 'none',
  };
}

describe('EmailRow drag preview', () => {
  it.each([
    ['default', false],
    ['compact', true],
  ])('sends the payload and a sender/subject preview (%s row)', (_layout, compact) => {
    act(() => {
      root.render(<EmailRow email={email} isSelected={false} onClick={() => {}} compact={compact} />);
    });
    const row = container.querySelector<HTMLElement>('[draggable="true"]');
    if (!row) throw new Error('row is not draggable');

    const dt = fakeDataTransfer();
    const event = new Event('dragstart', { bubbles: true }) as Event & { dataTransfer: unknown };
    event.dataTransfer = dt;
    act(() => {
      row.dispatchEvent(event);
    });

    expect(JSON.parse(dt.data[EMAIL_DRAG_MIME])).toMatchObject({ emailId: 'email-1', mailbox: 'inbox' });
    expect(dt.setDragImage).toHaveBeenCalledTimes(1);
    const card = dt.setDragImage.mock.calls[0][0] as HTMLElement;
    expect(card.textContent).toContain('Alice Martin');
    expect(card.textContent).toContain('Invoice 42');
  });
});
