// The About tab: build version plus links to the website, the source code and
// the GitHub Sponsors page, each opened in the system browser.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const openExternal = vi.fn((_url: string) => Promise.resolve());
vi.mock('@tauri-apps/plugin-shell', () => ({ open: (url: string) => openExternal(url) }));
vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));
vi.mock('@/lib/api', () => ({ getBuildInfo: vi.fn() }));

import * as api from '@/lib/api';
import { AboutSettings } from './AboutSettings';

let container: HTMLDivElement;
let root: Root;
beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(api.getBuildInfo).mockResolvedValue({ version: '0.6.16', commit: 'abc1234', isRelease: true });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});
afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function renderAbout() {
  await act(async () => {
    root.render(<AboutSettings />);
  });
}

function button(label: string): HTMLButtonElement | undefined {
  return [...container.querySelectorAll('button')].find((b) => b.textContent?.includes(label));
}

describe('AboutSettings', () => {
  it('shows the build version', async () => {
    await renderAbout();
    expect(container.textContent).toContain('v0.6.16');
  });

  it.each([
    ['settings:about.website', 'https://getemailops.com/'],
    ['settings:about.sourceCode', 'https://github.com/emailops/emailops'],
    ['settings:about.sponsor', 'https://github.com/sponsors/emailops'],
  ])('%s opens %s in the browser', async (label, url) => {
    await renderAbout();
    const link = button(label);
    expect(link).toBeDefined();
    act(() => link?.click());
    expect(openExternal).toHaveBeenCalledWith(url);
  });
});
