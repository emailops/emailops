// On a phone the docked output panel is not rendered at all, so this view is
// the only place backend progress and errors are visible — and the only way to
// get them off the device as text. A crash or an empty render here is not a
// cosmetic bug; it is the difference between a diagnosable problem and a
// silent one.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string) => key,
    i18n: { language: 'en' },
  }),
}));

vi.mock('@/hooks/useFormatters', () => ({
  useFormatters: () => ({ time: (seconds: number) => new Date(seconds * 1000).toISOString().slice(11, 19) }),
}));

import { useLogStore } from '@/stores/logStore';
import { LogView } from './LogView';

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  useLogStore.setState({ entries: [], nextId: 1 });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

async function mount() {
  await act(async () => {
    root.render(<LogView />);
  });
}

function buttonWithText(text: string): HTMLButtonElement | null {
  return (Array.from(container.querySelectorAll('button')).find((b) => b.textContent?.includes(text)) ??
    null) as HTMLButtonElement | null;
}

describe('LogView', () => {
  it('lists the entries that have been logged', async () => {
    useLogStore.getState().addLog('info', 'sync', 'Checking for new mail');
    useLogStore.getState().addLog('error', 'ai', 'Model failed to load');
    await mount();

    expect(container.textContent).toContain('Checking for new mail');
    expect(container.textContent).toContain('Model failed to load');
  });

  it('copies the visible entries as text', async () => {
    // The whole point of the view: getting a log off the phone and into a bug
    // report without retyping it from a screenshot.
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });
    useLogStore.getState().addLog('info', 'sync', 'Checking for new mail');
    await mount();

    await act(async () => {
      buttonWithText('dashboard:log.copy')?.click();
    });

    expect(writeText).toHaveBeenCalledOnce();
    expect(writeText.mock.calls[0][0]).toContain('Checking for new mail');
  });

  it('reports a failed copy instead of pretending it worked', async () => {
    // A copy that silently did nothing is indistinguishable from one that
    // worked until the paste comes up empty.
    Object.assign(navigator, { clipboard: { writeText: vi.fn().mockRejectedValue(new Error('denied')) } });
    useLogStore.getState().addLog('info', 'sync', 'Checking for new mail');
    await mount();

    await act(async () => {
      buttonWithText('dashboard:log.copy')?.click();
    });

    const errors = useLogStore.getState().entries.filter((e) => e.level === 'error');
    expect(errors).toHaveLength(1);
    expect(errors[0].message).toContain('denied');
  });

  it('offers nothing to copy when there is nothing logged', async () => {
    await mount();
    expect(buttonWithText('dashboard:log.copy')?.disabled).toBe(true);
  });

  it('clears the entries on request', async () => {
    useLogStore.getState().addLog('info', 'sync', 'Checking for new mail');
    await mount();

    await act(async () => {
      buttonWithText('dashboard:log.clearLogs')?.click();
    });

    expect(useLogStore.getState().entries).toHaveLength(0);
  });
});
