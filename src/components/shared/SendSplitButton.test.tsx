// The composer's Send control: Send, and the Schedule send menu (presets in
// local time, a custom time that must be in the future).

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { initI18n } from '@/i18n';
import { SendSplitButton } from './SendSplitButton';

let container: HTMLDivElement;
let root: Root;
const onSend = vi.fn();
const onSchedule = vi.fn();

// Wednesday 2026-10-07 10:20 local time.
const NOW = new Date(2026, 9, 7, 10, 20);

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(NOW);
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  onSend.mockReset();
  onSchedule.mockReset();
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

const q = (id: string) => container.querySelector<HTMLElement>(`[data-testid="${id}"]`);
const click = (id: string) => act(() => q(id)?.click());

function render(disabled = false) {
  act(() =>
    root.render(
      <SendSplitButton label="Send" onSend={onSend} onSchedule={onSchedule} disabled={disabled} testId="send" />,
    ),
  );
}

function setInput(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
  act(() => {
    setter?.call(input, value);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

describe('SendSplitButton', () => {
  it('sends with the main button and opens no menu', () => {
    render();
    click('send');
    expect(onSend).toHaveBeenCalledOnce();
    expect(q('send-schedule-menu')).toBeNull();
  });

  it('schedules a preset and closes the menu', () => {
    render();
    click('send-schedule');
    expect(q('send-schedule-menu')?.textContent).toMatch(/must be open/);
    click('send-preset-tomorrowAfternoon');
    expect(onSchedule).toHaveBeenCalledWith(new Date(2026, 9, 8, 13, 0));
    expect(q('send-schedule-menu')).toBeNull();
    expect(onSend).not.toHaveBeenCalled();
  });

  it('refuses a custom time in the past and accepts one in the future', () => {
    render();
    click('send-schedule');
    click('send-custom');
    const input = q('send-custom-input') as HTMLInputElement;
    setInput(input, '2026-10-07T09:00');
    click('send-custom-confirm');
    expect(q('send-custom-error')).not.toBeNull();
    expect(onSchedule).not.toHaveBeenCalled();
    setInput(input, '2026-10-09T17:45');
    click('send-custom-confirm');
    expect(onSchedule).toHaveBeenCalledWith(new Date(2026, 9, 9, 17, 45));
  });

  it('is fully disabled while the message cannot be sent', () => {
    render(true);
    expect((q('send') as HTMLButtonElement).disabled).toBe(true);
    expect((q('send-schedule') as HTMLButtonElement).disabled).toBe(true);
  });
});
