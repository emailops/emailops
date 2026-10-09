// Long extracted values (a summary, a pasted paragraph) used to render in
// full, so one cell could fill the screen. They now clamp to a few lines and
// expand in place on demand.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));

import { isLongText, LongText } from './LensLongText';

describe('isLongText', () => {
  it.each([
    ['short value', false],
    ['x'.repeat(180), false],
    ['x'.repeat(181), true],
    ['a\nb\nc', false],
    ['a\nb\nc\nd', true],
  ])('%#: %j → %s', (text, expected) => {
    expect(isLongText(text)).toBe(expected);
  });
});

describe('LongText', () => {
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

  const toggle = () => container.querySelector('button');
  const textSpan = () => container.querySelector('span[title]') as HTMLSpanElement;

  it('renders a short value as is, with no toggle', () => {
    act(() => root.render(<LongText text="Net 30" className="" />));
    expect(container.textContent).toBe('Net 30');
    expect(toggle()).toBeNull();
  });

  it('clamps a long value, keeps it in the tooltip, and expands it in place', () => {
    const long = 'word '.repeat(80).trim();
    act(() => root.render(<LongText text={long} className="" />));
    expect(textSpan().className).toContain('line-clamp-3');
    // `block` would override the clamp's `display: -webkit-box` and show it all.
    expect(textSpan().className.split(' ')).not.toContain('block');
    expect(textSpan().title).toBe(long);
    expect(toggle()?.textContent).toBe('lenses:table.showMore');

    act(() => toggle()?.click());
    expect(textSpan().className).not.toContain('line-clamp-3');
    expect(toggle()?.textContent).toBe('lenses:table.showLess');
  });
});
