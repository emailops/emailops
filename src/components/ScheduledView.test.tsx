// The Scheduled view: waiting and failed outbox rows with their actions, and
// the note that the app must be open for them to go out.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({
  listOutbox: vi.fn(async () => []),
  cancelOutboxMessage: vi.fn(),
  sendOutboxMessageNow: vi.fn(async () => {}),
}));

import { initI18n } from '@/i18n';
import type { OutboxEntry } from '@/lib/api';
import * as api from '@/lib/api';
import { useOutboxStore } from '@/stores/outboxStore';
import { ScheduledView } from './ScheduledView';

let container: HTMLDivElement;
let root: Root;

const entry = (id: string, extra: Partial<OutboxEntry> = {}): OutboxEntry => ({
  id,
  accountId: 'acc',
  kind: 'new',
  replyToEmailId: null,
  origin: 'scheduled',
  toAddresses: ['ana@example.com'],
  ccAddresses: [],
  subject: `Subject ${id}`,
  attachmentCount: 0,
  sendAt: 1_900_000_000,
  status: 'scheduled',
  attempts: 0,
  lastError: null,
  failureKind: null,
  createdAt: 1_800_000_000,
  ...extra,
});

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  vi.clearAllMocks();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

const rows = () => Array.from(container.querySelectorAll<HTMLElement>('[data-testid="scheduled-row"]'));

async function render(entries: OutboxEntry[]) {
  vi.mocked(api.listOutbox).mockResolvedValue(entries);
  await act(async () => {
    root.render(<ScheduledView accountId="acc" accounts={[]} />);
  });
}

describe('ScheduledView', () => {
  it('lists the account’s waiting messages and says the app must be open', async () => {
    await render([entry('a'), entry('b', { attachmentCount: 2 })]);
    expect(api.listOutbox).toHaveBeenCalledWith('acc');
    expect(rows().map((r) => r.dataset.outboxId)).toEqual(['a', 'b']);
    expect(container.textContent).toContain('ana@example.com');
    expect(container.textContent).toContain('2 attachments');
    expect(container.querySelector('[data-testid="scheduled-app-open-note"]')?.textContent).toMatch(/open/);
  });

  it('shows why a message failed and offers Retry', async () => {
    await render([entry('f', { status: 'failed', failureKind: 'interrupted' })]);
    expect(container.querySelector('[data-testid="scheduled-failed"]')?.textContent).toMatch(/check Sent/i);
    const retry = container.querySelector<HTMLButtonElement>('[data-testid="scheduled-send-now"]');
    expect(retry?.textContent).toBe('Retry');
    await act(async () => retry?.click());
    expect(api.sendOutboxMessageNow).toHaveBeenCalledWith('acc', 'f');
  });

  it('Edit hands the message back to a composer', async () => {
    const restore = vi.fn();
    useOutboxStore.getState().setRestoreHandler(restore);
    const message = {
      accountId: 'acc',
      to: ['ana@example.com'],
      cc: [],
      subject: 'S',
      body: 'b',
      inlineImages: [],
      attachments: [],
    };
    vi.mocked(api.cancelOutboxMessage).mockResolvedValue(message);
    await render([entry('a')]);
    await act(async () => container.querySelector<HTMLButtonElement>('[data-testid="scheduled-edit"]')?.click());
    expect(api.cancelOutboxMessage).toHaveBeenCalledWith('acc', 'a');
    expect(restore).toHaveBeenCalledWith(message);
    expect(rows()).toHaveLength(0);
  });

  it('says so when nothing is scheduled', async () => {
    await render([]);
    expect(container.textContent).toContain('No scheduled messages');
  });
});
