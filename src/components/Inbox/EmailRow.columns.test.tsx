// The optional Size and Has attachment cells of the single-line row. They
// describe the listed message; a size the provider never reported reads as
// unknown, never as zero.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it } from 'vitest';
import { initI18n } from '@/i18n';
import type { ListColumns } from '@/lib/listColumns';
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
  isStarred: false,
};

let container: HTMLDivElement;
let root: Root;

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function render(row: Email, columns: ListColumns, compact = true) {
  act(() => {
    root.render(<EmailRow email={row} isSelected={false} onClick={() => {}} compact={compact} columns={columns} />);
  });
}

const cell = (name: string) => container.querySelector(`[data-testid="row-${name}"]`);
const both: ListColumns = { size: true, attachment: true };

describe('EmailRow optional columns', () => {
  it('shows the size of the listed message', () => {
    render({ ...email, sizeBytes: 48_213 }, both);
    expect(cell('size')?.textContent).toBe('47.1 KB');
  });

  it('shows an unknown size as unknown', () => {
    render({ ...email, sizeBytes: null }, both);
    expect(cell('size')?.textContent).toBe('—');
    expect(cell('size')?.getAttribute('title')).toBe('Size unknown');
  });

  it('marks a message with attachments', () => {
    render({ ...email, hasAttachments: true }, both);
    expect(cell('attachment')?.querySelector('svg')).not.toBeNull();
    expect(cell('attachment')?.getAttribute('title')).toBe('Has attachment');

    render({ ...email, hasAttachments: false }, both);
    expect(cell('attachment')?.querySelector('svg')).toBeNull();
  });

  it('puts the attachment cell before the size cell', () => {
    render({ ...email, sizeBytes: 10, hasAttachments: true }, both);
    const attachment = cell('attachment');
    const size = cell('size');
    expect(
      attachment && size && attachment.compareDocumentPosition(size) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it('leaves out the columns that are hidden', () => {
    render({ ...email, sizeBytes: 10, hasAttachments: true }, { size: false, attachment: false });
    expect(cell('size')).toBeNull();
    expect(cell('attachment')).toBeNull();
  });

  it('keeps the two-line row of the split layout as it was', () => {
    render({ ...email, sizeBytes: 10, hasAttachments: true }, both, false);
    expect(cell('size')).toBeNull();
    expect(cell('attachment')).toBeNull();
  });
});
