// Dragging an email row toward a sidebar folder shows a preview card under
// the pointer (sender + subject) instead of nothing, in both row layouts.
// A checked row carries the whole multi-selection, with a count badge; an
// unchecked row carries only itself, and so does a checked row whose
// selection the bulk toolbar could not move together.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { EMAIL_DRAG_MIME } from '@/lib/emailDrag';
import { useAccountStore } from '@/stores/accountStore';
import { useEmailStore } from '@/stores/emailStore';
import { useFolderStore } from '@/stores/folderStore';
import { useSelectionStore } from '@/stores/selectionStore';
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
  isStarred: false,
};

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  // Move-to-folder (and so dragging) is offered for IMAP inbox messages.
  useAccountStore.setState({
    accounts: [
      { id: 'acct-1', email: 'me@example.com', provider: 'imap' } as Account,
      { id: 'acct-2', email: 'other@example.com', provider: 'imap' } as Account,
    ],
  } as never);
  useFolderStore.setState({ folders: [], accountId: 'acct-1' });
  useEmailStore.setState({
    emails: [
      email,
      { ...email, id: 'email-2' },
      { ...email, id: 'email-3' },
      { ...email, id: 'email-4', accountId: 'acct-2' },
    ],
  });
  useSelectionStore.getState().clear();
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

/** Render one row and start dragging it; returns what the drag carried. */
function dragRow(compact: boolean, isChecked = false) {
  act(() => {
    root.render(
      <EmailRow email={email} isSelected={false} onClick={() => {}} compact={compact} isChecked={isChecked} />,
    );
  });
  const row = container.querySelector<HTMLElement>('[draggable="true"]');
  if (!row) throw new Error('row is not draggable');

  const dt = fakeDataTransfer();
  const event = new Event('dragstart', { bubbles: true }) as Event & { dataTransfer: unknown };
  event.dataTransfer = dt;
  act(() => {
    row.dispatchEvent(event);
  });
  expect(dt.setDragImage).toHaveBeenCalledTimes(1);
  return {
    payload: JSON.parse(dt.data[EMAIL_DRAG_MIME]),
    card: dt.setDragImage.mock.calls[0][0] as HTMLElement,
  };
}

describe('EmailRow drag preview', () => {
  it.each([
    ['default', false],
    ['compact', true],
  ])('sends the payload and a sender/subject preview (%s row)', (_layout, compact) => {
    const { payload, card } = dragRow(compact);

    expect(payload).toMatchObject({ emailIds: ['email-1'], mailbox: 'inbox' });
    expect(card.textContent).toContain('Alice Martin');
    expect(card.textContent).toContain('Invoice 42');
    expect(card.querySelector('[data-role="count"]')).toBeNull();
  });

  it.each([
    ['default', false],
    ['compact', true],
  ])('drags every checked email, with a count badge, when the row is checked (%s row)', (_layout, compact) => {
    for (const id of ['email-2', 'email-1', 'email-3']) useSelectionStore.getState().toggle(id);

    const { payload, card } = dragRow(compact, true);

    expect(payload.emailIds).toEqual(['email-1', 'email-2', 'email-3']);
    expect(card.querySelector('[data-role="count"]')?.textContent).toBe('3');
  });

  it('drags only its own email when the row is not checked, whatever else is', () => {
    for (const id of ['email-2', 'email-3']) useSelectionStore.getState().toggle(id);

    const { payload, card } = dragRow(false);

    expect(payload.emailIds).toEqual(['email-1']);
    expect(card.querySelector('[data-role="count"]')).toBeNull();
  });

  it('drags only its own email when the checked rows span two accounts', () => {
    for (const id of ['email-1', 'email-4']) useSelectionStore.getState().toggle(id);

    const { payload, card } = dragRow(false, true);

    expect(payload.emailIds).toEqual(['email-1']);
    expect(card.querySelector('[data-role="count"]')).toBeNull();
  });
});
