// Meeting-reminder OS notifications leave the meeting title out unless the
// user opts in: they show on the lock screen, outside the app's lock.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({ useTranslation: () => ({ t: (key: string) => key }) }));

const prefs = vi.hoisted(() => ({ stored: {} as Record<string, string | null> }));
vi.mock('@/lib/api', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/lib/api')>()),
  getPref: vi.fn((key: string) => Promise.resolve(prefs.stored[key] ?? null)),
  setPref: vi.fn(() => Promise.resolve()),
  getCalendars: vi.fn(() => Promise.resolve([])),
  syncCalendarNow: vi.fn(() => Promise.resolve(0)),
}));

import * as api from '@/lib/api';
import { CalendarSettings } from './CalendarSettings';

describe('CalendarSettings — meeting title in notifications', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    prefs.stored = {};
    vi.mocked(api.setPref).mockClear();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
  });

  const titleSwitch = () => {
    const found = container.querySelector<HTMLButtonElement>(
      'button[role="switch"][aria-label="settings:calendar.showTitleLabel"]',
    );
    if (!found) throw new Error('show-title switch not rendered');
    return found;
  };

  it('is off when the preference was never set', async () => {
    await act(async () => root.render(<CalendarSettings />));
    expect(titleSwitch().getAttribute('aria-checked')).toBe('false');
  });

  it('reflects a stored opt-in', async () => {
    prefs.stored.calendar_notification_show_title = 'true';
    await act(async () => root.render(<CalendarSettings />));
    expect(titleSwitch().getAttribute('aria-checked')).toBe('true');
  });

  it('saves the opt-in when switched on', async () => {
    await act(async () => root.render(<CalendarSettings />));
    await act(async () => titleSwitch().click());
    expect(api.setPref).toHaveBeenCalledWith('calendar_notification_show_title', 'true');
    expect(titleSwitch().getAttribute('aria-checked')).toBe('true');
  });
});
