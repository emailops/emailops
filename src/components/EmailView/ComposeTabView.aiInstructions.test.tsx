// The compose tab (a maximized New Compose, or a reopened draft) keeps the
// Instructions for AI apart from the email like the modal does: they steer
// Generate and Polish, and never reach the saved draft or the sent message.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const handlers = new Map<string, (e: { payload: unknown }) => void>();

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, cb: (e: { payload: unknown }) => void) => {
    handlers.set(name, cb);
    return () => handlers.delete(name);
  }),
}));
vi.mock('@/components/shared/RichTextEditor', () => ({
  RichTextEditor: ({
    value,
    onChange,
    disabled,
  }: {
    value: string;
    onChange: (v: string) => void;
    disabled?: boolean;
  }) => <textarea data-testid="body" value={value} disabled={disabled} onChange={(e) => onChange(e.target.value)} />,
}));
vi.mock('@/components/shared/TranslateComposeControl', () => ({ TranslateComposeControl: () => null }));
vi.mock('@/components/shared/Select', () => ({ Select: () => null }));
vi.mock('@/lib/api', () => ({
  getPref: vi.fn(async (key: string) => (key === 'compose.undo_send_delay_secs' ? '0' : null)),
  autocompleteRecipients: vi.fn(async () => []),
  saveDraft: vi.fn(async () => ({ id: 'd1' })),
  deleteDraft: vi.fn(async () => {}),
  sendNewEmail: vi.fn(async () => {}),
  sendDraft: vi.fn(async () => {}),
  generateNewDraft: vi.fn(async () => 'r1'),
  polishDraft: vi.fn(async () => 'r1'),
}));

import * as api from '@/lib/api';
import { AI_DRAFT_TIMEOUT_MS } from '@/lib/draftRequest';
import type { ComposeTab } from '@/stores/emailStore';
import { useOutboxStore } from '@/stores/outboxStore';
import type { Account } from '@/types';
import { ComposeTabView } from './ComposeTabView';

const account = { id: 'a1', email: 'me@example.com', name: 'Me' } as Account;
const STEER = 'SECRET-STEER make it short';
const tab: ComposeTab = {
  type: 'compose',
  id: 't1',
  accountId: 'a1',
  toAddresses: ['bob@example.com'],
  subject: 'Quarterly numbers',
  bodyHtml: '<p>Hi Bob, the numbers are ready.</p>',
};

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  useOutboxStore.setState({ undoDelaySecs: null });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
  vi.clearAllMocks();
  handlers.clear();
});

const q = <T extends Element>(selector: string) => {
  const el = container.querySelector<T>(selector);
  if (!el) throw new Error(`${selector} not rendered`);
  return el;
};
const body = () => q<HTMLTextAreaElement>('[data-testid="body"]');
const send = () => q<HTMLButtonElement>('[data-testid="compose-tab-send"]');

async function mountWithInstructions(t: ComposeTab = tab) {
  await act(async () => {
    root.render(<ComposeTabView tab={t} accounts={[account]} onClose={() => {}} />);
  });
  const input = q<HTMLInputElement>('[data-testid="ai-instructions"]');
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
  await act(async () => {
    setter?.call(input, STEER);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

async function click(selector: string) {
  await act(async () => {
    q<HTMLButtonElement>(selector).click();
  });
}

describe('ComposeTabView Instructions for AI', () => {
  it('generates from the instructions, with the body as the draft', async () => {
    await mountWithInstructions();
    await click('[data-testid="ai-generate"]');
    expect(api.generateNewDraft).toHaveBeenCalledWith(
      'a1',
      ['bob@example.com'],
      'Quarterly numbers',
      STEER,
      'Hi Bob, the numbers are ready.',
    );
  });

  it('polishes a reopened reply as a reply', async () => {
    await mountWithInstructions({ ...tab, replyToEmailId: 'e1' });
    await click('[data-testid="ai-polish"]');
    expect(api.polishDraft).toHaveBeenCalledWith('a1', 'reply', 'Hi Bob, the numbers are ready.', STEER);
  });

  it('does not offer Polish on a body with a link, which it would drop', async () => {
    await mountWithInstructions({ ...tab, bodyHtml: '<p>See <a href="https://example.com/report">the report</a></p>' });
    expect(q<HTMLButtonElement>('[data-testid="ai-polish"]').disabled).toBe(true);
  });

  it('never saves the instructions with the draft nor sends them', async () => {
    await mountWithInstructions();
    // An edit, so the tab saves.
    const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set;
    await act(async () => {
      setter?.call(body(), '<p>Hi Bob, the numbers are ready now.</p>');
      body().dispatchEvent(new Event('input', { bubbles: true }));
    });
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 900));
    });
    expect(api.saveDraft).toHaveBeenCalled();
    expect(JSON.stringify(vi.mocked(api.saveDraft).mock.calls)).not.toContain('SECRET-STEER');

    await click('[data-testid="compose-tab-send"]');
    expect(api.sendNewEmail).toHaveBeenCalledTimes(1);
    expect(JSON.stringify(vi.mocked(api.sendNewEmail).mock.calls)).not.toContain('SECRET-STEER');
  });

  it('locks the body and Send while the AI works, and frees them at the deadline with the text kept', async () => {
    await mountWithInstructions();
    vi.useFakeTimers();
    await click('[data-testid="ai-polish"]');
    expect(body().disabled).toBe(true);
    expect(send().disabled).toBe(true);

    await act(async () => {
      vi.advanceTimersByTime(AI_DRAFT_TIMEOUT_MS);
    });
    expect(body().disabled).toBe(false);
    expect(send().disabled).toBe(false);
    expect(body().value).toBe('<p>Hi Bob, the numbers are ready.</p>');
    expect(container.textContent).toContain('compose:aiDraft.timedOut');
  });
});
