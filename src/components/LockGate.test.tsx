// The lock check runs before anything else renders: if it cannot be answered
// the window must say so, not stay blank, and must not show the mailbox.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));
vi.mock('@/lib/api', () => ({
  hasMainPassword: vi.fn(),
  verifyMainPassword: vi.fn(),
}));

import * as api from '@/lib/api';
import { useLogStore } from '@/stores/logStore';
import { LockGate } from './LockGate';

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.clearAllMocks();
  useLogStore.setState({ entries: [] });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function mount() {
  await act(async () => {
    root.render(
      <LockGate>
        <p>mailbox</p>
      </LockGate>,
    );
  });
}

describe('LockGate', () => {
  it('shows the app when no password is set', async () => {
    vi.mocked(api.hasMainPassword).mockResolvedValue(false);
    await mount();
    expect(container.textContent).toBe('mailbox');
  });

  it('asks for the password when one is set', async () => {
    vi.mocked(api.hasMainPassword).mockResolvedValue(true);
    await mount();
    expect(container.textContent).not.toContain('mailbox');
    expect(container.textContent).toContain('auth:lock.title');
  });

  it('says it could not check, without showing the app, and offers to retry', async () => {
    vi.mocked(api.hasMainPassword).mockRejectedValueOnce(new Error('db locked'));
    await mount();

    expect(container.textContent).not.toContain('mailbox');
    expect(container.textContent).toContain('auth:lock.checkFailed');
    expect(useLogStore.getState().entries.some((e) => e.level === 'error')).toBe(true);

    vi.mocked(api.hasMainPassword).mockResolvedValue(false);
    const retry = [...container.querySelectorAll('button')].find((b) => b.textContent === 'common:actions.retry');
    await act(async () => retry?.click());

    expect(container.textContent).toBe('mailbox');
  });
});
