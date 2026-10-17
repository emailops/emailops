// The email frame under the app's dark theme: the srcdoc carries the plan
// `src/lib/emailScheme` chose, and the rendered iframe follows the theme store.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

import { initialThemeState, useThemeStore } from '@/stores/themeStore';
import { buildSrcDoc, EmailHtmlFrame, frameSchemeCss } from './EmailHtmlFrame';

const INVERT = 'filter: invert(1) hue-rotate(180deg)';

describe('buildSrcDoc per scheme plan', () => {
  it('light: the rendering the sender designed, dark rules dropped', () => {
    const doc = buildSrcDoc('<p>x</p>', true, { canvas: 'light', invert: false, darkRules: 'drop' });
    expect(doc).toContain('<meta name="color-scheme" content="light">');
    expect(doc.startsWith('<!doctype html><html><head>')).toBe(true);
    expect(doc).not.toContain(INVERT);
  });

  it('dark + force: the message’s own dark styles, on a dark canvas', () => {
    const doc = buildSrcDoc('<p>x</p>', true, { canvas: 'dark', invert: false, darkRules: 'force' });
    expect(doc.startsWith('<!doctype html><html data-dark-rules="force"><head>')).toBe(true);
    expect(doc).toContain('<meta name="color-scheme" content="dark">');
    expect(doc).toMatch(/:root \{ color-scheme: dark; \}/);
    expect(doc).not.toContain(INVERT);
  });

  it('dark + invert: adapted on a dark canvas, images put back', () => {
    const css = frameSchemeCss({ canvas: 'dark', invert: true, darkRules: 'drop' });
    expect(css).toMatch(/html \{ background: #111827; \}/);
    expect(css).toMatch(/body \{ filter: invert\(1\) hue-rotate\(180deg\); \}/);
    expect(css).toMatch(/img, picture, video, svg, canvas, mark, \[style\*="background-image"\] \{ filter: invert/);
    // Inverted rendering keeps the light system colours: they invert to dark.
    const doc = buildSrcDoc('<p>x</p>', true, { canvas: 'dark', invert: true, darkRules: 'drop' });
    expect(doc).toContain('<meta name="color-scheme" content="light">');
  });

  it('dark design left alone: dark canvas, light defaults, no inversion', () => {
    const css = frameSchemeCss({ canvas: 'dark', invert: false, darkRules: 'drop' });
    expect(css).toMatch(/body \{ color: #e5e7eb; \}/);
    expect(css).not.toContain(INVERT);
  });
});

describe('EmailHtmlFrame follows the theme store', () => {
  let container: HTMLDivElement;
  let root: Root;
  beforeEach(() => {
    useThemeStore.setState({ ...initialThemeState, isLoading: false });
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });
  afterEach(() => {
    act(() => root.unmount());
    container.remove();
    useThemeStore.setState({ ...initialThemeState });
  });

  const srcdoc = () => container.querySelector('iframe')?.getAttribute('srcdoc') ?? '';

  it('renders a plain message light on the light theme and adapted on the dark theme', async () => {
    await act(async () => {
      root.render(<EmailHtmlFrame html="<p>Hello</p>" allowRemoteContent={false} />);
    });
    expect(srcdoc()).not.toContain(INVERT);

    await act(async () => {
      useThemeStore.setState({ scheme: 'dark' });
    });
    expect(srcdoc()).toContain(INVERT);
  });
});
