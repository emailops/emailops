// With remote content off, the frame itself must refuse remote image loads:
// the sanitizer cannot rewrite every CSS url() (a <style> block's background
// images are left as written), and the app CSP allows https: images. A
// per-frame CSP limited to inline sources closes that. With remote content on,
// no extra policy is added.

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

function frameCsp(allowRemoteContent: boolean): string | null {
  act(() => root.render(<EmailHtmlFrame html="<p>hi</p>" allowRemoteContent={allowRemoteContent} />));
  const srcdoc = container.querySelector('iframe')?.getAttribute('srcdoc') ?? '';
  const doc = new DOMParser().parseFromString(srcdoc, 'text/html');
  return doc.querySelector('meta[http-equiv="Content-Security-Policy"]')?.getAttribute('content') ?? null;
}

describe('EmailHtmlFrame remote content policy', () => {
  it('limits images to inline sources when remote content is off', () => {
    const csp = frameCsp(false);
    expect(csp).not.toBeNull();
    const imgSrc = csp
      ?.split(';')
      .map((d) => d.trim())
      .find((d) => d.startsWith('img-src'));
    expect(imgSrc).toBeDefined();
    expect(imgSrc).toContain('data:');
    expect(imgSrc).not.toContain('https:');
    expect(imgSrc).not.toContain('http:');
    expect(imgSrc).not.toContain('*');
  });

  it('does not restrict the frame when remote content is allowed', () => {
    expect(frameCsp(true)).toBeNull();
  });
});
