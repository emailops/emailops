// The one discreet line at the foot of the first onboarding step: where the
// data goes, and a link to put it somewhere else. No extra wizard screen.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, values?: Record<string, unknown>) => (values ? `${key} ${Object.values(values).join(' ')}` : key),
    i18n: { language: 'en' },
  }),
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
vi.mock('@/lib/api', () => ({
  getDataLocation: vi.fn(),
  pickDataFolder: vi.fn(),
  moveDataLocation: vi.fn(),
}));

import * as api from '@/lib/api';
import { DataLocationLine } from './DataLocationLine';

const atDefault: api.DataLocationInfo = {
  supported: true,
  path: '/home/user/.local/share/com.emailops.app',
  defaultPath: '/home/user/.local/share/com.emailops.app',
  isDefault: true,
  setByEnv: false,
  previousPath: null,
  sizeBytes: 90 * 1024 ** 2,
};

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  document.body.innerHTML = '';
  vi.clearAllMocks();
});

async function mount(info: api.DataLocationInfo | Error) {
  if (info instanceof Error) vi.mocked(api.getDataLocation).mockRejectedValue(info);
  else vi.mocked(api.getDataLocation).mockResolvedValue(info);
  await act(async () => {
    root.render(<DataLocationLine />);
  });
}

describe('DataLocationLine', () => {
  it('names the folder the data goes to', async () => {
    await mount(atDefault);
    expect(container.textContent).toContain(atDefault.path);
  });

  it('moves the data through the same pick and confirm flow as Settings', async () => {
    vi.mocked(api.pickDataFolder).mockResolvedValue({ path: '/media/ext/EmailOps', neededBytes: 1024 });
    vi.mocked(api.moveDataLocation).mockReturnValue(new Promise(() => {}));
    await mount(atDefault);

    await act(async () => {
      container.querySelector<HTMLButtonElement>('[data-testid="data-location-change"]')?.click();
    });
    await act(async () => {
      document.querySelector<HTMLButtonElement>('[data-testid="data-move-confirm"]')?.click();
    });

    expect(api.moveDataLocation).toHaveBeenCalledWith(false);
  });

  it('has no change link when EMAILOPS_DATA_DIR decides the folder', async () => {
    await mount({ ...atDefault, setByEnv: true });
    expect(container.querySelector('[data-testid="data-location-change"]')).toBeNull();
  });

  it('stays out of the way where the platform decides the folder', async () => {
    await mount({ ...atDefault, supported: false });
    expect(container.textContent).toBe('');
  });

  it('stays out of the way when the location cannot be read', async () => {
    await mount(new Error('ipc down'));
    expect(container.textContent).toBe('');
  });
});
