// In "All accounts" the "New event" dialog asks which account the event goes
// to: the first writable account by default, and a Proton account (read-only
// feed, no write API) hands the day over to Proton Calendar's own form.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string) => key,
    i18n: { language: 'en' },
  }),
}));
vi.mock('@/hooks/useFormatters', () => ({
  useFormatters: () => ({ time: () => '00:00', date: () => '', dateTime: () => '' }),
}));
const api = vi.hoisted(() => ({
  createCalendarEvent: vi.fn(),
  autocompleteRecipients: vi.fn(async () => []),
  currentPlatform: vi.fn(() => ''),
}));
vi.mock('@/lib/api', () => api);
const shell = vi.hoisted(() => ({ open: vi.fn(async () => {}) }));
vi.mock('@tauri-apps/plugin-shell', () => shell);

import type { CreateAccountChoice } from '@/lib/calendarAccounts';
import type { Account } from '@/types';
import { NewEventDialog } from './NewEventDialog';

function account(id: string, provider: Account['provider'] = 'gmail'): Account {
  return {
    id,
    provider,
    email: `${id}@mail.example`,
    name: '',
    createdAt: 0,
    sortOrder: 0,
    enabled: true,
    syncFromTimestamp: null,
    color: 'blue',
  };
}

const PROTON_URL = 'https://calendar.proton.me/week/2026/8/18?action=create&title=';
const choices: CreateAccountChoice[] = [
  { account: account('work'), plan: { kind: 'dialog' } },
  { account: account('home'), plan: { kind: 'dialog' } },
  { account: account('proton', 'imap'), plan: { kind: 'proton', url: PROTON_URL } },
];

let container: HTMLDivElement;
let root: Root;
const onClose = vi.fn();

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.clearAllMocks();
});

function renderDialog(list: CreateAccountChoice[]) {
  const start = Math.floor(new Date(2026, 7, 18, 5, 0).getTime() / 1000);
  act(() => {
    root.render(
      <NewEventDialog
        choices={list}
        initialStart={start}
        initialEnd={start + 3600}
        onClose={onClose}
        onCreated={() => {}}
        onAuthError={() => {}}
      />,
    );
  });
}

function accountSelect(): HTMLElement | null {
  return container.querySelector('[aria-label="calendar:create.account"]');
}

function pickAccount(email: string) {
  const trigger = accountSelect();
  if (!trigger) throw new Error('account selector not rendered');
  act(() => trigger.click());
  const option = Array.from(document.querySelectorAll('[role="option"]')).find((o) => o.textContent === email);
  if (!option) throw new Error(`option ${email} not rendered`);
  act(() => (option as HTMLElement).click());
}

describe('NewEventDialog account choice', () => {
  it('asks for no account when only one can take the event', () => {
    renderDialog([choices[0]]);
    expect(accountSelect()).toBeNull();
  });

  it('defaults to the first writable account', () => {
    renderDialog(choices);
    expect(accountSelect()?.textContent).toContain('work@mail.example');
  });

  it('creates the event in the picked account', async () => {
    api.createCalendarEvent.mockResolvedValue({ id: 'ev' });
    renderDialog(choices);
    pickAccount('home@mail.example');
    const title = container.querySelector<HTMLInputElement>('#new-event-title');
    if (!title) throw new Error('title not rendered');
    const setValue = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
    act(() => {
      setValue?.call(title, 'Planning');
      title.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await act(async () => {
      container.querySelector('form')?.requestSubmit();
    });
    expect(api.createCalendarEvent.mock.calls[0][0]).toBe('home');
  });

  it('hands a Proton account over to Proton Calendar instead of the form', async () => {
    renderDialog(choices);
    pickAccount('proton@mail.example');
    expect(container.querySelector('#new-event-title')).toBeNull();
    const button = Array.from(container.querySelectorAll('button')).find(
      (b) => b.textContent === 'calendar:create.openProton',
    );
    if (!button) throw new Error('Proton button not rendered');
    await act(async () => button.click());
    expect(shell.open).toHaveBeenCalledWith(PROTON_URL);
    expect(onClose).toHaveBeenCalled();
  });
});
