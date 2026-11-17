// Settings → Privacy → Backups: the hour copies are made at, the local data
// copy (and restoring one), and the full database copy (and deleting them).

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, values?: Record<string, unknown>) => (values ? `${key} ${Object.values(values).join(' ')}` : key),
    i18n: { language: 'en' },
  }),
}));
vi.mock('@/lib/api', () => ({
  currentPlatform: () => 'macos',
  getBackups: vi.fn(),
  setBackupSettings: vi.fn(),
  deleteFullBackups: vi.fn(),
  restoreLocalBackup: vi.fn(),
}));

import * as api from '@/lib/api';
import { useLogStore } from '@/stores/logStore';
import { BackupsSection } from './BackupsSection';

const MIB = 1024 ** 2;
const GIB = 1024 ** 3;
const DAY = 86_400;
const NEWEST = 1_800_000_000;

const status: api.BackupStatus = {
  hour: 3,
  local: 'daily',
  full: 'daily',
  keep: 3,
  localCopies: [
    { name: 'local-20270115_080000.db', takenAt: NEWEST, bytes: 3 * MIB },
    { name: 'local-20270114_080000.db', takenAt: NEWEST - DAY, bytes: 3 * MIB },
  ],
  fullCopies: [
    { name: 'emailops-20270115_080500.db', takenAt: NEWEST, bytes: 6 * GIB },
    { name: 'emailops-20270114_080500.db', takenAt: NEWEST - DAY, bytes: 6 * GIB },
  ],
};

const noCopies: api.BackupStatus = { ...status, localCopies: [], fullCopies: [] };

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  useLogStore.setState({ entries: [] });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.clearAllMocks();
});

async function mount(shown: api.BackupStatus) {
  vi.mocked(api.getBackups).mockResolvedValue(shown);
  await act(async () => {
    root.render(<BackupsSection />);
  });
}

const byTestId = <T extends HTMLElement = HTMLElement>(testId: string) =>
  container.querySelector<T>(`[data-testid="${testId}"]`);

async function click(testId: string) {
  await act(async () => {
    byTestId(testId)?.click();
  });
}

async function choose(testId: string, value: string) {
  const select = byTestId<HTMLSelectElement>(testId);
  if (!select) throw new Error(`no ${testId}`);
  await act(async () => {
    select.value = value;
    select.dispatchEvent(new Event('change', { bubbles: true }));
  });
}

const checked = (testId: string) => byTestId(testId)?.getAttribute('aria-checked');

describe('BackupsSection', () => {
  it('shows the stored hour, how often each copy is made and the copies to keep', async () => {
    await mount({ ...status, hour: 22, local: 'weekly', full: 'off', keep: 2 });

    expect(byTestId<HTMLSelectElement>('backup-hour')?.value).toBe('22');
    expect(checked('backup-local-weekly')).toBe('true');
    expect(checked('backup-full-off')).toBe('true');
    expect(checked('backup-full-daily')).toBe('false');
    expect(checked('backup-keep-2')).toBe('true');
  });

  it('offers every hour of the day', async () => {
    await mount(status);

    expect(byTestId<HTMLSelectElement>('backup-hour')?.options.length).toBe(24);
  });

  it('shows how many copies of each kind there are, their size and the newest', async () => {
    await mount(status);

    const local = byTestId('backup-local-summary')?.textContent ?? '';
    const full = byTestId('backup-full-summary')?.textContent ?? '';
    expect(local).toContain('settings:privacy.backups.summary 2 6 MB');
    expect(full).toContain('settings:privacy.backups.summary 2 12 GB');
    expect(full).toContain('2027');
  });

  it('says there are no copies and offers nothing to delete or restore', async () => {
    await mount(noCopies);

    expect(byTestId('backup-local-summary')?.textContent).toContain('settings:privacy.backups.none');
    expect(byTestId('backup-full-summary')?.textContent).toContain('settings:privacy.backups.none');
    expect(byTestId('backup-delete')).toBeNull();
    expect(byTestId('backup-restore')).toBeNull();
  });

  it('changing the hour stores it with the rest of the settings', async () => {
    await mount(status);
    vi.mocked(api.setBackupSettings).mockResolvedValue({ ...status, hour: 13 });

    await choose('backup-hour', '13');

    expect(api.setBackupSettings).toHaveBeenCalledWith({ hour: 13, local: 'daily', full: 'daily', keep: 3 });
    expect(byTestId<HTMLSelectElement>('backup-hour')?.value).toBe('13');
  });

  it('turning the full copy off keeps the local copy as it was', async () => {
    await mount(status);
    vi.mocked(api.setBackupSettings).mockResolvedValue({ ...status, full: 'off' });

    await click('backup-full-off');

    expect(api.setBackupSettings).toHaveBeenCalledWith({ hour: 3, local: 'daily', full: 'off', keep: 3 });
    expect(checked('backup-full-off')).toBe('true');
    expect(checked('backup-local-daily')).toBe('true');
  });

  it('keeping fewer full copies shows what is left on disk', async () => {
    await mount(status);
    vi.mocked(api.setBackupSettings).mockResolvedValue({
      ...status,
      keep: 1,
      fullCopies: status.fullCopies.slice(0, 1),
    });

    await click('backup-keep-1');

    expect(api.setBackupSettings).toHaveBeenCalledWith({ hour: 3, local: 'daily', full: 'daily', keep: 1 });
    expect(byTestId('backup-full-summary')?.textContent).toContain('6 GB');
  });

  it('a failed save shows the error and logs it', async () => {
    await mount(status);
    vi.mocked(api.setBackupSettings).mockRejectedValue(new Error('disk is read-only'));

    await click('backup-local-weekly');

    expect(container.querySelector('[role="alert"]')?.textContent).toContain('disk is read-only');
    expect(checked('backup-local-daily')).toBe('true');
    expect(useLogStore.getState().entries.some((e) => e.level === 'error')).toBe(true);
  });

  it('deleting the full copies asks first, and cancelling deletes nothing', async () => {
    await mount(status);

    await click('backup-delete');
    expect(byTestId('backup-delete-confirm')).not.toBeNull();
    await click('backup-delete-cancel');

    expect(api.deleteFullBackups).not.toHaveBeenCalled();
    expect(byTestId('backup-delete-confirm')).toBeNull();
  });

  it('confirming the delete removes the full copies and logs it', async () => {
    await mount(status);
    vi.mocked(api.deleteFullBackups).mockResolvedValue({ ...status, fullCopies: [] });

    await click('backup-delete');
    await click('backup-delete-confirm');

    expect(api.deleteFullBackups).toHaveBeenCalledTimes(1);
    expect(byTestId('backup-full-summary')?.textContent).toContain('settings:privacy.backups.none');
    expect(useLogStore.getState().entries.some((e) => e.level === 'success')).toBe(true);
  });

  it('restoring asks first with the newest copy chosen, and cancelling restores nothing', async () => {
    await mount(status);

    await click('backup-restore');
    expect(byTestId<HTMLSelectElement>('backup-restore-copy')?.value).toBe('local-20270115_080000.db');
    await click('backup-restore-cancel');

    expect(api.restoreLocalBackup).not.toHaveBeenCalled();
    expect(byTestId('backup-restore-confirm')).toBeNull();
  });

  it('confirming the restore restores the chosen copy', async () => {
    await mount(status);
    vi.mocked(api.restoreLocalBackup).mockResolvedValue(undefined);

    await click('backup-restore');
    await choose('backup-restore-copy', 'local-20270114_080000.db');
    await click('backup-restore-confirm');

    expect(api.restoreLocalBackup).toHaveBeenCalledWith('local-20270114_080000.db');
  });

  it('a failed restore shows the error and logs it', async () => {
    await mount(status);
    vi.mocked(api.restoreLocalBackup).mockRejectedValue(new Error('made by a newer version'));

    await click('backup-restore');
    await click('backup-restore-confirm');

    expect(container.querySelector('[role="alert"]')?.textContent).toContain('made by a newer version');
    expect(useLogStore.getState().entries.some((e) => e.level === 'error')).toBe(true);
  });
});
