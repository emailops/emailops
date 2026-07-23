// Regression test: opening the kebab menu on a row near the bottom of the
// viewport must not clip the dropdown below the window — it should flip above
// the button. jsdom does no layout, so getBoundingClientRect is stubbed: the
// prototype stub stands in for the rendered dropdown's height, and the button
// instance gets its own near-bottom anchor rect.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { Email } from '@/types';
import { EmailRow } from './EmailRow';

const MENU_HEIGHT = 400;

const email: Email = {
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
};

let container: HTMLDivElement;
let root: Root;
const realGetRect = Element.prototype.getBoundingClientRect;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  // Every element (in particular the portaled dropdown) measures MENU_HEIGHT
  // tall; the anchor button below overrides this with its own rect.
  Element.prototype.getBoundingClientRect = () =>
    ({ top: 0, bottom: MENU_HEIGHT, left: 0, right: 224, width: 224, height: MENU_HEIGHT }) as DOMRect;
});

afterEach(() => {
  Element.prototype.getBoundingClientRect = realGetRect;
  act(() => root.unmount());
  container.remove();
});

function openMenuWithAnchor(anchor: { top: number; bottom: number }) {
  act(() => {
    root.render(<EmailRow email={email} isSelected={false} onClick={() => {}} />);
  });
  const button = container.querySelector('button');
  if (!button) throw new Error('kebab button not rendered');
  button.getBoundingClientRect = () =>
    ({ ...anchor, left: 980, right: 1000, width: 20, height: anchor.bottom - anchor.top }) as DOMRect;
  act(() => {
    button.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  });
  const dropdown = document.body.querySelector<HTMLElement>('div.fixed');
  if (!dropdown) throw new Error('dropdown not rendered');
  return dropdown;
}

describe('EmailRow kebab menu position', () => {
  it('flips the menu above the button when the row is near the viewport bottom', () => {
    // jsdom viewport is 768px tall; a button at 700..730 leaves no room below.
    const dropdown = openMenuWithAnchor({ top: 700, bottom: 730 });
    expect(dropdown.style.top).toBe(`${700 - 4 - MENU_HEIGHT}px`);
  });

  it('keeps the menu below the button when there is room', () => {
    const dropdown = openMenuWithAnchor({ top: 100, bottom: 130 });
    expect(dropdown.style.top).toBe('134px');
  });
});
