// The email frame may run only the trusted bridge script. Every link click is
// routed through the bridge to the parent (safe-scheme check + confirmation),
// so the frame never needs to open a window of its own.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));
vi.mock('@tauri-apps/plugin-shell', () => ({ open: vi.fn(async () => {}) }));

import { EmailHtmlFrame } from './EmailHtmlFrame';

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe('EmailHtmlFrame sandbox', () => {
  it('allows scripts and nothing else', () => {
    act(() => root.render(<EmailHtmlFrame html="<p>hi</p>" allowRemoteContent={false} />));
    expect(container.querySelector('iframe')?.getAttribute('sandbox')).toBe('allow-scripts');
  });
});
