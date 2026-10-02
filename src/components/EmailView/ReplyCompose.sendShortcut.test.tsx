// Cmd+Enter in the inline reply composer goes through the composer's one send
// function, exactly once — the same path as the Send button.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));
vi.mock('@/components/shared/RichTextEditor', () => ({
  RichTextEditor: ({ value }: { value: string }) => (
    <div data-testid="body" contentEditable suppressContentEditableWarning>
      {value}
    </div>
  ),
}));
vi.mock('@/components/shared/TranslateComposeControl', () => ({ TranslateComposeControl: () => null }));
vi.mock('@/components/shared/Select', () => ({ Select: () => null }));
vi.mock('@/lib/api', () => ({
  autocompleteRecipients: vi.fn(async () => []),
  getAccountSignature: vi.fn(async () => null),
  currentPlatform: () => 'macos',
}));

import { useShortcutStore } from '@/stores/shortcutStore';
import { useSignatureStore } from '@/stores/signatureStore';
import type { Account, Email } from '@/types';
import { ReplyCompose } from './ReplyCompose';

const account = { id: 'a1', email: 'me@example.com', name: 'Me' } as Account;
const email = {
  id: 'e1',
  accountId: 'a1',
  threadId: 't1',
  senderEmail: 'bea@example.com',
  recipients: ['me@example.com'],
  cc: [],
  subject: 'Hello',
  timestamp: 0,
} as unknown as Email;

let container: HTMLDivElement;
let root: Root;
const onSend = vi.fn(async () => {});

beforeEach(() => {
  onSend.mockClear();
  useShortcutStore.setState({ enabled: true });
  useSignatureStore.setState({ byAccount: {} });
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
      <ReplyCompose
        email={email}
        threadEmails={[email]}
        accounts={[account]}
        defaultAccountId="a1"
        onSend={onSend}
        onCancel={() => {}}
        initialBody="Thanks, see you then."
        mode="reply"
      />,
    );
  });
}

async function pressInBody(init: KeyboardEventInit) {
  const body = container.querySelector('[data-testid="body"]') as HTMLElement;
  const event = new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true, ...init });
  await act(async () => {
    body.dispatchEvent(event);
  });
  return event;
}

describe('ReplyCompose send shortcut', () => {
  it('Cmd+Enter sends once and claims the key from the editor', async () => {
    await render();
    const event = await pressInBody({ metaKey: true });
    expect(onSend).toHaveBeenCalledTimes(1);
    expect(onSend).toHaveBeenCalledWith(expect.objectContaining({ toEmails: ['bea@example.com'] }));
    expect(event.defaultPrevented).toBe(true);
  });

  it('plain Enter is a new line, not a send', async () => {
    await render();
    await pressInBody({});
    expect(onSend).not.toHaveBeenCalled();
  });

  it('does nothing when keyboard shortcuts are turned off', async () => {
    useShortcutStore.setState({ enabled: false });
    await render();
    const event = await pressInBody({ metaKey: true });
    expect(onSend).not.toHaveBeenCalled();
    expect(event.defaultPrevented).toBe(false);
  });
});
