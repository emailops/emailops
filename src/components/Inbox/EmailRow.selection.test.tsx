// Multi-select on a row: the checkbox and Cmd/Ctrl/Shift-click select without
// opening the conversation; a plain click still opens it.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Email } from '@/types';

vi.mock('@/lib/api', () => ({
  applyThreadAction: vi.fn(async () => ({ failed: [] })),
}));

import { EmailRow } from './EmailRow';

const email = {
  id: 'email-1',
  accountId: 'acct-1',
  threadId: 'thread-1',
  subject: 'Quarterly invoice',
  sender: 'Billing',
  senderEmail: 'billing@example.com',
  recipients: [],
  cc: [],
  body: '',
  snippet: 'snippet',
  timestamp: 1_700_000_000,
  isRead: true,
  triageStatus: null,
  category: 'primary',
  mailbox: 'inbox',
  isSent: false,
  isStarred: false,
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

function render(compact: boolean, isChecked = false) {
  const onClick = vi.fn();
  const onCheck = vi.fn();
  act(() => {
    root.render(
      <EmailRow
        email={email}
        isSelected={false}
        onClick={onClick}
        compact={compact}
        isChecked={isChecked}
        selectionActive={isChecked}
        onCheck={onCheck}
      />,
    );
  });
  const checkbox = container.querySelector<HTMLInputElement>('[data-testid="row-select"]');
  const row = container.querySelector<HTMLElement>('[role="button"]');
  if (!checkbox || !row) throw new Error('row not rendered');
  return { onClick, onCheck, checkbox, row };
}

describe.each([false, true])('EmailRow selection (compact: %s)', (compact) => {
  it('the checkbox toggles the row without opening it', () => {
    const { onClick, onCheck, checkbox } = render(compact);
    act(() => checkbox.dispatchEvent(new MouseEvent('click', { bubbles: true })));
    expect(onCheck).toHaveBeenCalledWith({ range: false });
    expect(onClick).not.toHaveBeenCalled();
  });

  it('shift-click on the checkbox extends a range', () => {
    const { onCheck, checkbox } = render(compact);
    act(() => checkbox.dispatchEvent(new MouseEvent('click', { bubbles: true, shiftKey: true })));
    expect(onCheck).toHaveBeenCalledWith({ range: true });
  });

  it.each([{ metaKey: true }, { ctrlKey: true }])('modifier-click %o on the row toggles it', (mods) => {
    const { onClick, onCheck, row } = render(compact);
    act(() => row.dispatchEvent(new MouseEvent('click', { bubbles: true, ...mods })));
    expect(onCheck).toHaveBeenCalledWith({ range: false });
    expect(onClick).not.toHaveBeenCalled();
  });

  it('shift-click on the row selects a range', () => {
    const { onClick, onCheck, row } = render(compact);
    act(() => row.dispatchEvent(new MouseEvent('click', { bubbles: true, shiftKey: true })));
    expect(onCheck).toHaveBeenCalledWith({ range: true });
    expect(onClick).not.toHaveBeenCalled();
  });

  it('a plain click opens the conversation', () => {
    const { onClick, onCheck, row } = render(compact);
    act(() => row.dispatchEvent(new MouseEvent('click', { bubbles: true })));
    expect(onClick).toHaveBeenCalled();
    expect(onCheck).not.toHaveBeenCalled();
  });

  it('a checked row shows its checkbox ticked', () => {
    const { checkbox } = render(compact, true);
    expect(checkbox.checked).toBe(true);
  });
});
