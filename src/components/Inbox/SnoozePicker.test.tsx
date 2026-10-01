// The snooze picker: presets for "now", and a custom date & time that must be
// in the future.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { initI18n } from '@/i18n';
import { SnoozeMenuButton, SnoozeOptions } from './SnoozePicker';

let container: HTMLDivElement;
let root: Root;
const onPick = vi.fn();

// Wednesday 2026-10-07 10:20 local time.
const NOW = new Date(2026, 9, 7, 10, 20);
const unix = (d: Date) => Math.floor(d.getTime() / 1000);

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(NOW);
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  onPick.mockReset();
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

const q = (id: string) => container.querySelector<HTMLElement>(`[data-testid="${id}"]`);

function setInput(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
  setter?.call(input, value);
  input.dispatchEvent(new Event('input', { bubbles: true }));
}

describe('SnoozeOptions', () => {
  it('lists the presets for now and picks one as unix seconds', () => {
    act(() => root.render(<SnoozeOptions onPick={onPick} />));
    expect(q('snooze-preset-laterToday')?.textContent).toContain('Later today');
    expect(q('snooze-preset-thisWeekend')?.textContent).toContain('This weekend');
    act(() => q('snooze-preset-tomorrow')?.click());
    expect(onPick).toHaveBeenCalledWith(unix(new Date(2026, 9, 8, 8, 0)));
  });

  it('refuses a custom time in the past and accepts one in the future', () => {
    act(() => root.render(<SnoozeOptions onPick={onPick} />));
    act(() => q('snooze-custom')?.click());
    const input = q('snooze-custom-input') as HTMLInputElement;

    act(() => setInput(input, '2026-10-07T09:00'));
    act(() => q('snooze-custom-confirm')?.click());
    expect(onPick).not.toHaveBeenCalled();
    expect(q('snooze-custom-error')?.textContent).toBe('Choose a time in the future');

    act(() => setInput(input, '2026-10-09T17:45'));
    act(() => q('snooze-custom-confirm')?.click());
    expect(onPick).toHaveBeenCalledWith(unix(new Date(2026, 9, 9, 17, 45)));
  });
});

describe('SnoozeMenuButton', () => {
  it('opens the options and closes once a time is picked', () => {
    act(() => root.render(<SnoozeMenuButton testId="snooze" onPick={onPick} />));
    expect(q('snooze-preset-tomorrow')).toBeNull();
    act(() => q('snooze')?.click());
    act(() => q('snooze-preset-nextWeek')?.click());
    expect(onPick).toHaveBeenCalledWith(unix(new Date(2026, 9, 12, 8, 0)));
    expect(q('snooze-preset-nextWeek')).toBeNull();
  });

  it('opens when its open signal changes (the b shortcut), not on mount', () => {
    act(() => root.render(<SnoozeMenuButton testId="snooze" onPick={onPick} openSignal={3} />));
    expect(q('snooze-preset-tomorrow')).toBeNull();
    act(() => root.render(<SnoozeMenuButton testId="snooze" onPick={onPick} openSignal={4} />));
    expect(q('snooze-preset-tomorrow')).not.toBeNull();
  });
});
