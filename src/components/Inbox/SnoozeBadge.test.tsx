// The snooze badge on a list row: "Snoozed until …" in the Snoozed view, and
// a "Snoozed" marker on a conversation back from snooze until it is read.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it } from 'vitest';
import { initI18n } from '@/i18n';
import type { ThreadSnooze } from '@/lib/api';
import { snoozeMap, useEmailStore } from '@/stores/emailStore';
import type { Email } from '@/types';
import { SnoozeBadge } from './SnoozeBadge';

let container: HTMLDivElement;
let root: Root;

const row = (extra: Partial<Email> = {}) =>
  ({ id: 'a', accountId: 'acc', threadId: 't1', isRead: true, mailbox: 'inbox', ...extra }) as Email;
const record = (extra: Partial<ThreadSnooze> = {}): ThreadSnooze => ({
  accountId: 'acc',
  threadId: 't1',
  snoozedUntil: 1_900_000_000,
  createdAt: 1,
  wokeAt: null,
  ...extra,
});

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

function render(email: Email, records: ThreadSnooze[], listScope: 'inbox' | 'snoozed') {
  useEmailStore.setState({ snoozes: snoozeMap(records), listScope });
  act(() => root.render(<SnoozeBadge email={email} />));
  return container.querySelector<HTMLElement>('[data-testid="snooze-badge"]');
}

describe('SnoozeBadge', () => {
  it('shows the wake time in the Snoozed view', () => {
    expect(render(row(), [record()], 'snoozed')?.textContent).toMatch(/^Snoozed until /);
  });

  it('marks a conversation back from snooze while it is unread', () => {
    const woken = [record({ wokeAt: 1_800_000_000 })];
    expect(render(row({ isRead: false }), woken, 'inbox')?.textContent).toBe('Snoozed');
    expect(render(row({ isRead: true }), woken, 'inbox')).toBeNull();
  });

  it('shows nothing for a conversation without a snooze', () => {
    expect(render(row({ isRead: false }), [], 'inbox')).toBeNull();
  });
});
