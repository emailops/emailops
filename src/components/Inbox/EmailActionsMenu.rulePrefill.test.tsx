// "Create attachment rule" from a row in All accounts must create the rule in
// that email's account, so the prefill carries the account.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));
vi.mock('@/lib/api', () => ({}));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}), emit: vi.fn(async () => {}) }));

import type { Email } from '@/types';
import { EmailActionsMenu, type RulePrefill } from './EmailActionsMenu';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const sentInvoice = {
  id: 'e1',
  accountId: 'acc-2',
  threadId: 't1',
  mailbox: 'sent',
  isRead: true,
  isStarred: false,
  isSent: true,
  subject: 'Invoice 7',
  sender: 'Me',
  senderEmail: 'b@example.com',
  recipients: ['client@example.com'],
  timestamp: 1_700_000_000,
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

function clickItem(label: string) {
  // The open menu is portalled to the document body.
  const item = Array.from(document.querySelectorAll('button')).find((b) => b.textContent?.includes(label));
  if (!item) throw new Error(`menu item "${label}" not found`);
  act(() => item.click());
}

describe('EmailActionsMenu rule prefill', () => {
  it.each([
    ['inbox:emailRow.createAttachmentRule', 'onCreateAttachmentRule'],
    ['inbox:emailRow.createClassificationRule', 'onCreateClassificationRule'],
  ] as const)('%s carries the account of the email', (label, prop) => {
    const onRule = vi.fn<(prefill: RulePrefill) => void>();
    act(() => root.render(<EmailActionsMenu email={sentInvoice} onStatus={vi.fn()} {...{ [prop]: onRule }} />));

    act(() => container.querySelector<HTMLButtonElement>('[aria-label="inbox:emailRow.moreActions"]')?.click());
    clickItem(label);

    expect(onRule).toHaveBeenCalledWith(expect.objectContaining({ accountId: 'acc-2', senderEmail: 'b@example.com' }));
  });
});
