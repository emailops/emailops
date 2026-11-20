// "Import .ics": pick a file, see what each event would do, import it into the
// chosen account's primary calendar and read the summary.

import { act, StrictMode } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, unknown>) =>
      opts && typeof opts.count === 'number'
        ? `${key}|${opts.count}`
        : opts && typeof opts.message === 'string'
          ? `${key}|${opts.message}`
          : key,
    i18n: { language: 'en' },
  }),
}));
vi.mock('@/hooks/useFormatters', () => ({
  useFormatters: () => ({ time: () => '09:00', date: () => 'Tue', dateTime: () => 'Tue 09:00' }),
}));
const api = vi.hoisted(() => ({
  previewIcsImport: vi.fn(),
  importIcs: vi.fn(),
  currentPlatform: vi.fn(() => ''),
}));
vi.mock('@/lib/api', () => api);
const dialog = vi.hoisted(() => ({ open: vi.fn() }));
vi.mock('@tauri-apps/plugin-dialog', () => dialog);

import type { CreateAccountChoice } from '@/lib/calendarAccounts';
import { useLogStore } from '@/stores/logStore';
import type { Account, IcsImportPreview } from '@/types';
import { ImportIcsDialog } from './ImportIcsDialog';

function account(id: string): Account {
  return {
    id,
    provider: 'gmail',
    email: `${id}@mail.example`,
    name: '',
    createdAt: 0,
    sortOrder: 0,
    enabled: true,
    syncFromTimestamp: null,
    color: 'blue',
  };
}

const choices: CreateAccountChoice[] = [
  { account: account('work'), plan: { kind: 'dialog' } },
  { account: account('home'), plan: { kind: 'dialog' } },
];

function item(uid: string, status: IcsImportPreview['items'][number]['status'], extra = {}) {
  return {
    uid,
    title: `Event ${uid}`,
    startTime: 1_800_000_000,
    endTime: 1_800_003_600,
    isAllDay: false,
    recurrence: null,
    status,
    reason: null,
    reasonDetail: null,
    ...extra,
  };
}

const preview: IcsImportPreview = {
  fileName: 'agenda.ics',
  items: [
    item('a', 'importable'),
    item('b', 'alreadyInCalendar'),
    item('c', 'unsupported', { reason: 'outlookRecurrence', reasonDetail: 'FREQ=HOURLY' }),
  ],
};

let container: HTMLDivElement;
let root: Root;
const props = { onClose: vi.fn(), onImported: vi.fn(), onAuthError: vi.fn() };

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  dialog.open.mockResolvedValue('/tmp/agenda.ics');
  api.previewIcsImport.mockResolvedValue(preview);
  useLogStore.setState({ entries: [] } as never);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.clearAllMocks();
});

async function render(list: CreateAccountChoice[] = choices, initialAccountId = 'work') {
  await act(async () => {
    root.render(<ImportIcsDialog choices={list} initialAccountId={initialAccountId} {...props} />);
  });
}

function button(label: string): HTMLButtonElement {
  const found = Array.from(document.querySelectorAll('button')).find((b) => b.textContent?.startsWith(label));
  if (!found) throw new Error(`button ${label} not rendered`);
  return found;
}

function pickAccount(email: string) {
  const trigger = document.querySelector<HTMLElement>('[aria-label="calendar:import.account"]');
  if (!trigger) throw new Error('account selector not rendered');
  act(() => trigger.click());
  const option = Array.from(document.querySelectorAll('[role="option"]')).find((o) => o.textContent === email);
  if (!option) throw new Error(`option ${email} not rendered`);
  return act(async () => (option as HTMLElement).click());
}

describe('ImportIcsDialog', () => {
  it('opens_the_picker_once_on_mount', async () => {
    await render();
    expect(dialog.open).toHaveBeenCalledTimes(1);
  });

  it('opens_the_picker_once_under_strict_mode', async () => {
    await act(async () => {
      root.render(
        <StrictMode>
          <ImportIcsDialog choices={choices} initialAccountId="work" {...props} />
        </StrictMode>,
      );
    });
    expect(dialog.open).toHaveBeenCalledTimes(1);
  });

  it('keeps_the_choose_button_and_skips_the_preview_when_the_picker_is_cancelled', async () => {
    dialog.open.mockResolvedValue(null);
    await render();
    expect(api.previewIcsImport).not.toHaveBeenCalled();
    await act(async () => button('calendar:import.choose').click());
    expect(dialog.open).toHaveBeenCalledTimes(2);
  });

  it('shows_each_item_with_its_status', async () => {
    await render();
    const text = document.body.textContent ?? '';
    expect(text).toContain('Event a');
    expect(text).toContain('calendar:import.status.importable');
    expect(text).toContain('Event b');
    expect(text).toContain('calendar:import.status.alreadyInCalendar');
    expect(text).toContain('Event c');
    expect(text).toContain('calendar:import.reason.outlookRecurrence (FREQ=HOURLY)');
    expect(button('calendar:import.importN').textContent).toBe('calendar:import.importN|1');
    expect(api.previewIcsImport).toHaveBeenCalledWith('work', '/tmp/agenda.ics', expect.any(String));
  });

  it('changing_account_requests_a_new_preview', async () => {
    await render();
    await pickAccount('home@mail.example');
    expect(api.previewIcsImport).toHaveBeenCalledTimes(2);
    expect(api.previewIcsImport).toHaveBeenLastCalledWith('home', '/tmp/agenda.ics', expect.any(String));
  });

  it('import_button_disabled_when_nothing_importable', async () => {
    api.previewIcsImport.mockResolvedValue({ fileName: 'x.ics', items: [item('b', 'alreadyInCalendar')] });
    await render();
    expect(button('calendar:import.importN').disabled).toBe(true);
  });

  it('shows_the_summary_after_import', async () => {
    api.importIcs.mockResolvedValue({
      imported: 1,
      alreadyPresent: 1,
      unsupported: 1,
      failed: [{ title: 'Event d', reason: 'provider said no' }],
    });
    await render();
    await act(async () => button('calendar:import.importN').click());
    expect(api.importIcs).toHaveBeenCalledWith('work', '/tmp/agenda.ics', expect.any(String));
    const text = document.body.textContent ?? '';
    expect(text).toContain('calendar:import.summary.imported|1');
    expect(text).toContain('Event d');
    expect(text).toContain('provider said no');
    expect(props.onImported).toHaveBeenCalledWith('work');
    expect(button('common:actions.close')).toBeTruthy();
  });

  it('pins_a_failed_preview_at_the_top_and_logs_it_without_the_path', async () => {
    api.previewIcsImport.mockRejectedValue(new Error('file too large'));
    await render();
    const alert = document.querySelector('[role="alert"]');
    expect(alert?.textContent).toContain('file too large');
    expect(useLogStore.getState().entries.some((l) => l.level === 'error' && !l.message.includes('/tmp/'))).toBe(true);
  });

  it('hands_an_auth_error_to_the_view', async () => {
    api.previewIcsImport.mockRejectedValue(new Error('Authentication required'));
    await render();
    expect(props.onAuthError).toHaveBeenCalledWith('work');
  });
});
