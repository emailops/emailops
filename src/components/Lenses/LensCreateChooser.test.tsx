// "New Lens" first asks how to build it: with the chat (which opens a new
// conversation with a request to complete) or by hand (the create dialog).

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, string>) => (opts?.account ? `${key}|${opts.account}` : key),
    i18n: { language: 'es' },
  }),
}));

const accountState = {
  activeAccountId: 'acc-1' as string | null,
  accounts: [{ id: 'acc-1', email: 'owner@studio.example' }],
};
vi.mock('@/stores/accountStore', async () => {
  const actual = await vi.importActual<typeof import('@/stores/accountStore')>('@/stores/accountStore');
  return {
    ...actual,
    useAccountStore: (selector: (s: typeof accountState) => unknown) => selector(accountState),
  };
});

import { ALL_ACCOUNTS_ID } from '@/stores/accountStore';
import { LensCreateChooser } from './LensCreateChooser';

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  accountState.activeAccountId = 'acc-1';
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function render(onManual = vi.fn(), onChat = vi.fn(), onClose = vi.fn()) {
  act(() => root.render(<LensCreateChooser open onClose={onClose} onManual={onManual} onChat={onChat} />));
  return { onManual, onChat, onClose };
}

const button = (label: string) =>
  [...document.querySelectorAll('button')].find((b) => b.textContent?.includes(label)) as HTMLButtonElement;

describe('LensCreateChooser', () => {
  it('opens the manual dialog', () => {
    const { onManual, onChat } = render();
    act(() => button('lenses:chooser.manual').click());
    expect(onManual).toHaveBeenCalledTimes(1);
    expect(onChat).not.toHaveBeenCalled();
  });

  it('hands the chat a request naming the current account', () => {
    const { onChat } = render();
    act(() => button('lenses:chooser.chat').click());
    expect(onChat).toHaveBeenCalledWith('lenses:chooser.chatPrompt|owner@studio.example');
  });

  it('asks for all accounts in the unified view', () => {
    accountState.activeAccountId = ALL_ACCOUNTS_ID;
    const { onChat } = render();
    act(() => button('lenses:chooser.chat').click());
    expect(onChat).toHaveBeenCalledWith('lenses:chooser.chatPromptAllAccounts');
  });
});
