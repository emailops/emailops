// The row ⋮ menu once printed the sender's address beside "Add sender as
// smart filter" and "Block sender": in a 224px menu the address squeezed the
// labels onto two lines and was itself cut to a few characters. The address
// is now shown once, as the header of the sender items, and labels never wrap.

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
  subject: 'Quarterly report',
  sender: 'Ada Example',
  senderEmail: 'ada.lovelace-longname@example.com',
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
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function openMenu(): HTMLElement {
  act(() => {
    root.render(
      <EmailRow
        email={email}
        isSelected={false}
        onClick={() => {}}
        onAddSenderFilter={() => {}}
        onHideSenderFromFilters={() => {}}
      />,
    );
  });
  const button = container.querySelector<HTMLButtonElement>('button:not([data-testid="star-toggle"])');
  if (!button) throw new Error('kebab button not rendered');
  act(() => {
    button.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  });
  const dropdown = document.body.querySelector<HTMLElement>('div.fixed');
  if (!dropdown) throw new Error('dropdown not rendered');
  return dropdown;
}

describe('EmailRow menu layout', () => {
  it('shows the sender address once, as the header of the sender items', () => {
    const dropdown = openMenu();
    const occurrences = dropdown.textContent?.split(email.senderEmail).length ?? 1;
    expect(occurrences - 1).toBe(1);
    const header = dropdown.querySelector<HTMLElement>('[data-testid="menu-sender"]');
    expect(header?.textContent).toBe(email.senderEmail);
    expect(header?.title).toBe(email.senderEmail);
    expect(header?.className).toContain('truncate');
  });

  it('keeps every item label on one line', () => {
    const dropdown = openMenu();
    const items = [...dropdown.querySelectorAll<HTMLButtonElement>('button')];
    expect(items.length).toBeGreaterThan(5);
    for (const item of items) expect(item.className).toContain('whitespace-nowrap');
  });
});
