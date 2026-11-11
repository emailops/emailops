// Settings → Privacy → Data location: shows where the data is and moves it
// through the native folder picker, a confirmation and a restart.

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
  dismissPreviousDataLocation: vi.fn(async () => {}),
  pickDataFolder: vi.fn(),
  moveDataLocation: vi.fn(),
}));

import * as api from '@/lib/api';
import { DataLocationSection } from './DataLocationSection';

const atDefault: api.DataLocationInfo = {
  supported: true,
  path: '/home/user/.local/share/com.emailops.app',
  defaultPath: '/home/user/.local/share/com.emailops.app',
  isDefault: true,
  setByEnv: false,
  previousPath: null,
  sizeBytes: 3 * 1024 ** 3,
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

async function mount(info: api.DataLocationInfo) {
  vi.mocked(api.getDataLocation).mockResolvedValue(info);
  await act(async () => {
    root.render(<DataLocationSection />);
  });
}

const button = (testId: string) => document.querySelector<HTMLButtonElement>(`[data-testid="${testId}"]`);

async function click(testId: string) {
  await act(async () => {
    button(testId)?.click();
  });
}

describe('DataLocationSection', () => {
  it('shows the current folder and its size', async () => {
    await mount(atDefault);
    expect(container.textContent).toContain(atDefault.path);
    expect(container.textContent).toContain('3 GB');
  });

  it('asks to confirm the picked folder before moving anything', async () => {
    vi.mocked(api.pickDataFolder).mockResolvedValue({ path: '/media/ext/EmailOps', neededBytes: 1024 });
    await mount(atDefault);

    await click('data-location-change');

    expect(document.body.textContent).toContain('/media/ext/EmailOps');
    expect(api.moveDataLocation).not.toHaveBeenCalled();
  });

  it('warns before moving that the data is not encrypted by EmailOps and the old copy stays', async () => {
    vi.mocked(api.pickDataFolder).mockResolvedValue({ path: '/media/ext/EmailOps', neededBytes: 1024 });
    await mount(atDefault);

    await click('data-location-change');

    expect(document.body.textContent).toContain('settings:privacy.dataLocation.confirmEncryption');
    expect(document.body.textContent).toContain('settings:privacy.dataLocation.confirmOldCopy');
  });

  it('moves to the picked folder once confirmed', async () => {
    vi.mocked(api.pickDataFolder).mockResolvedValue({ path: '/media/ext/EmailOps', neededBytes: 1024 });
    vi.mocked(api.moveDataLocation).mockReturnValue(new Promise(() => {}));
    await mount(atDefault);

    await click('data-location-change');
    await click('data-move-confirm');

    expect(api.moveDataLocation).toHaveBeenCalledWith(false);
  });

  it('does nothing when the folder dialog is cancelled', async () => {
    vi.mocked(api.pickDataFolder).mockResolvedValue(null);
    await mount(atDefault);

    await click('data-location-change');

    expect(button('data-move-confirm')).toBeNull();
  });

  it('says why a picked folder cannot be used', async () => {
    vi.mocked(api.pickDataFolder).mockRejectedValue(new Error('has files'));
    await mount(atDefault);

    await click('data-location-change');

    expect(container.querySelector('[role="alert"]')?.textContent).toContain('has files');
  });

  it('keeps the confirmation open with the error when the move fails', async () => {
    vi.mocked(api.pickDataFolder).mockResolvedValue({ path: '/media/ext/EmailOps', neededBytes: 1024 });
    vi.mocked(api.moveDataLocation).mockRejectedValue(new Error('disk full'));
    await mount(atDefault);

    await click('data-location-change');
    await click('data-move-confirm');

    expect(document.body.querySelector('[role="alert"]')?.textContent).toContain('disk full');
    expect(button('data-move-confirm')?.disabled).toBe(false);
  });

  it('offers to go back to the default folder when the data is elsewhere', async () => {
    vi.mocked(api.moveDataLocation).mockReturnValue(new Promise(() => {}));
    await mount({ ...atDefault, path: '/media/ext/EmailOps', isDefault: false });

    await click('data-location-restore');
    await click('data-move-confirm');

    expect(api.moveDataLocation).toHaveBeenCalledWith(true);
  });

  it('has no restore button at the default folder', async () => {
    await mount(atDefault);
    expect(button('data-location-restore')).toBeNull();
  });

  it('cannot move a folder set by EMAILOPS_DATA_DIR', async () => {
    await mount({ ...atDefault, setByEnv: true });
    expect(button('data-location-change')?.disabled).toBe(true);
    expect(container.textContent).toContain('settings:privacy.dataLocation.setByEnv');
  });

  it('points at the old copy until the user dismisses the note', async () => {
    await mount({ ...atDefault, previousPath: '/old/place' });
    expect(container.textContent).toContain('/old/place');

    await click('data-location-dismiss-previous');

    expect(api.dismissPreviousDataLocation).toHaveBeenCalled();
    expect(container.textContent).not.toContain('/old/place');
  });

  it('renders nothing where the platform decides the data folder', async () => {
    await mount({ ...atDefault, supported: false });
    expect(container.textContent).toBe('');
  });
});
