// The sidebar's "star us on GitHub" link. Most people install EmailOps
// without ever landing on the repository, so this is the only place in the
// product that asks.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

vi.mock('@tauri-apps/plugin-shell', () => ({
  open: vi.fn(),
}));

import { open as openExternal } from '@tauri-apps/plugin-shell';
import { REPO_URL, StarOnGitHub } from './StarOnGitHub';

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.clearAllMocks();
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
    root.render(<StarOnGitHub />);
  });
}

function click() {
  const button = container.querySelector('button');
  if (!button) throw new Error('no button rendered');
  act(() => {
    button.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  });
}

describe('StarOnGitHub', () => {
  it('labels itself from the sidebar namespace', async () => {
    vi.mocked(openExternal).mockResolvedValue(undefined);
    await render();
    expect(container.textContent).toContain('sidebar:starOnGitHub');
  });

  // The stargazers URL, not the repo root: it lands on the repo with the star
  // action already in focus, which is the whole point of the link.
  it('opens the repository in the browser when clicked', async () => {
    vi.mocked(openExternal).mockResolvedValue(undefined);
    await render();
    click();
    expect(openExternal).toHaveBeenCalledWith(REPO_URL);
  });

  // A browser that refuses to open is not worth an error state in the
  // sidebar, but it must not surface as an unhandled rejection either.
  it('swallows a failure to open the browser', async () => {
    vi.mocked(openExternal).mockRejectedValue(new Error('no browser'));
    await render();
    expect(() => click()).not.toThrow();
  });
});
