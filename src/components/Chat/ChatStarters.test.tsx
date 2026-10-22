// The suggested requests an empty conversation offers: fixed ones at once,
// then the mailbox-derived ones from the backend.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, string>) => (opts ? `${key}|${Object.values(opts).join(',')}` : key),
  }),
}));

vi.mock('@/lib/api', () => ({ getChatStarters: vi.fn() }));

import { getChatStarters } from '@/lib/api';
import { useLogStore } from '@/stores/logStore';
import type { ChatStarter } from '@/types';
import { ChatStarters } from './ChatStarters';

const fromMailbox: ChatStarter[] = [
  { kind: 'today', action: 'send' },
  { kind: 'sender', action: 'send', senderName: 'Ana Ruiz' },
  { kind: 'thread', action: 'send', subject: 'Kickoff agenda' },
  { kind: 'thisWeek', action: 'send' },
  { kind: 'draft', action: 'prefill' },
];

let container: HTMLDivElement;
let root: Root;
const onSend = vi.fn();
const onPrefill = vi.fn();
const onSomethingElse = vi.fn();

beforeEach(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  vi.mocked(getChatStarters).mockReset();
  onSend.mockReset();
  onPrefill.mockReset();
  onSomethingElse.mockReset();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function render() {
  await act(async () => {
    root.render(
      <ChatStarters
        accountId="acct"
        disabled={false}
        onSend={onSend}
        onPrefill={onPrefill}
        onSomethingElse={onSomethingElse}
      />,
    );
  });
}

const rows = () => [...container.querySelectorAll<HTMLButtonElement>('[data-testid="chat-starter"]')];
const labels = () => rows().map((b) => b.querySelector('span:last-child')?.textContent);

describe('ChatStarters', () => {
  it('lists the mailbox starters the backend returns, then "something else"', async () => {
    vi.mocked(getChatStarters).mockResolvedValue(fromMailbox);
    await render();
    expect(getChatStarters).toHaveBeenCalledWith('acct');
    expect(labels()).toEqual([
      'chat:shortcuts.today.label',
      'chat:starters.sender.label|Ana Ruiz',
      'chat:starters.thread.label|Kickoff agenda',
      'chat:shortcuts.thisWeek.label',
      'chat:shortcuts.draft.label',
    ]);
    expect(container.querySelector('[data-testid="chat-starter-other"] span:last-child')?.textContent).toBe(
      'chat:starters.somethingElse',
    );
  });

  it('keeps the fixed starters when the backend fails, and logs it', async () => {
    vi.mocked(getChatStarters).mockRejectedValue(new Error('db locked'));
    useLogStore.setState({ entries: [] });
    await render();
    expect(labels()).toEqual([
      'chat:shortcuts.today.label',
      'chat:shortcuts.thisWeek.label',
      'chat:shortcuts.draft.label',
    ]);
    expect(useLogStore.getState().entries.some((l) => l.level === 'error')).toBe(true);
  });

  it('sends a mailbox starter with its own prompt', async () => {
    vi.mocked(getChatStarters).mockResolvedValue(fromMailbox);
    await render();
    await act(async () => rows()[1].click());
    expect(onSend).toHaveBeenCalledWith('chat:starters.sender.prompt|Ana Ruiz');
    expect(onPrefill).not.toHaveBeenCalled();
  });

  it('puts the draft stem in the input instead of sending it', async () => {
    vi.mocked(getChatStarters).mockResolvedValue(fromMailbox);
    await render();
    await act(async () => rows()[4].click());
    expect(onPrefill).toHaveBeenCalledWith('chat:shortcuts.draft.prompt');
    expect(onSend).not.toHaveBeenCalled();
  });

  it('"something else" hands over to the input', async () => {
    vi.mocked(getChatStarters).mockResolvedValue(fromMailbox);
    await render();
    await act(async () => container.querySelector<HTMLButtonElement>('[data-testid="chat-starter-other"]')?.click());
    expect(onSomethingElse).toHaveBeenCalled();
  });

  it('moves between rows with the arrow keys', async () => {
    vi.mocked(getChatStarters).mockResolvedValue(fromMailbox);
    await render();
    rows()[0].focus();
    const press = (key: string) =>
      act(async () => {
        document.activeElement?.dispatchEvent(new KeyboardEvent('keydown', { key, bubbles: true }));
      });
    await press('ArrowDown');
    expect(document.activeElement).toBe(rows()[1]);
    await press('ArrowUp');
    await press('ArrowUp');
    expect(document.activeElement).toBe(rows()[0]);
  });
});
