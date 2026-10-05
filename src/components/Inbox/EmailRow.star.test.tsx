// The row's star toggles the conversation's star in place: it must not open
// the row, and it shows the thread's starred state.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Email } from '@/types';

vi.mock('@/lib/api', () => ({
  applyThreadAction: vi.fn(async () => ({ failed: [] })),
}));

import * as api from '@/lib/api';
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

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  vi.clearAllMocks();
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function render(row: Email, onClick: () => void, compact = false) {
  act(() => {
    root.render(<EmailRow email={row} isSelected={false} onClick={onClick} compact={compact} />);
  });
  const star = container.querySelector<HTMLButtonElement>('[data-testid="star-toggle"]');
  if (!star) throw new Error('star toggle not rendered');
  return star;
}

describe('EmailRow star toggle', () => {
  it.each([false, true])('stars the conversation without opening the row (compact: %s)', async (compact) => {
    const onClick = vi.fn();
    const star = render(email, onClick, compact);

    await act(async () => {
      star.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    });

    expect(onClick).not.toHaveBeenCalled();
    expect(api.applyThreadAction).toHaveBeenCalledWith([{ accountId: 'acct-1', threadId: 'thread-1' }], 'star');
  });

  it('shows a starred conversation as pressed and unstars it', async () => {
    const star = render({ ...email, isStarred: true }, () => {});
    expect(star.getAttribute('aria-pressed')).toBe('true');

    await act(async () => {
      star.dispatchEvent(new MouseEvent('click', { bubbles: true }));
    });

    expect(api.applyThreadAction).toHaveBeenCalledWith([{ accountId: 'acct-1', threadId: 'thread-1' }], 'unstar');
  });
});
