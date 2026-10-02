// "Confirm junk" waits for the provider before leaving the conversation. If the
// user opened another conversation meanwhile, the late answer must not close
// or replace it.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

let resolveReport: (filed: boolean) => void = () => {};
vi.mock('@/lib/api', () => ({
  reportJunkToProvider: vi.fn(
    () =>
      new Promise<boolean>((resolve) => {
        resolveReport = resolve;
      }),
  ),
  getJunkVerdicts: vi.fn(async () => []),
  getThread: vi.fn(async () => []),
  getEmailBody: vi.fn(async () => ''),
  markAsRead: vi.fn(async () => {}),
}));

import { initI18n } from '@/i18n';
import { useAutoAdvanceStore } from '@/stores/autoAdvanceStore';
import { useEmailStore } from '@/stores/emailStore';
import { useJunkStore } from '@/stores/junkStore';
import { useShortcutStore } from '@/stores/shortcutStore';
import type { Email, JunkVerdict } from '@/types';
import { JunkBanner } from './JunkBanner';

const row = (id: string) =>
  ({ id, accountId: 'a1', threadId: `t-${id}`, mailbox: 'inbox', isRead: true, subject: id, timestamp: 1 }) as Email;
const LIST = [row('e1'), row('e2'), row('e3')];

let container: HTMLDivElement;
let root: Root;

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(async () => {
  useAutoAdvanceStore.setState({ mode: 'next' });
  useEmailStore.setState({ emails: LIST, tabs: [], activeTabId: null });
  useShortcutStore.setState({ listEmails: LIST });
  useJunkStore.setState({
    loadVerdicts: async () => {},
    verdictsByEmail: {
      e2: {
        emailId: 'e2',
        band: 'junk',
        primaryKind: 'spam',
        reasons: [],
        userOverride: null,
      } as unknown as JunkVerdict,
    },
  });
  await useEmailStore.getState().selectEmail(LIST[1]);
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  await act(async () => root.render(<JunkBanner emailId="e2" accountId="a1" />));
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

const confirmButton = () =>
  Array.from(container.querySelectorAll('button')).find((b) => b.textContent === 'Confirm junk') as HTMLButtonElement;

describe('JunkBanner confirm', () => {
  it('opens the next conversation once the provider answers', async () => {
    act(() => confirmButton().click());
    await act(async () => resolveReport(true));
    expect(useEmailStore.getState().selectedEmail?.id).toBe('e3');
  });

  it('leaves alone a conversation the user opened while it waited', async () => {
    act(() => confirmButton().click());
    await act(async () => {
      await useEmailStore.getState().selectEmail(LIST[0]);
    });
    await act(async () => resolveReport(true));
    expect(useEmailStore.getState().selectedEmail?.id).toBe('e1');
  });
});
