// Settings → Privacy → Database backup: how often the database is copied after
// launch, how many copies are kept, what they take on disk, and deleting them.

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
  getDbBackups: vi.fn(),
  setDbBackupSettings: vi.fn(),
  deleteDbBackups: vi.fn(),
}));

import * as api from '@/lib/api';
import { useLogStore } from '@/stores/logStore';
import { DatabaseBackupSection } from './DatabaseBackupSection';

const GIB = 1024 ** 3;

const threeDailyCopies: api.BackupStatus = {
  frequency: 'daily',
  keep: 3,
  count: 3,
  totalBytes: 12 * GIB,
  newestAt: 1_800_000_000,
};

const noCopies: api.BackupStatus = { ...threeDailyCopies, count: 0, totalBytes: 0, newestAt: null };

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

async function mount(status: api.BackupStatus) {
  vi.mocked(api.getDbBackups).mockResolvedValue(status);
  await act(async () => {
    root.render(<DatabaseBackupSection />);
  });
}

const byTestId = (testId: string) => container.querySelector<HTMLElement>(`[data-testid="${testId}"]`);

async function click(testId: string) {
  await act(async () => {
    byTestId(testId)?.click();
  });
}

const checked = (testId: string) => byTestId(testId)?.getAttribute('aria-checked');

describe('DatabaseBackupSection', () => {
  it('shows the stored frequency and copies to keep', async () => {
    await mount({ ...threeDailyCopies, frequency: 'weekly', keep: 2 });

    expect(checked('db-backup-frequency-weekly')).toBe('true');
    expect(checked('db-backup-frequency-daily')).toBe('false');
    expect(checked('db-backup-keep-2')).toBe('true');
  });

  it('shows how many copies there are, their size and the newest', async () => {
    await mount(threeDailyCopies);

    const summary = byTestId('db-backup-summary')?.textContent ?? '';
    expect(summary).toContain('settings:privacy.dbBackup.summary');
    expect(summary).toContain('3');
    expect(summary).toContain('12 GB');
    expect(summary).toContain('2027');
  });

  it('says there are no copies and offers nothing to delete', async () => {
    await mount(noCopies);

    expect(byTestId('db-backup-summary')?.textContent).toContain('settings:privacy.dbBackup.none');
    expect(byTestId('db-backup-delete')).toBeNull();
  });

  it('turning backups off stores it with the copies to keep', async () => {
    await mount(threeDailyCopies);
    vi.mocked(api.setDbBackupSettings).mockResolvedValue({ ...threeDailyCopies, frequency: 'off' });

    await click('db-backup-frequency-off');

    expect(api.setDbBackupSettings).toHaveBeenCalledWith('off', 3);
    expect(checked('db-backup-frequency-off')).toBe('true');
  });

  it('keeping fewer copies shows what is left on disk', async () => {
    await mount(threeDailyCopies);
    vi.mocked(api.setDbBackupSettings).mockResolvedValue({
      ...threeDailyCopies,
      keep: 1,
      count: 1,
      totalBytes: 4 * GIB,
    });

    await click('db-backup-keep-1');

    expect(api.setDbBackupSettings).toHaveBeenCalledWith('daily', 1);
    expect(byTestId('db-backup-summary')?.textContent).toContain('4 GB');
  });

  it('a failed save shows the error and logs it', async () => {
    await mount(threeDailyCopies);
    vi.mocked(api.setDbBackupSettings).mockRejectedValue(new Error('disk is read-only'));

    await click('db-backup-frequency-weekly');

    expect(container.querySelector('[role="alert"]')?.textContent).toContain('disk is read-only');
    expect(checked('db-backup-frequency-daily')).toBe('true');
    expect(useLogStore.getState().entries.some((e) => e.level === 'error')).toBe(true);
  });

  it('deleting asks first, and cancelling deletes nothing', async () => {
    await mount(threeDailyCopies);

    await click('db-backup-delete');
    expect(byTestId('db-backup-delete-confirm')).not.toBeNull();
    await click('db-backup-delete-cancel');

    expect(api.deleteDbBackups).not.toHaveBeenCalled();
    expect(byTestId('db-backup-delete-confirm')).toBeNull();
  });

  it('confirming the delete removes the copies and logs it', async () => {
    await mount(threeDailyCopies);
    vi.mocked(api.deleteDbBackups).mockResolvedValue(noCopies);

    await click('db-backup-delete');
    await click('db-backup-delete-confirm');

    expect(api.deleteDbBackups).toHaveBeenCalledTimes(1);
    expect(byTestId('db-backup-summary')?.textContent).toContain('settings:privacy.dbBackup.none');
    expect(useLogStore.getState().entries.some((e) => e.level === 'success')).toBe(true);
  });
});
