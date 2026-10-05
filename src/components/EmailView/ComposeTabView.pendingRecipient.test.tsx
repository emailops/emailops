// Regression: an address typed into the To box but never tokenised (no
// Enter/Tab, no suggestion picked) left Send disabled in the tab composer and
// was dropped from the autosaved draft, while the modal composer counted it.
// A valid pending address must count for Send, be included in the outgoing
// message and land in the saved draft.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('@/components/shared/RichTextEditor', () => ({
  RichTextEditor: ({ value }: { value: string }) => <div data-testid="body">{value}</div>,
}));
vi.mock('@/components/shared/TranslateComposeControl', () => ({ TranslateComposeControl: () => null }));
vi.mock('@/components/shared/Select', () => ({ Select: () => null }));
// Undo send off unless a test turns it on: these cases pin the direct send.
let undoDelay = '0';
vi.mock('@/lib/api', () => ({
  getPref: vi.fn(async (key: string) => (key === 'compose.undo_send_delay_secs' ? undoDelay : null)),
  queueOutgoingEmail: vi.fn(async () => ({ id: 'o1' })),
  sendReply: vi.fn(async () => {}),
  autocompleteRecipients: vi.fn(async () => []),
  saveDraft: vi.fn(async () => ({ id: 'd1' })),
  deleteDraft: vi.fn(async () => {}),
  sendNewEmail: vi.fn(async () => {}),
  sendDraft: vi.fn(async () => {}),
  generateNewDraft: vi.fn(async () => 'req'),
}));

import * as api from '@/lib/api';
import type { ComposeTab } from '@/stores/emailStore';
import { useOutboxStore } from '@/stores/outboxStore';
import type { Account } from '@/types';
import { ComposeTabView } from './ComposeTabView';

const account = {
  id: 'a1',
  provider: 'imap',
  email: 'me@example.com',
  name: 'Me',
  createdAt: 0,
  sortOrder: 0,
  enabled: true,
  syncFromTimestamp: null,
} as Account;

const tab: ComposeTab = {
  type: 'compose',
  id: 't1',
  accountId: 'a1',
  toAddresses: [],
  subject: 'Feedback',
  bodyHtml: '<p>Hi there</p>',
};

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  undoDelay = '0';
  useOutboxStore.setState({ undoDelaySecs: null });
  vi.useFakeTimers();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
  vi.clearAllMocks();
});

async function mount() {
  await act(async () => {
    root.render(<ComposeTabView tab={tab} accounts={[account]} onClose={() => {}} />);
  });
}

async function typeIntoTo(value: string) {
  const input = container.querySelector<HTMLInputElement>('#compose-tab-t1-to-input');
  if (!input) throw new Error('To input not rendered');
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
  await act(async () => {
    setter?.call(input, value);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

function sendButton(): HTMLButtonElement {
  const b = container.querySelector<HTMLButtonElement>('[data-testid="compose-tab-send"]');
  if (!b) throw new Error('Send button not rendered');
  return b;
}

describe('ComposeTabView with a recipient typed but not tokenised', () => {
  it('enables Send once a valid address sits in the To box', async () => {
    await mount();
    expect(sendButton().disabled).toBe(true);
    await typeIntoTo('bob@example.com');
    expect(sendButton().disabled).toBe(false);
  });

  it('sends to the pending address', async () => {
    await mount();
    await typeIntoTo('bob@example.com');
    await act(async () => {
      sendButton().click();
    });
    expect(vi.mocked(api.sendNewEmail)).toHaveBeenCalledTimes(1);
    expect(vi.mocked(api.sendNewEmail).mock.calls[0][1]).toEqual(['bob@example.com']);
  });

  it('autosaves the draft with the pending address', async () => {
    await mount();
    await typeIntoTo('bob@example.com');
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1000);
    });
    const saves = vi.mocked(api.saveDraft).mock.calls;
    expect(saves.length).toBeGreaterThan(0);
    expect(saves[saves.length - 1][0]).toMatchObject({ toAddresses: ['bob@example.com'] });
  });
});

describe('ComposeTabView opened from the compose modal', () => {
  it('starts with the files attached in the modal and sends them', async () => {
    const file = { filename: 'notes.txt', mimeType: 'text/plain', data: 'aGVsbG8=' };
    await act(async () => {
      root.render(
        <ComposeTabView
          tab={{ ...tab, toAddresses: ['bob@example.com'], fileAttachments: [file] }}
          accounts={[account]}
          onClose={() => {}}
        />,
      );
    });
    expect(container.textContent).toContain('notes.txt');

    await act(async () => {
      sendButton().click();
    });
    expect(vi.mocked(api.sendNewEmail).mock.calls[0][5]).toEqual([file]);
  });
});

describe('ComposeTabView with undo send on', () => {
  it('queues the message for the undo window, hands over its draft, and closes', async () => {
    undoDelay = '10';
    const onClose = vi.fn();
    await act(async () => {
      root.render(
        <ComposeTabView
          tab={{ ...tab, toAddresses: ['bob@example.com'], draftId: 'd9' }}
          accounts={[account]}
          onClose={onClose}
        />,
      );
    });
    await act(async () => {
      sendButton().click();
    });
    expect(api.sendNewEmail).not.toHaveBeenCalled();
    const [message, schedule, draftId] = vi.mocked(api.queueOutgoingEmail).mock.calls[0];
    expect(message).toMatchObject({
      accountId: 'a1',
      to: ['bob@example.com'],
      subject: 'Feedback',
      replyToEmailId: null,
    });
    expect(schedule).toEqual({ type: 'undo', delaySecs: 10 });
    expect(draftId).toBe('d9');
    expect(onClose).toHaveBeenCalled();
  });

  it('a reopened reply is queued as a reply to its message', async () => {
    undoDelay = '5';
    await act(async () => {
      root.render(
        <ComposeTabView
          tab={{ ...tab, toAddresses: ['bob@example.com'], replyToEmailId: 'e1' }}
          accounts={[account]}
          onClose={() => {}}
        />,
      );
    });
    await act(async () => {
      sendButton().click();
    });
    expect(vi.mocked(api.queueOutgoingEmail).mock.calls[0][0]).toMatchObject({ replyToEmailId: 'e1' });
  });
});
