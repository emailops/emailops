// How an email body is shown under the app theme — the pure planner behind
// `EmailHtmlFrame`.
//
// On the light theme a message renders as its sender designed it. On the dark
// theme it is shown dark whenever it can be:
//
//   - a message that ships `@media (prefers-color-scheme: dark)` rules gets
//     them applied unconditionally (`darkRules: 'force'`) on a dark canvas;
//   - a plain message, or a light design (white and pale backgrounds), is
//     adapted: dark canvas plus a luminance inversion that keeps hues, with
//     images inverted back (`invert: true`) — the same trick Dark Reader uses;
//   - a design that is already dark is left alone on a dark canvas.
//
// "Light or dark design" is judged from the backgrounds the message declares
// (`bgcolor` attributes, `background(-color)` in inline styles and `<style>`
// blocks): the median luminance decides, so one dark header band does not
// stop a white newsletter from being adapted.

import type { Scheme } from './theme';

export interface EmailSchemePlan {
  /** What the frame paints behind the message. */
  canvas: Scheme;
  /** Invert luminance (hue-preserving) and re-invert images. */
  invert: boolean;
  /** What the bridge does with the message's dark-scheme media rules. */
  darkRules: 'drop' | 'force';
}

/** The light theme's rendering: as the sender designed it. */
export const LIGHT_EMAIL_PLAN: EmailSchemePlan = { canvas: 'light', invert: false, darkRules: 'drop' };

export function planEmailScheme(html: string, appScheme: Scheme): EmailSchemePlan {
  if (appScheme === 'light') return LIGHT_EMAIL_PLAN;
  if (hasDarkSchemeRules(html)) return { canvas: 'dark', invert: false, darkRules: 'force' };
  const lum = declaredBackgrounds(html)
    .map(relativeLuminance)
    .filter((l): l is number => l !== null)
    .sort((a, b) => a - b);
  if (lum.length === 0) return { canvas: 'dark', invert: true, darkRules: 'drop' };
  const median = lum[Math.floor(lum.length / 2)];
  return { canvas: 'dark', invert: median >= 0.5, darkRules: 'drop' };
}

const DARK_SCHEME_RULE = /@media[^{]*prefers-color-scheme\s*:\s*dark/i;

export function hasDarkSchemeRules(html: string): boolean {
  return DARK_SCHEME_RULE.test(html);
}

const BGCOLOR_ATTR = /\bbgcolor\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))/gi;
const BACKGROUND_DECL = /(?:^|[;\s{"'])background(?:-color)?\s*:\s*([^;}"']+)/gi;
const COLOR_TOKEN = /#[0-9a-f]{3,8}\b|rgba?\([^)]*\)|hsla?\([^)]*\)|\b[a-z]+\b/gi;

/** Background colours the message declares, in document order. */
export function declaredBackgrounds(html: string): string[] {
  const found: { at: number; color: string }[] = [];
  for (const m of html.matchAll(BGCOLOR_ATTR)) found.push({ at: m.index, color: (m[1] ?? m[2] ?? m[3]).trim() });
  for (const m of html.matchAll(BACKGROUND_DECL)) {
    const token = firstColorToken(m[1]);
    if (token) found.push({ at: m.index, color: token });
  }
  return found.sort((a, b) => a.at - b.at).map((f) => f.color);
}

function firstColorToken(value: string): string | null {
  for (const m of value.matchAll(COLOR_TOKEN)) {
    const token = m[0];
    if (token.startsWith('#') || /^(rgb|hsl)a?\(/i.test(token)) return token;
    if (NAMED_COLORS[token.toLowerCase()] !== undefined) return token;
  }
  return null;
}

/**
 * WCAG relative luminance (0 black … 1 white), or null when unreadable or
 * fully transparent. A translucent colour counts as what it shows over the
 * white page behind it: `rgba(0,0,0,.1)` is a light grey tint, not black.
 */
export function relativeLuminance(color: string): number | null {
  const parsed = parseColor(color.trim());
  if (!parsed) return null;
  const [rgb, alpha] = parsed;
  if (alpha === 0) return null;
  const [r, g, b] = rgb.map((c) => {
    c = c * alpha + 255 * (1 - alpha);
    const v = c / 255;
    return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

type Rgb = [number, number, number];

/** The colour's channels and its alpha (0 … 1), or null when unreadable. */
function parseColor(color: string): [Rgb, number] | null {
  const short = color.match(/^#([0-9a-f]{3,4})$/i)?.[1];
  const hex = short ? [...short].map((d) => d + d).join('') : color.match(/^#([0-9a-f]{6}|[0-9a-f]{8})$/i)?.[1];
  if (hex) {
    const [r, g, b, a = 255] = (hex.match(/../g) ?? []).map((pair) => Number.parseInt(pair, 16));
    return [[r, g, b], a / 255];
  }
  const rgb = color.match(/^rgba?\(\s*([\d.]+)\s*,\s*([\d.]+)\s*,\s*([\d.]+)\s*(?:[,/]\s*([\d.]+)(%?))?/i);
  if (rgb) {
    const alpha = rgb[4] === undefined ? 1 : Number(rgb[4]) / (rgb[5] ? 100 : 1);
    return [[Number(rgb[1]), Number(rgb[2]), Number(rgb[3])], Math.min(1, alpha)];
  }
  const named = NAMED_COLORS[color.toLowerCase()];
  return named ? [named, 1] : null;
}

// The named colours that show up as email backgrounds. Anything else (and
// `transparent`, `inherit`, `var(…)`) reads as "no colour declared".
const NAMED_COLORS: Record<string, Rgb> = {
  white: [255, 255, 255],
  black: [0, 0, 0],
  gray: [128, 128, 128],
  grey: [128, 128, 128],
  silver: [192, 192, 192],
  whitesmoke: [245, 245, 245],
  gainsboro: [220, 220, 220],
  lightgray: [211, 211, 211],
  lightgrey: [211, 211, 211],
  darkgray: [169, 169, 169],
  dimgray: [105, 105, 105],
  ivory: [255, 255, 240],
  beige: [245, 245, 220],
  linen: [250, 240, 230],
  snow: [255, 250, 250],
  aliceblue: [240, 248, 255],
  ghostwhite: [248, 248, 255],
  navy: [0, 0, 128],
  maroon: [128, 0, 0],
  red: [255, 0, 0],
  blue: [0, 0, 255],
  green: [0, 128, 0],
  yellow: [255, 255, 0],
  orange: [255, 165, 0],
};
