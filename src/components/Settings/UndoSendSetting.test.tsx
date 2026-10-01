// The undo-send setting reads its window from the SQLite preferences.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@/lib/api')>()),
  getPref: vi.fn(async () => null),
  setPref: vi.fn(async () => {}),
}));

import { initI18n } from '@/i18n';
import * as api from '@/lib/api';
import { useOutboxStore } from '@/stores/outboxStore';
import { UndoSendSetting } from './UndoSendSetting';

let container: HTMLDivElement;
let root: Root;

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  useOutboxStore.setState({ undoDelaySecs: null });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function render() {
  await act(async () => {
    root.render(<UndoSendSetting />);
  });
}

describe('UndoSendSetting', () => {
  it('shows the stored window', async () => {
    vi.mocked(api.getPref).mockResolvedValue('20');
    await render();
    expect(api.getPref).toHaveBeenCalledWith('compose.undo_send_delay_secs');
    expect(container.textContent).toContain('20 seconds');
  });

  it('shows the 10 s default when nothing is stored', async () => {
    vi.mocked(api.getPref).mockResolvedValue(null);
    await render();
    expect(container.textContent).toContain('10 seconds');
  });

  it('saving a window stores it as a preference', async () => {
    await useOutboxStore.getState().setUndoDelay(0);
    expect(api.setPref).toHaveBeenCalledWith('compose.undo_send_delay_secs', '0');
    expect(useOutboxStore.getState().undoDelaySecs).toBe(0);
  });
});
