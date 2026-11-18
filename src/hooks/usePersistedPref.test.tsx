// usePersistedPref: what a plain-string preference such as `sidebar_density`
// relies on — a rejected stored value keeps the default without rewriting the
// row, a valid one is adopted, and a failed save reaches the output panel.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({
  getPref: vi.fn(),
  setPref: vi.fn(async () => {}),
}));

import * as api from '@/lib/api';
import { isSidebarDensity, type SidebarDensity } from '@/lib/sidebarDensity';
import { useLogStore } from '@/stores/logStore';
import { usePersistedPref } from './usePersistedPref';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let value: SidebarDensity;
let setValue: (v: SidebarDensity) => void;
function Probe() {
  const [v, set] = usePersistedPref<SidebarDensity>('sidebar_density', 'comfortable', {
    parse: (raw) => (isSidebarDensity(raw) ? raw : null),
    serialize: (v) => v,
  });
  value = v;
  setValue = set;
  return null;
}

let container: HTMLDivElement;
let root: Root;

async function mount() {
  await act(async () => {
    root.render(<Probe />);
  });
}

beforeEach(() => {
  vi.mocked(api.getPref).mockReset();
  vi.mocked(api.setPref).mockReset().mockResolvedValue(undefined);
  useLogStore.setState({ entries: [] });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe('usePersistedPref with a plain-string enum', () => {
  it('keeps the default and does not rewrite the row when the stored value is unknown', async () => {
    vi.mocked(api.getPref).mockResolvedValue('spacious');

    await mount();

    expect(value).toBe('comfortable');
    expect(api.setPref).not.toHaveBeenCalled();
  });

  it('adopts a valid stored value', async () => {
    vi.mocked(api.getPref).mockResolvedValue('compact');

    await mount();

    expect(value).toBe('compact');
  });

  it('logs a failed save as a system error', async () => {
    vi.mocked(api.getPref).mockResolvedValue(null);
    vi.mocked(api.setPref).mockRejectedValue(new Error('disk full'));
    await mount();

    await act(async () => {
      setValue('compact');
    });

    const { entries } = useLogStore.getState();
    const entry = entries[entries.length - 1];
    expect(entry?.level).toBe('error');
    expect(entry?.source).toBe('system');
    expect(entry?.message).toMatch(/^Failed to save pref "sidebar_density"/);
  });
});
