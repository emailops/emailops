// An address typed into the To/Cc box of a reply but not tokenised (no
// Enter/Tab, no suggestion picked) must still count: it enables Send and goes
// out with the message, like the modal and tab composers.

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
  senderEmail: 'alice@example.com',
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

async function typeInto(id: string, value: string) {
  const input = container.querySelector<HTMLInputElement>(`#${id}`);
  if (!input) throw new Error(`${id} not rendered`);
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
  await act(async () => {
    setter?.call(input, value);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

function sendButton(): HTMLButtonElement {
  const b = [...container.querySelectorAll('button')].find((x) => x.textContent === 'compose:forward');
  if (!b) throw new Error('Send button not rendered');
  return b;
}

describe('ReplyCompose with a recipient typed but not tokenised', () => {
  it('enables Send and sends to the pending To and Cc addresses', async () => {
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
    expect(sendButton().disabled).toBe(true);

    await typeInto('to-input', 'bob@example.com');
    const ccToggle = [...container.querySelectorAll('button')].find((b) => b.textContent === '+ Cc');
    await act(async () => ccToggle?.click());
    await typeInto('cc-input', 'carol@example.com');
    expect(sendButton().disabled).toBe(false);

    await act(async () => {
      sendButton().click();
    });
    expect(onSend).toHaveBeenCalledWith(
      expect.objectContaining({ toEmails: ['bob@example.com'], ccEmails: ['carol@example.com'] }),
    );
  });
});
