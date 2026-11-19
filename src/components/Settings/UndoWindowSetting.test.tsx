// Settings → Appearance: how long archive and delete wait for an Undo, stored
// in the SQLite preferences like Undo send.

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
import { UNDO_WINDOW_PREF } from '@/lib/undoWindow';
import { useLogStore } from '@/stores/logStore';
import { useUndoWindowStore } from '@/stores/undoWindowStore';
import { UndoWindowSetting } from './UndoWindowSetting';

let container: HTMLDivElement;
let root: Root;

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  useUndoWindowStore.setState({ secs: 6 });
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
    root.render(<UndoWindowSetting />);
  });
}

const select = () => container.querySelector('select') as HTMLSelectElement;

async function choose(value: string) {
  await act(async () => {
    select().value = value;
    select().dispatchEvent(new Event('change', { bubbles: true }));
  });
}

describe('UndoWindowSetting', () => {
  it('offers off and 6 to 30 seconds, with 6 seconds as the default', async () => {
    await render();
    expect(container.textContent).toContain('Undo archive and delete');
    expect(Array.from(select().options).map((o) => o.textContent)).toEqual([
      'Off',
      '6 seconds',
      '10 seconds',
      '20 seconds',
      '30 seconds',
    ]);
    expect(select().value).toBe('6');
  });

  it('shows the stored window', async () => {
    useUndoWindowStore.setState({ secs: 20 });
    await render();
    expect(select().value).toBe('20');
  });

  it('stores the choice, off included', async () => {
    await render();
    await choose('0');
    expect(api.setPref).toHaveBeenCalledWith(UNDO_WINDOW_PREF, '0');
    expect(useUndoWindowStore.getState().secs).toBe(0);
    expect(select().value).toBe('0');
  });

  it('shows and logs a failed save, keeping the old choice', async () => {
    vi.mocked(api.setPref).mockRejectedValue(new Error('disk full'));
    await render();
    await choose('30');
    expect(container.textContent).toContain('disk full');
    expect(useUndoWindowStore.getState().secs).toBe(6);
    expect(useLogStore.getState().entries.some((l) => l.level === 'error' && l.message.includes('disk full'))).toBe(
      true,
    );
  });
});
