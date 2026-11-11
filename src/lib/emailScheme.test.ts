// How an email body is shown under each app theme: the pure planner behind
// EmailHtmlFrame. On the dark theme a message is shown dark whenever it can
// be — a message that ships its own dark styles uses them, a plain or
// light-designed one is adapted, and one that is already dark is left alone.

import { describe, expect, it } from 'vitest';

import { declaredBackgrounds, hasDarkSchemeRules, planEmailScheme, relativeLuminance } from './emailScheme';

const PLAIN = '<p>Hello,</p><p>see you Monday.</p>';
const DARK_TEXT_NO_BG = '<div style="color:#222;font-family:Arial"><p>Hello</p></div>';
const LIGHT_NEWSLETTER =
  '<table bgcolor="#ffffff"><tr><td style="background-color:#1e3a8a;color:#fff">Header</td></tr>' +
  '<tr><td bgcolor="#f3f4f6">Body</td></tr><tr><td style="background:#ffffff">Footer</td></tr></table>';
const DARK_NEWSLETTER =
  '<body bgcolor="#0b0b0b"><table style="background:#111111"><tr><td style="color:#eee">Hi</td></tr></table></body>';
const WITH_DARK_RULES =
  '<style>body{background:#fff;color:#111} @media (prefers-color-scheme: dark){body{background:#000;color:#eee}}</style><p>Hi</p>';

describe('relativeLuminance', () => {
  it.each([
    ['#ffffff', 1],
    ['#fff', 1],
    ['#000000', 0],
    ['white', 1],
    ['black', 0],
    ['rgb(255, 255, 255)', 1],
    // A translucent colour is what it shows over the white page behind it.
    ['rgba(0,0,0,0.5)', 0.21],
    ['rgb(0,0,0,.1)', 0.79],
    ['rgba(0, 0, 0, 10%)', 0.79],
    ['#0000001a', 0.78],
  ])('%s → %s', (color, expected) => {
    expect(relativeLuminance(color)).toBeCloseTo(expected, 2);
  });

  it('returns null for colours it cannot read', () => {
    expect(relativeLuminance('rgba(0,0,0,0)')).toBeNull();
    expect(relativeLuminance('transparent')).toBeNull();
    expect(relativeLuminance('url(x.png)')).toBeNull();
    expect(relativeLuminance('inherit')).toBeNull();
    expect(relativeLuminance('var(--x)')).toBeNull();
  });
});

describe('declaredBackgrounds', () => {
  it('finds bgcolor attributes, inline styles and style blocks', () => {
    expect(declaredBackgrounds(LIGHT_NEWSLETTER)).toEqual(['#ffffff', '#1e3a8a', '#f3f4f6', '#ffffff']);
    expect(declaredBackgrounds('<style>.x{background-color: rgb(1,2,3) !important}</style>')).toEqual(['rgb(1,2,3)']);
  });

  it('reads the colour out of a background shorthand with an image', () => {
    expect(declaredBackgrounds('<td style="background: #abcdef url(a.png) no-repeat">x</td>')).toEqual(['#abcdef']);
  });

  it('is empty for a plain message', () => {
    expect(declaredBackgrounds(PLAIN)).toEqual([]);
    expect(declaredBackgrounds(DARK_TEXT_NO_BG)).toEqual([]);
  });
});

describe('hasDarkSchemeRules', () => {
  it('is true only when a style block has a dark-scheme media rule', () => {
    expect(hasDarkSchemeRules(WITH_DARK_RULES)).toBe(true);
    expect(hasDarkSchemeRules(LIGHT_NEWSLETTER)).toBe(false);
    expect(hasDarkSchemeRules('<style>@media (prefers-color-scheme:light){p{color:#000}}</style>')).toBe(false);
  });
});

describe('planEmailScheme', () => {
  it('on the light theme every message renders light, as designed', () => {
    for (const html of [PLAIN, DARK_TEXT_NO_BG, LIGHT_NEWSLETTER, DARK_NEWSLETTER, WITH_DARK_RULES]) {
      expect(planEmailScheme(html, 'light')).toEqual({ canvas: 'light', invert: false, darkRules: 'drop' });
    }
  });

  it('on the dark theme a message with its own dark styles uses them', () => {
    expect(planEmailScheme(WITH_DARK_RULES, 'dark')).toEqual({ canvas: 'dark', invert: false, darkRules: 'force' });
  });

  it('on the dark theme a plain message is adapted (inverted on a dark canvas)', () => {
    expect(planEmailScheme(PLAIN, 'dark')).toEqual({ canvas: 'dark', invert: true, darkRules: 'drop' });
    expect(planEmailScheme(DARK_TEXT_NO_BG, 'dark')).toEqual({ canvas: 'dark', invert: true, darkRules: 'drop' });
  });

  it('on the dark theme a light design is adapted', () => {
    expect(planEmailScheme(LIGHT_NEWSLETTER, 'dark')).toEqual({ canvas: 'dark', invert: true, darkRules: 'drop' });
  });

  it('on the dark theme a design that is already dark is left alone', () => {
    expect(planEmailScheme(DARK_NEWSLETTER, 'dark')).toEqual({ canvas: 'dark', invert: false, darkRules: 'drop' });
  });

  it('judges by the typical background, not the first one', () => {
    const darkHeaderLightBody =
      '<table><tr><td bgcolor="#000000">Header</td></tr><tr><td bgcolor="#ffffff">A</td></tr>' +
      '<tr><td bgcolor="#ffffff">B</td></tr></table>';
    expect(planEmailScheme(darkHeaderLightBody, 'dark').invert).toBe(true);
  });

  it('reads translucent overlays as the light page they tint, not as black', () => {
    const lightPageWithOverlays =
      '<div style="background:rgb(0,0,0,.1)">a</div><div style="background:rgb(0,0,0,.1)">b</div>' +
      '<div style="background-color:rgba(0,0,0,.1)">c</div><a style="background-color:#155dff">Open</a>';
    expect(planEmailScheme(lightPageWithOverlays, 'dark').invert).toBe(true);
  });

  it('treats unreadable background values as absent', () => {
    expect(planEmailScheme('<td style="background: url(a.png)">x</td>', 'dark').invert).toBe(true);
  });
});
