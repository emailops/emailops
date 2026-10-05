// Settings → Appearance: what opens after archiving or deleting the open
// conversation, stored in the SQLite preferences.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({
  getPref: vi.fn(async () => null),
  setPref: vi.fn(async () => {}),
  currentPlatform: () => 'macos',
}));

import { initI18n } from '@/i18n';
import * as api from '@/lib/api';
import { AFTER_LEAVE_PREF } from '@/lib/autoAdvance';
import { useAutoAdvanceStore } from '@/stores/autoAdvanceStore';
import { useLogStore } from '@/stores/logStore';
import { AutoAdvanceSetting } from './AutoAdvanceSetting';

let container: HTMLDivElement;
let root: Root;

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  useAutoAdvanceStore.setState({ mode: 'next' });
  vi.mocked(api.setPref).mockReset();
  vi.mocked(api.setPref).mockResolvedValue(undefined);
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
    root.render(<AutoAdvanceSetting />);
  });
}

const select = () => container.querySelector('select') as HTMLSelectElement;

async function choose(value: string) {
  await act(async () => {
    select().value = value;
    select().dispatchEvent(new Event('change', { bubbles: true }));
  });
}

describe('AutoAdvanceSetting', () => {
  it('offers next, previous and back to the list, with next as the default', async () => {
    await render();
    expect(container.textContent).toContain('After archiving or deleting');
    expect(Array.from(select().options).map((o) => o.textContent)).toEqual([
      'Open the next conversation',
      'Open the previous conversation',
      'Go back to the list',
    ]);
    expect(select().value).toBe('next');
  });

  it('stores the choice', async () => {
    await render();
    await choose('list');
    expect(api.setPref).toHaveBeenCalledWith(AFTER_LEAVE_PREF, 'list');
    expect(useAutoAdvanceStore.getState().mode).toBe('list');
  });

  it('shows and logs a failed save, keeping the old choice', async () => {
    vi.mocked(api.setPref).mockRejectedValue(new Error('disk full'));
    await render();
    await choose('previous');
    expect(container.textContent).toContain('disk full');
    expect(useAutoAdvanceStore.getState().mode).toBe('next');
    expect(useLogStore.getState().entries.some((l) => l.level === 'error' && l.message.includes('disk full'))).toBe(
      true,
    );
  });
});
