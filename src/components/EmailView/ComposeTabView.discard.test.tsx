// Discard is the one way out of a compose tab that gets rid of the message:
// it asks first and removes the draft, which the autosave has already written.
// Closing the tab is what keeps a draft. A failed send is pinned outside the
// scrolling body, so it is seen without scrolling past a long message.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('@/components/shared/RichTextEditor', () => ({
  RichTextEditor: ({ value, onChange }: { value: string; onChange: (html: string) => void }) => (
    <textarea data-testid="body" value={value} onChange={(e) => onChange(e.target.value)} />
  ),
}));
vi.mock('@/components/shared/TranslateComposeControl', () => ({ TranslateComposeControl: () => null }));
vi.mock('@/components/shared/Select', () => ({ Select: () => null }));
vi.mock('@/lib/api', () => ({
  getPref: vi.fn(async (key: string) => (key === 'compose.undo_send_delay_secs' ? '0' : null)),
  autocompleteRecipients: vi.fn(async () => []),
  saveDraft: vi.fn(async () => ({ id: 'd1' })),
  deleteDraft: vi.fn(async () => {}),
  sendNewEmail: vi.fn(async () => {}),
  generateNewDraft: vi.fn(async () => 'req'),
}));

import * as api from '@/lib/api';
import type { ComposeTab } from '@/stores/emailStore';
import { useLogStore } from '@/stores/logStore';
import { useOutboxStore } from '@/stores/outboxStore';
import type { Account } from '@/types';
import { ComposeTabView } from './ComposeTabView';

const account = { id: 'a1', email: 'me@example.com', name: 'Me', enabled: true } as Account;
const tab: ComposeTab = {
  type: 'compose',
  id: 't1',
  accountId: 'a1',
  toAddresses: ['bob@example.com'],
  subject: 'Feedback',
  bodyHtml: '<p>Hi there</p>',
};

let container: HTMLDivElement;
let root: Root;
const onClose = vi.fn();

beforeEach(() => {
  vi.useFakeTimers();
  vi.clearAllMocks();
  useOutboxStore.setState({ undoDelaySecs: null });
  useLogStore.setState({ entries: [] });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

async function mount(over: Partial<ComposeTab> = {}) {
  await act(async () => {
    root.render(<ComposeTabView tab={{ ...tab, ...over }} accounts={[account]} onClose={onClose} />);
  });
}

async function edit(text: string) {
  const area = container.querySelector<HTMLTextAreaElement>('[data-testid="body"]');
  const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set;
  await act(async () => {
    setter?.call(area, text);
    area?.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

function button(label: string, scope: ParentNode = container): HTMLButtonElement {
  const found = [...scope.querySelectorAll('button')].find((b) => b.textContent === label);
  if (!found) throw new Error(`${label} not rendered`);
  return found;
}

const dialog = () => container.querySelector<HTMLElement>('[data-testid="modal-overlay"]');
const clickDiscard = () => act(async () => button('compose:discardDraft').click());
const confirmDiscard = async () => {
  const confirm = dialog();
  if (!confirm) throw new Error('the confirmation did not open');
  await act(async () => button('compose:discardDraft', confirm).click());
  await act(async () => {
    await vi.runAllTimersAsync();
  });
};

describe('Discard in a compose tab', () => {
  it('asks first, and Cancel goes back to the message', async () => {
    await mount({ draftId: 'd9' });

    await clickDiscard();

    expect(dialog()?.textContent).toContain('compose:discardConfirm');
    expect(api.deleteDraft).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();

    await act(async () => button('common:actions.cancel', dialog() as HTMLElement).click());

    expect(dialog()).toBeNull();
    expect(api.deleteDraft).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });

  it('removes the draft this tab was opened on', async () => {
    await mount({ draftId: 'd9' });

    await clickDiscard();
    await confirmDiscard();

    expect(api.deleteDraft).toHaveBeenCalledWith('d9', 'a1');
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('removes the draft the autosave wrote', async () => {
    await mount();
    await edit('<p>Second thoughts</p>');
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1000);
    });

    await clickDiscard();
    await confirmDiscard();

    expect(api.deleteDraft).toHaveBeenCalledWith('d1', 'a1');
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('does not let a save still waiting bring the draft back', async () => {
    await mount({ draftId: 'd9' });

    await clickDiscard();
    await edit('<p>Typed behind the dialog</p>');
    await confirmDiscard();

    expect(api.deleteDraft).toHaveBeenCalledTimes(1);
    expect(api.saveDraft).not.toHaveBeenCalled();
  });

  it('closes without a delete when nothing was saved', async () => {
    await mount();

    await clickDiscard();
    await confirmDiscard();

    expect(api.deleteDraft).not.toHaveBeenCalled();
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('logs a delete that fails, and still closes', async () => {
    vi.mocked(api.deleteDraft).mockRejectedValueOnce(new Error('offline'));
    await mount({ draftId: 'd9' });

    await clickDiscard();
    await confirmDiscard();

    expect(useLogStore.getState().entries.some((e) => e.level === 'error')).toBe(true);
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});

describe('a failed send in a compose tab', () => {
  it('shows outside the scrolling body', async () => {
    vi.mocked(api.sendNewEmail).mockRejectedValueOnce(new Error('Authentication required'));
    await mount();

    await act(async () => container.querySelector<HTMLButtonElement>('[data-testid="compose-tab-send"]')?.click());

    const alert = container.querySelector('[role="alert"]');
    expect(alert?.textContent).toContain('Authentication required');
    expect(alert?.closest('.overflow-y-auto')).toBeNull();
  });
});
