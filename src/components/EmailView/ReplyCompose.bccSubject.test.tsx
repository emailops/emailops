// #270: a reply or forward can carry blind copies and an edited subject, and a
// reply answers a message's Reply-To instead of its sender.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));
vi.mock('@/components/shared/RichTextEditor', () => ({
  RichTextEditor: ({ value }: { value: string }) => <div data-testid="body">{value}</div>,
}));
vi.mock('@/components/shared/TranslateComposeControl', () => ({ TranslateComposeControl: () => null }));
vi.mock('@/components/shared/Select', () => ({ Select: () => null }));
vi.mock('@/lib/api', () => ({ autocompleteRecipients: vi.fn(async () => []) }));

import type { Account, Email } from '@/types';
import { ReplyCompose } from './ReplyCompose';

const account = { id: 'a1', email: 'me@example.com', name: 'Me' } as Account;
const email = {
  id: 'e1',
  accountId: 'a1',
  threadId: 't1',
  senderEmail: 'noreply@shop.example',
  recipients: ['me@example.com'],
  cc: [],
  subject: 'Hello',
  timestamp: 0,
} as unknown as Email;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function typeInto(input: HTMLInputElement | null, value: string) {
  if (!input) throw new Error('input not rendered');
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
  await act(async () => {
    setter?.call(input, value);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

function button(text: string): HTMLButtonElement {
  const b = [...container.querySelectorAll('button')].find((x) => x.textContent === text);
  if (!b) throw new Error(`${text} not rendered`);
  return b;
}

const subjectInput = () => container.querySelector<HTMLInputElement>('[data-testid="reply-subject"]');

describe('ReplyCompose blind copies and subject', () => {
  it('forwards to a Bcc alone, with the subject the user edited', async () => {
    const onSend = vi.fn(async () => {});
    await act(async () => {
      root.render(
        <ReplyCompose
          email={email}
          threadEmails={[email]}
          accounts={[account]}
          defaultAccountId="a1"
          onSend={onSend}
          onCancel={() => {}}
          initialBody="See below"
          mode="forward"
        />,
      );
    });
    expect(subjectInput()?.value).toBe('Fwd: Hello');

    await act(async () => button('+ compose:bcc').click());
    await typeInto(container.querySelector<HTMLInputElement>('#bcc-input'), 'hidden@example.com');
    await typeInto(subjectInput(), 'Fwd: Hello (for the team)');
    expect(button('compose:forward').disabled).toBe(false);

    await act(async () => button('compose:forward').click());
    expect(onSend).toHaveBeenCalledWith(
      expect.objectContaining({
        toEmails: [],
        bccEmails: ['hidden@example.com'],
        subject: 'Fwd: Hello (for the team)',
      }),
    );
  });

  it('opens a reply at the Reply-To address with the "Re:" subject', async () => {
    await act(async () => {
      root.render(
        <ReplyCompose
          email={email}
          threadEmails={[email]}
          accounts={[account]}
          defaultAccountId="a1"
          onSend={vi.fn(async () => {})}
          onCancel={() => {}}
          initialBody=""
          mode="reply"
          replyTo={{ e1: ['help@shop.example'] }}
        />,
      );
    });

    expect(subjectInput()?.value).toBe('Re: Hello');
    expect(container.textContent).toContain('help@shop.example');
    expect(container.textContent).not.toContain('noreply@shop.example');
  });
});
