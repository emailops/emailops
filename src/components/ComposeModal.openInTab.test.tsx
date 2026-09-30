// "Open in tab" hands the composer's full state to the tab: Cc, attached
// files, an address typed but not tokenised, and the draft row already
// auto-saved. Dropping the draft id made the tab create a second draft; the
// rest was silently lost.

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
vi.mock('@/lib/api', () => ({
  getPref: vi.fn(async () => null),
  autocompleteRecipients: vi.fn(async () => []),
  saveDraft: vi.fn(async () => ({ id: 'd1' })),
  deleteDraft: vi.fn(async () => {}),
  sendNewEmail: vi.fn(async () => {}),
  generateNewDraft: vi.fn(async () => 'req'),
}));

import * as api from '@/lib/api';
import type { Account } from '@/types';
import { type ComposeMaximizeState, ComposeModal } from './ComposeModal';

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

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
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

async function mount(onMaximize: (s: ComposeMaximizeState) => void) {
  await act(async () => {
    root.render(<ComposeModal accounts={[account]} defaultAccountId="a1" onClose={() => {}} onMaximize={onMaximize} />);
  });
}

async function typeInto(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
  await act(async () => {
    setter?.call(input, value);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

function byId(id: string): HTMLInputElement {
  const el = document.querySelector<HTMLInputElement>(`#${id}`);
  if (!el) throw new Error(`${id} not rendered`);
  return el;
}

async function fillComposer() {
  await typeInto(byId('compose-to-input'), 'bob@example.com');
  const ccToggle = [...document.querySelectorAll('button')].find((b) => b.textContent === '+ Cc');
  await act(async () => ccToggle?.click());
  await typeInto(byId('compose-cc-input'), 'carol@example.com');
  const subject = document.querySelector<HTMLInputElement>('input[placeholder="compose:subjectPlaceholderLong"]');
  if (!subject) throw new Error('subject not rendered');
  await typeInto(subject, 'Quarterly numbers');
}

async function clickOpenInTab() {
  const button = document.querySelector<HTMLButtonElement>('button[title="compose:openInTab"]');
  if (!button) throw new Error('open-in-tab not rendered');
  await act(async () => {
    button.click();
    await vi.runAllTimersAsync();
  });
}

describe('ComposeModal "Open in tab"', () => {
  it('carries Cc, pending recipients and the saved draft id', async () => {
    const onMaximize = vi.fn();
    await mount(onMaximize);
    await fillComposer();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1000);
    });

    await clickOpenInTab();

    expect(onMaximize).toHaveBeenCalledWith(
      expect.objectContaining({
        toAddresses: ['bob@example.com'],
        ccAddresses: ['carol@example.com'],
        subject: 'Quarterly numbers',
        draftId: 'd1',
      }),
    );
  });

  it('saves an edit still in the quiet period once, and hands over that draft', async () => {
    const onMaximize = vi.fn();
    await mount(onMaximize);
    await fillComposer();

    await clickOpenInTab();

    expect(api.saveDraft).toHaveBeenCalledTimes(1);
    expect(onMaximize.mock.calls[0][0].draftId).toBe('d1');
  });

  it('carries attached files', async () => {
    const onMaximize = vi.fn();
    await mount(onMaximize);
    const fileInput = document.querySelector<HTMLInputElement>('input[type="file"]');
    if (!fileInput) throw new Error('file input not rendered');
    const file = new File(['hello'], 'notes.txt', { type: 'text/plain' });
    Object.defineProperty(fileInput, 'files', { value: [file], configurable: true });
    await act(async () => {
      fileInput.dispatchEvent(new Event('change', { bubbles: true }));
    });
    // Open in tab waits for files still being read.
    await vi.waitFor(async () => {
      await act(async () => {
        await vi.runAllTimersAsync();
      });
      expect(document.querySelector<HTMLButtonElement>('button[title="compose:openInTab"]')?.disabled).toBe(false);
    });

    await clickOpenInTab();

    expect(onMaximize.mock.calls[0][0].attachments).toEqual([
      expect.objectContaining({ filename: 'notes.txt', mimeType: 'text/plain' }),
    ]);
  });
});
