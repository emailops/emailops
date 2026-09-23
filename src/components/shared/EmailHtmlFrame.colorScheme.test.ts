// The app renders email bodies on a light reading pane, but the webview (and so
// the srcdoc iframe) inherits the OS appearance. On a Mac in dark mode, a
// newsletter's `@media (prefers-color-scheme: dark)` rules turn its text white
// while its dark background never materialises — white text on a white pane.
// The bridge drops the dark-scheme media conditions from the email's own
// stylesheets so the body always renders in the scheme the pane actually shows.

import { afterEach, describe, expect, it } from 'vitest';
import { BRIDGE_SCRIPT, FRAME_BASE_CSS } from './EmailHtmlFrame';

function loadEmailStyles(css: string): CSSStyleSheet {
  const style = document.createElement('style');
  style.textContent = css;
  document.body.appendChild(style);
  new Function(BRIDGE_SCRIPT)();
  return style.sheet as CSSStyleSheet;
}

function mediaTexts(sheet: CSSStyleSheet): string[] {
  return Array.from(sheet.cssRules)
    .filter((r): r is CSSMediaRule => r instanceof CSSMediaRule)
    .map((r) => r.media.mediaText);
}

afterEach(() => {
  document.body.innerHTML = '';
});

describe('EmailHtmlFrame colour scheme', () => {
  it('drops a dark-scheme media rule and keeps the plain rules', () => {
    const sheet = loadEmailStyles(`
      p { color: #111111; }
      @media (prefers-color-scheme: dark) { p { color: #ffffff !important; } }
    `);
    expect(mediaTexts(sheet)).toEqual([]);
    expect(sheet.cssRules).toHaveLength(1);
    expect((sheet.cssRules[0] as CSSStyleRule).selectorText).toBe('p');
  });

  it('drops a dark-scheme rule combined with other features', () => {
    const sheet = loadEmailStyles(`
      @media (prefers-color-scheme: dark) and (hover: hover) { span { color: #ffffff; } }
    `);
    expect(sheet.cssRules).toHaveLength(0);
  });

  it('keeps the other branches of a comma-separated media list', () => {
    const sheet = loadEmailStyles(`
      @media (max-width: 600px), (prefers-color-scheme: dark) { td { display: block; } }
    `);
    expect(mediaTexts(sheet)).toEqual(['(max-width: 600px)']);
  });

  it('drops dark-scheme rules nested inside another grouping rule', () => {
    const sheet = loadEmailStyles(`
      @media screen {
        a { color: #2255aa; }
        @media (prefers-color-scheme: dark) { a { color: #ffffff; } }
      }
    `);
    const outer = sheet.cssRules[0] as CSSMediaRule;
    expect(outer.cssRules).toHaveLength(1);
    expect((outer.cssRules[0] as CSSStyleRule).selectorText).toBe('a');
  });

  it('leaves light-scheme and unrelated media rules untouched', () => {
    const sheet = loadEmailStyles(`
      @media (prefers-color-scheme: light) { p { color: #000000; } }
      @media (max-width: 600px) { td { display: block; } }
    `);
    expect(mediaTexts(sheet)).toEqual(['(prefers-color-scheme: light)', '(max-width: 600px)']);
  });

  it('pins the frame document to the light scheme so system colours resolve light', () => {
    expect(FRAME_BASE_CSS).toMatch(/:root\s*\{\s*color-scheme:\s*light only;?\s*\}/);
  });
});
