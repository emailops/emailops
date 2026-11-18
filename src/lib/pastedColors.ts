/**
 * Pasted rich text keeps its text colours only where they can be read.
 *
 * A browser copy carries the source page's computed styles, so text copied
 * from a dark-themed app arrives with that theme's foreground colour (white
 * or near-white) set on it. On the composer's dark surface it looks like any
 * other text; on a recipient's white page it is invisible, and the dark
 * reader's inversion turns it black. A colour is judged against the
 * background it will be shown on — the nearest one the pasted HTML declares,
 * else the white page a message renders on — and dropped below WCAG's 3:1,
 * the floor for large text. Readable colours (the red a sender chose) stay.
 */

import { relativeLuminance } from './emailScheme';

/** WCAG 2 minimum contrast for large text; anything below is not legible as text. */
const MIN_CONTRAST = 3;

/** The page a message renders on when nothing declares a background. */
const PAGE_LUMINANCE = 1;

export function dropUnreadableColors(html: string): string {
  const doc = new DOMParser().parseFromString(`<body>${html}</body>`, 'text/html');
  for (const el of Array.from(doc.body.querySelectorAll<HTMLElement>('[style], font[color]'))) {
    const color = el.tagName === 'FONT' && el.hasAttribute('color') ? el.getAttribute('color') : el.style.color;
    const text = color ? relativeLuminance(color) : null;
    if (text === null || contrast(text, backgroundLuminance(el)) >= MIN_CONTRAST) continue;
    if (el.tagName === 'FONT') el.removeAttribute('color');
    el.style.removeProperty('color');
    if (el.style.length === 0) el.removeAttribute('style');
  }
  return doc.body.innerHTML;
}

/** Luminance of the nearest background the element or an ancestor declares. */
function backgroundLuminance(el: HTMLElement): number {
  for (let node: HTMLElement | null = el; node; node = node.parentElement) {
    const declared = node.style.backgroundColor || node.getAttribute('bgcolor');
    const luminance = declared ? relativeLuminance(declared) : null;
    if (luminance !== null) return luminance;
  }
  return PAGE_LUMINANCE;
}

function contrast(a: number, b: number): number {
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}
