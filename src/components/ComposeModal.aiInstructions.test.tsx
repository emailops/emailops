// New Compose keeps the Instructions for AI apart from the email: Generate and
// Polish read them, the body goes to the model as the draft, and neither the
// saved draft nor the sent message ever carries them. While the AI works, the
// body and Send are locked; a failure or a timeout frees them with the text
// untouched.

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
  // Undo send off: the message goes out directly.
  getPref: vi.fn(async (key: string) => (key === 'compose.undo_send_delay_secs' ? '0' : null)),
  autocompleteRecipients: vi.fn(async () => []),
  saveDraft: vi.fn(async () => ({ id: 'd1' })),
  deleteDraft: vi.fn(async () => {}),
  sendNewEmail: vi.fn(async () => {}),
  generateNewDraft: vi.fn(async () => 'r1'),
  polishDraft: vi.fn(async () => 'r1'),
  getAccountSignature: vi.fn(async (accountId: string) => ({
    accountId,
    html: '',
    useForNew: true,
    useForReplies: true,
    updatedAt: null,
  })),
}));

import * as api from '@/lib/api';
import { AI_DRAFT_TIMEOUT_MS } from '@/lib/draftRequest';
import { useOutboxStore } from '@/stores/outboxStore';
import { useSignatureStore } from '@/stores/signatureStore';
import type { Account } from '@/types';
import { ComposeModal } from './ComposeModal';

const account = { id: 'a1', email: 'me@example.com', name: 'Me' } as Account;
const STEER = 'SECRET-STEER make it short';

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
  useSignatureStore.setState({ byAccount: {} });
});

const q = <T extends Element>(selector: string) => {
  const el = document.querySelector<T>(selector);
  if (!el) throw new Error(`${selector} not rendered`);
  return el;
};
const body = () => q<HTMLTextAreaElement>('[data-testid="body"]');
const instructions = () => q<HTMLInputElement>('[data-testid="ai-instructions"]');
const generate = () => q<HTMLButtonElement>('[data-testid="ai-generate"]');
const polish = () => q<HTMLButtonElement>('[data-testid="ai-polish"]');
const send = () => q<HTMLButtonElement>('[data-testid="compose-send"]');

async function type(el: HTMLInputElement | HTMLTextAreaElement, value: string) {
  const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  const setter = Object.getOwnPropertyDescriptor(proto, 'value')?.set;
  await act(async () => {
    setter?.call(el, value);
    el.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

async function click(el: HTMLElement) {
  await act(async () => {
    el.click();
  });
}

/** A composer with a recipient, a subject, a body and instructions typed. */
async function composed() {
  await act(async () => {
    root.render(
      <ComposeModal
        accounts={[account]}
        defaultAccountId="a1"
        defaultToRecipients={['bob@example.com']}
        onClose={() => {}}
      />,
    );
  });
  await type(q<HTMLInputElement>('input[placeholder="compose:subjectPlaceholderLong"]'), 'Quarterly numbers');
  await type(body(), '<p>Hi Bob, the numbers are ready.</p>');
  await type(instructions(), STEER);
}

describe('ComposeModal Instructions for AI', () => {
  it('generates from the instructions, with the body as the draft', async () => {
    await composed();
    await click(generate());
    expect(api.generateNewDraft).toHaveBeenCalledWith(
      'a1',
      ['bob@example.com'],
      'Quarterly numbers',
      STEER,
      'Hi Bob, the numbers are ready.',
    );
  });

  it('polishes the body following the instructions, and puts the result in its place', async () => {
    await composed();
    await click(polish());
    expect(api.polishDraft).toHaveBeenCalledWith('a1', 'new', 'Hi Bob, the numbers are ready.', STEER);
    await act(async () => {
      handlers.get('draft-generated')?.({
        payload: { requestId: 'r1', emailId: '', body: 'Hi Bob,\n\nThe numbers are ready.', sources: [] },
      });
    });
    expect(body().value).toContain('The numbers are ready.');
    expect(body().value).not.toContain('Hi Bob, the numbers are ready.');
  });

  it('never saves the instructions with the draft nor sends them', async () => {
    await composed();
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 900));
    });
    expect(api.saveDraft).toHaveBeenCalled();
    expect(JSON.stringify(vi.mocked(api.saveDraft).mock.calls)).not.toContain('SECRET-STEER');

    await click(send());
    expect(api.sendNewEmail).toHaveBeenCalledTimes(1);
    expect(JSON.stringify(vi.mocked(api.sendNewEmail).mock.calls)).not.toContain('SECRET-STEER');
    expect(JSON.stringify(vi.mocked(api.sendNewEmail).mock.calls)).toContain('the numbers are ready');
  });

  it('locks the body and Send while the AI works', async () => {
    await composed();
    expect(send().disabled).toBe(false);
    await click(generate());
    expect(body().disabled).toBe(true);
    expect(send().disabled).toBe(true);
    expect(instructions().disabled).toBe(true);
  });

  it('frees the composer after a failure, keeps the text and says why', async () => {
    await composed();
    await click(polish());
    await act(async () => {
      handlers.get('draft-failed')?.({ payload: { requestId: 'r1', emailId: '', error: 'too long' } });
    });
    expect(body().disabled).toBe(false);
    expect(send().disabled).toBe(false);
    expect(body().value).toBe('<p>Hi Bob, the numbers are ready.</p>');
    expect(instructions().value).toBe(STEER);
    expect(container.ownerDocument.body.textContent).toContain('compose:aiDraft.failed');
  });

  it('gives up at the deadline: frees the composer, keeps the text and drops a late draft', async () => {
    await composed();
    vi.useFakeTimers();
    await click(generate());
    await act(async () => {
      vi.advanceTimersByTime(AI_DRAFT_TIMEOUT_MS);
    });
    expect(body().disabled).toBe(false);
    expect(send().disabled).toBe(false);
    expect(container.ownerDocument.body.textContent).toContain('compose:aiDraft.timedOut');

    await act(async () => {
      handlers.get('draft-generated')?.({ payload: { requestId: 'r1', emailId: '', body: 'Late draft', sources: [] } });
    });
    expect(body().value).toBe('<p>Hi Bob, the numbers are ready.</p>');
  });
});
