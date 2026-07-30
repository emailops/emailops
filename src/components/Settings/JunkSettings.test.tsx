// Regression: the Settings radio and the inbox checkbox are two controls over
// ONE preference, and they disagreed.
//
// `JunkSettings` kept the config in local state and wrote it straight through
// `api.setJunkConfig`. `useJunkStore` — which the inbox reads to decide whether
// to drop flagged rows — loads the preference exactly once, at app start. So
// ticking "keep it out of the inbox" persisted the choice and changed nothing
// on screen: the inbox was still consulting the stale `'dim'` it had cached,
// and the "Hide N junk messages" checkbox above the list still showed unticked.
// Only a restart made the setting appear to work.
//
// The fix is for the store to be the single source of truth the moment the
// write lands. These tests pin both directions: the store learns, and a failed
// write does not leave the store claiming something the backend rejected.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

vi.mock('@/stores/logStore', () => ({
  useLogStore: (selector: (s: { addLog: () => void }) => unknown) => selector({ addLog: vi.fn() }),
}));

interface TestAccount {
  id: string;
  email: string;
  enabled: boolean;
}

// Hoisted so the module mock below can read whatever a test assigns, without
// tripping the temporal-dead-zone problem `vi.mock` hoisting otherwise creates.
const mocks = vi.hoisted(() => ({ accounts: [] as { id: string; email: string; enabled: boolean }[] }));

vi.mock('@/stores/accountStore', () => ({
  useAccountStore: (selector: (s: { accounts: unknown[] }) => unknown) => selector({ accounts: mocks.accounts }),
}));

const getJunkConfig = vi.fn(() => Promise.resolve({ enabled: true, phishingEnabled: false, flaggedAction: 'dim' }));
const setJunkConfig = vi.fn(() => Promise.resolve());
/** Stats shaped like the backend's, with the counts a test wants to see. */
function statsFor(scored: number, unscored: number) {
  return {
    scored,
    unscored,
    phishing: 0,
    spam: 0,
    graymail: 0,
    markedJunk: 0,
    markedNotJunk: 0,
    models: [] as never[],
  };
}

// Annotated, not inferred: the default implementation only ever rejects, so
// inference would pin the mock to `Promise<never>` and reject every resolving
// override a test installs.
const getJunkStats = vi.fn(
  (_accountId: string): Promise<ReturnType<typeof statsFor>> => Promise.reject(new Error('no stats in this test')),
);

vi.mock('@/lib/api', () => ({
  getJunkConfig: (...args: unknown[]) => getJunkConfig(...(args as [])),
  setJunkConfig: (...args: unknown[]) => setJunkConfig(...(args as [])),
  getJunkStats: (...args: unknown[]) => getJunkStats(...(args as [string])),
  backfillJunkScores: vi.fn(() => Promise.resolve()),
  getJunkVerdicts: vi.fn(() => Promise.resolve({})),
  setJunkFeedback: vi.fn(() => Promise.resolve()),
}));

import { useJunkStore } from '@/stores/junkStore';
import { JunkSettings } from './JunkSettings';

/** Click the radio for one of the two flagged-mail actions. */
function radioFor(container: HTMLElement, action: 'dim' | 'hide'): HTMLInputElement {
  const radios = Array.from(container.querySelectorAll<HTMLInputElement>('input[name="junk-flagged-action"]'));
  const index = action === 'dim' ? 0 : 1;
  const radio = radios[index];
  if (!radio) throw new Error(`expected two flagged-action radios, found ${radios.length}`);
  return radio;
}

describe('JunkSettings', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    useJunkStore.setState({ flaggedAction: 'dim' });
    setJunkConfig.mockClear();
    setJunkConfig.mockImplementation(() => Promise.resolve());
    getJunkStats.mockClear();
    getJunkStats.mockImplementation(() => Promise.reject(new Error('no stats in this test')));
    mocks.accounts = [];
  });

  afterEach(() => {
    act(() => {
      root.unmount();
    });
    container.remove();
  });

  async function mount() {
    await act(async () => {
      root.render(<JunkSettings />);
    });
    // Drain the config load so the radios reflect the persisted preference.
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });
  }

  it('publishes the chosen action to the store so the inbox stops showing junk', async () => {
    await mount();

    await act(async () => {
      radioFor(container, 'hide').click();
    });

    expect(setJunkConfig).toHaveBeenCalledWith({ enabled: true, phishingEnabled: false, flaggedAction: 'hide' });
    // The assertion that matters: the inbox reads this, not the backend.
    expect(useJunkStore.getState().flaggedAction).toBe('hide');
  });

  it('leaves the store alone when the write fails', async () => {
    await mount();
    setJunkConfig.mockImplementation(() => Promise.reject(new Error('disk full')));

    await act(async () => {
      radioFor(container, 'hide').click();
    });

    expect(useJunkStore.getState().flaggedAction).toBe('dim');
  });

  it('shows the action the store already holds, not a hardcoded default', async () => {
    // The inbox checkbox writes through the store. Opening Settings afterwards
    // must not present "fade it out" as the current choice.
    getJunkConfig.mockImplementationOnce(() =>
      Promise.resolve({ enabled: true, phishingEnabled: false, flaggedAction: 'hide' }),
    );
    await mount();

    expect(radioFor(container, 'hide').checked).toBe(true);
    expect(useJunkStore.getState().flaggedAction).toBe('hide');
  });

  // Regression: the panel asked for stats for `activeAccountId` alone, so on an
  // install with several mailboxes connected it reported on exactly one of them
  // — and which one depended on whatever the rest of the app had selected. The
  // counts, the trained models and the "score N older messages" button are all
  // per-account, so all of them have to be per-account on screen too.
  describe('with several accounts connected', () => {
    const accounts: TestAccount[] = [
      { id: 'acct-1', email: 'first@example.com', enabled: true },
      { id: 'acct-2', email: 'second@example.com', enabled: true },
    ];

    beforeEach(() => {
      mocks.accounts = accounts;
      getJunkStats.mockImplementation((accountId: string) =>
        Promise.resolve(accountId === 'acct-1' ? statsFor(11, 2) : statsFor(97, 0)),
      );
    });

    it('asks for stats for every account, not just the active one', async () => {
      await mount();

      const asked = getJunkStats.mock.calls.map(([id]) => id);
      expect(new Set(asked)).toEqual(new Set(['acct-1', 'acct-2']));
    });

    it('names every account and shows its own counts', async () => {
      await mount();

      const text = container.textContent ?? '';
      expect(text).toContain('first@example.com');
      expect(text).toContain('second@example.com');
      // Both accounts' scored counts, so neither block is showing the other's.
      expect(text).toContain('11');
      expect(text).toContain('97');
    });

    it('gives each account its own backfill button', async () => {
      await mount();

      // Only the backfill buttons — the two ToggleSwitches render buttons too.
      // `t` is mocked to echo the key, so the label is the key itself.
      const backfillButtons = Array.from(container.querySelectorAll('button')).filter(
        (b) => b.textContent === 'settings:junk.backfill',
      );
      expect(backfillButtons).toHaveLength(accounts.length);
      // acct-2 has nothing left to score, so its button must be disabled while
      // acct-1's (2 unscored) is live. A single shared button could not express that.
      expect(backfillButtons.map((b) => b.disabled)).toEqual([false, true]);
    });

    it('skips disabled accounts — nothing syncs into them to score', async () => {
      mocks.accounts = [accounts[0] as TestAccount, { id: 'acct-3', email: 'off@example.com', enabled: false }];
      await mount();

      expect(container.textContent ?? '').not.toContain('off@example.com');
      expect(getJunkStats.mock.calls.map(([id]) => id)).not.toContain('acct-3');
    });
  });

  it('scrolls its own content with the same padding as every other settings panel', async () => {
    // Every sibling panel owns a `overflow-y-auto flex-1 px-6 py-5` container;
    // SettingsDialog gives the tab body no padding of its own. Without it the
    // controls sit flush against the dialog edge and a long panel cannot scroll.
    await mount();

    const scroller = container.querySelector('.overflow-y-auto');
    expect(scroller, 'panel must own a scroll container').not.toBeNull();
    expect(scroller?.className).toContain('px-6');
    expect(scroller?.className).toContain('py-5');
  });
});
