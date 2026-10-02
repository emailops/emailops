// Settings → Notifications: new-mail desktop notification switches, stored as
// backend prefs (master, per account, content, only-when-unfocused).

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/lib/api')>()),
  getPref: vi.fn(),
  setPref: vi.fn(),
}));

import { initI18n } from '@/i18n';
import * as api from '@/lib/api';
import type { Account } from '@/types';
import { NotificationsSettings } from './NotificationsSettings';

const work = { id: 'g1', email: 'ana@example.com', provider: 'gmail', enabled: true } as Account;
const home = { id: 'i1', email: 'ana@example.org', provider: 'imap', enabled: true } as Account;

let container: HTMLDivElement;
let root: Root;
let stored: Record<string, string>;

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  stored = {};
  vi.mocked(api.getPref).mockImplementation(async (key) => stored[key] ?? null);
  vi.mocked(api.setPref).mockResolvedValue(undefined);
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.clearAllMocks();
});

async function render(accounts: Account[] = [work, home]) {
  await act(async () => {
    root.render(<NotificationsSettings accounts={accounts} />);
  });
}

function switchNamed(name: string): HTMLButtonElement {
  const el = Array.from(container.querySelectorAll<HTMLButtonElement>('[role="switch"]')).find(
    (b) => b.getAttribute('aria-label') === name,
  );
  if (!el) throw new Error(`no switch labelled ${name}`);
  return el;
}

function radio(value: string): HTMLInputElement {
  const el = container.querySelector<HTMLInputElement>(`input[type="radio"][value="${value}"]`);
  if (!el) throw new Error(`no radio ${value}`);
  return el;
}

async function click(el: HTMLElement) {
  await act(async () => {
    el.click();
  });
}

describe('NotificationsSettings', () => {
  it('defaults to everything on and showing sender and subject', async () => {
    await render();
    expect(switchNamed('New mail notifications').getAttribute('aria-checked')).toBe('true');
    expect(switchNamed('Notify for ana@example.com').getAttribute('aria-checked')).toBe('true');
    expect(switchNamed('Notify for ana@example.org').getAttribute('aria-checked')).toBe('true');
    expect(switchNamed('Only when EmailOps is not focused').getAttribute('aria-checked')).toBe('true');
    expect(radio('preview').checked).toBe(true);
    expect(radio('hidden').checked).toBe(false);
  });

  it('reflects stored preferences', async () => {
    stored = {
      'notifications.new_mail.account:i1': 'false',
      'notifications.new_mail.content': 'hidden',
      'notifications.new_mail.only_unfocused': 'false',
    };
    await render();
    expect(switchNamed('Notify for ana@example.com').getAttribute('aria-checked')).toBe('true');
    expect(switchNamed('Notify for ana@example.org').getAttribute('aria-checked')).toBe('false');
    expect(switchNamed('Only when EmailOps is not focused').getAttribute('aria-checked')).toBe('false');
    expect(radio('hidden').checked).toBe(true);
  });

  it('saves each change as a backend preference', async () => {
    await render();
    await click(switchNamed('Notify for ana@example.org'));
    expect(api.setPref).toHaveBeenCalledWith('notifications.new_mail.account:i1', 'false');
    await click(radio('hidden'));
    expect(api.setPref).toHaveBeenCalledWith('notifications.new_mail.content', 'hidden');
    await click(switchNamed('Only when EmailOps is not focused'));
    expect(api.setPref).toHaveBeenCalledWith('notifications.new_mail.only_unfocused', 'false');
    await click(switchNamed('New mail notifications'));
    expect(api.setPref).toHaveBeenCalledWith('notifications.new_mail.enabled', 'false');
  });

  it('turning the master switch off disables the other controls', async () => {
    stored = { 'notifications.new_mail.enabled': 'false' };
    await render();
    expect(switchNamed('Notify for ana@example.com').disabled).toBe(true);
    expect(radio('hidden').disabled).toBe(true);
    expect(switchNamed('Only when EmailOps is not focused').disabled).toBe(true);
  });

  it('reverts a change the backend rejects and says so', async () => {
    vi.mocked(api.setPref).mockRejectedValueOnce(new Error('disk full'));
    await render();
    await click(switchNamed('Notify for ana@example.com'));
    expect(switchNamed('Notify for ana@example.com').getAttribute('aria-checked')).toBe('true');
    expect(container.textContent).toContain('disk full');
  });
});
