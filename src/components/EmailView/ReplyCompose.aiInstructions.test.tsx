// Reply and Reply All keep the Instructions for AI apart from the reply:
// Polish gets the reply's own text (signature left out) and the instructions,
// and neither the saved draft nor the sent reply carries the instructions.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));
vi.mock('@/components/shared/RichTextEditor', () => ({
  RichTextEditor: ({ value, onChange }: { value: string; onChange: (html: string) => void }) => (
    <textarea data-testid="body" value={value} onChange={(e) => onChange(e.target.value)} />
  ),
}));
vi.mock('@/components/shared/TranslateComposeControl', () => ({ TranslateComposeControl: () => null }));
vi.mock('@/components/shared/Select', () => ({ Select: () => null }));

const api = vi.hoisted(() => ({
  autocompleteRecipients: vi.fn(async () => []),
  saveDraft: vi.fn(),
  deleteDraft: vi.fn(),
}));
vi.mock('@/lib/api', () => api);

import type { Account, Email } from '@/types';
import { ReplyCompose } from './ReplyCompose';

const account = { id: 'a1', email: 'me@example.com', name: 'Me' } as Account;
const email = {
  id: 'e1',
  accountId: 'a1',
  threadId: 't1',
  senderEmail: 'alice@example.com',
  recipients: ['me@example.com'],
  cc: [],
  subject: 'Hello',
  timestamp: 0,
} as unknown as Email;
const STEER = 'SECRET-STEER make it warmer';

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  api.saveDraft.mockReset().mockImplementation(async (req: { id?: string }) => ({ id: req.id ?? 'draft-new' }));
  api.deleteDraft.mockReset().mockResolvedValue(undefined);
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  container.remove();
});

async function setValue(el: HTMLInputElement | HTMLTextAreaElement, value: string) {
  const proto = el instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  const setter = Object.getOwnPropertyDescriptor(proto, 'value')?.set;
  await act(async () => {
    setter?.call(el, value);
    el.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

const q = <T extends Element>(selector: string) => {
  const el = container.querySelector<T>(selector);
  if (!el) throw new Error(`${selector} not rendered`);
  return el;
};

async function written(props: Partial<Parameters<typeof ReplyCompose>[0]> = {}) {
  await act(async () => {
    root.render(
      <ReplyCompose
        email={email}
        threadEmails={[email]}
        accounts={[account]}
        defaultAccountId="a1"
        onSend={async () => {}}
        onCancel={() => {}}
        initialBody=""
        mode="reply"
        onGenerateDraft={() => {}}
        onPolishDraft={() => {}}
        {...props}
      />,
    );
  });
  await setValue(q<HTMLTextAreaElement>('[data-testid="body"]'), '<p>Hi Alice, thursday works.</p>');
  await setValue(q<HTMLInputElement>('[data-testid="ai-instructions"]'), STEER);
}

describe('ReplyCompose Instructions for AI', () => {
  it("polishes the reply's own text with the instructions", async () => {
    const onPolishDraft = vi.fn();
    await written({ onPolishDraft });
    await act(async () => q<HTMLButtonElement>('[data-testid="ai-polish"]').click());
    expect(onPolishDraft).toHaveBeenCalledWith(STEER, 'Hi Alice, thursday works.');
  });

  it('does not offer Polish on a reply with a link, which it would drop', async () => {
    await written();
    await setValue(
      q<HTMLTextAreaElement>('[data-testid="body"]'),
      '<p>See <a href="https://example.com/report">the report</a></p>',
    );
    expect(q<HTMLButtonElement>('[data-testid="ai-polish"]').disabled).toBe(true);
  });

  it('shows why the last AI request failed', async () => {
    await written({ draftError: 'The AI took too long' });
    expect(container.textContent).toContain('The AI took too long');
  });

  it('does not send while the AI writes the reply, not even from Send anyway', async () => {
    const onSend = vi.fn(async () => {});
    await written({ onSend });
    // A pre-send warning ("attached" with no file) offers Send anyway.
    await setValue(q<HTMLTextAreaElement>('[data-testid="body"]'), '<p>Hi Alice, the plan is attached.</p>');
    await act(async () => q<HTMLButtonElement>('[data-testid="reply-send"]').click());
    const sendAnyway = [...container.querySelectorAll('button')].find(
      (b) => b.textContent === 'compose:sendWarnings.sendAnyway',
    );
    if (!sendAnyway) throw new Error('Send anyway not offered');

    // The user asks the AI for a new draft before answering the warning.
    await act(async () => {
      root.render(
        <ReplyCompose
          email={email}
          threadEmails={[email]}
          accounts={[account]}
          defaultAccountId="a1"
          onSend={onSend}
          onCancel={() => {}}
          initialBody=""
          mode="reply"
          onGenerateDraft={() => {}}
          onPolishDraft={() => {}}
          isLoadingDraft
        />,
      );
    });
    await act(async () => sendAnyway.click());
    expect(onSend).not.toHaveBeenCalled();
  });

  it('never saves the instructions with the draft nor sends them', async () => {
    const onSend = vi.fn(async () => {});
    await written({ onSend });
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 900));
    });
    expect(api.saveDraft).toHaveBeenCalled();
    expect(JSON.stringify(api.saveDraft.mock.calls)).not.toContain('SECRET-STEER');

    await act(async () => q<HTMLButtonElement>('[data-testid="reply-send"]').click());
    expect(onSend).toHaveBeenCalledTimes(1);
    expect(JSON.stringify(onSend.mock.calls)).not.toContain('SECRET-STEER');
    expect(JSON.stringify(onSend.mock.calls)).toContain('thursday works');
  });
});
