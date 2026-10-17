// The chat's account picker is the shared Select: outside macOS/Windows (so
// in jsdom too) it owns its popup, which is what keeps it dark on Linux.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

import { useAccountStore } from '@/stores/accountStore';
import type { Account } from '@/types';
import { ChatAccountPicker } from './ChatAccountPicker';

const account = (id: string, email: string): Account => ({
  id,
  provider: 'imap',
  email,
  name: email,
  createdAt: 1,
  sortOrder: 0,
  enabled: true,
  syncFromTimestamp: null,
});

let container: HTMLDivElement;
let root: Root;
beforeEach(() => {
  useAccountStore.setState({ accounts: [account('a1', 'one@example.com'), account('a2', 'two@example.com')] });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe('ChatAccountPicker', () => {
  it('renders the shared Select, never a raw <select>', async () => {
    await act(async () => {
      root.render(<ChatAccountPicker accountId="a1" onChange={() => {}} compact />);
    });
    expect(container.querySelector('select')).toBeNull();
    const trigger = container.querySelector<HTMLButtonElement>('button[aria-label="chat:accountPicker.label"]');
    expect(trigger?.textContent).toContain('one@example.com');
  });

  it('opens its own popup and reports the chosen account', async () => {
    const onChange = vi.fn();
    await act(async () => {
      root.render(<ChatAccountPicker accountId="a1" onChange={onChange} />);
    });
    const trigger = container.querySelector<HTMLButtonElement>('button[aria-label="chat:accountPicker.label"]');
    await act(async () => trigger?.click());
    const option = [...container.querySelectorAll<HTMLButtonElement>('button')].find((b) =>
      b.textContent?.includes('two@example.com'),
    );
    expect(option).toBeDefined();
    await act(async () => option?.click());
    expect(onChange).toHaveBeenCalledWith('a2');
  });

  it('is not shown with a single enabled account', async () => {
    useAccountStore.setState({ accounts: [account('a1', 'one@example.com')] });
    await act(async () => {
      root.render(<ChatAccountPicker accountId="a1" onChange={() => {}} />);
    });
    expect(container.querySelector('button')).toBeNull();
  });
});
