// The "Sent with EmailOps" footer switch reads and writes the SQLite
// preference the send service checks when a message leaves.

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
import { useLogStore } from '@/stores/logStore';
import { SentFooterSetting } from './SentFooterSetting';

let container: HTMLDivElement;
let root: Root;

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  vi.mocked(api.getPref).mockReset().mockResolvedValue(null);
  vi.mocked(api.setPref).mockReset().mockResolvedValue(undefined);
  useLogStore.setState({ entries: [] });
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
    root.render(<SentFooterSetting />);
  });
}

function checkbox(): HTMLInputElement {
  const el = container.querySelector<HTMLInputElement>('[data-testid="sent-footer-setting"] input[type="checkbox"]');
  if (!el) throw new Error('footer switch not rendered');
  return el;
}

async function click(el: HTMLElement) {
  await act(async () => {
    el.click();
  });
}

describe('SentFooterSetting', () => {
  it('is on when nothing is stored', async () => {
    await render();
    expect(api.getPref).toHaveBeenCalledWith('compose.append_footer');
    expect(checkbox().checked).toBe(true);
  });

  it('is off when the stored value is false', async () => {
    vi.mocked(api.getPref).mockResolvedValue('false');
    await render();
    expect(checkbox().checked).toBe(false);
  });

  it('turning it off stores false', async () => {
    await render();
    await click(checkbox());
    expect(api.setPref).toHaveBeenCalledWith('compose.append_footer', 'false');
    expect(checkbox().checked).toBe(false);
  });

  it('a failed save keeps the switch as it was and shows the error', async () => {
    vi.mocked(api.setPref).mockRejectedValue(new Error('disk full'));
    await render();
    await click(checkbox());
    expect(checkbox().checked).toBe(true);
    expect(container.textContent).toContain('disk full');
    expect(useLogStore.getState().entries.some((l) => l.level === 'error')).toBe(true);
  });
});
