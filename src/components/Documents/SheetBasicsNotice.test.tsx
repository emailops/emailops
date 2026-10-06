// The notice above an EO Docs sheet that says sheets are basic for now, until
// the user closes it once.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({
  getPref: vi.fn(),
  setPref: vi.fn(),
  currentPlatform: vi.fn(() => ''),
}));

import { initI18n } from '@/i18n';
import * as api from '@/lib/api';
import { PREF_SHEET_BASICS_NOTICE_DISMISSED, SheetBasicsNotice } from './SheetBasicsNotice';

let container: HTMLDivElement;
let root: Root;

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(api.setPref).mockResolvedValue(undefined);
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
    root.render(<SheetBasicsNotice />);
  });
}

const notice = () => container.querySelector('[data-testid="sheet-basics-notice"]');

describe('SheetBasicsNotice', () => {
  it('says sheets are basic for now until it has been closed', async () => {
    vi.mocked(api.getPref).mockResolvedValue(null);
    await render();
    expect(notice()?.textContent).toMatch(/basic for now/);
  });

  it('closing it hides it and remembers that in the preferences', async () => {
    vi.mocked(api.getPref).mockResolvedValue(null);
    await render();
    await act(async () => {
      container.querySelector<HTMLButtonElement>('[data-testid="sheet-basics-notice-close"]')?.click();
    });
    expect(notice()).toBeNull();
    expect(api.setPref).toHaveBeenCalledWith(PREF_SHEET_BASICS_NOTICE_DISMISSED, 'true');
  });

  it('stays hidden once it was closed', async () => {
    vi.mocked(api.getPref).mockResolvedValue('true');
    await render();
    expect(notice()).toBeNull();
  });

  it('stays hidden while the preference is loading or cannot be read', async () => {
    vi.mocked(api.getPref).mockRejectedValue(new Error('db locked'));
    await render();
    expect(notice()).toBeNull();
  });
});
