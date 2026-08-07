import { describe, expect, it } from 'vitest';
import { declaresOwnColors, type EmailBodyTheme, emailThemeCss, planEmailBodyTheme } from './emailTheme';

describe('declaresOwnColors', () => {
  it('sees a legacy bgcolor attribute', () => {
    // Newsletters are still built on table layouts from 2005.
    expect(declaresOwnColors('<table bgcolor="#ffffff"><tr><td>hi</td></tr></table>')).toBe(true);
  });

  it('sees a background in an inline style', () => {
    expect(declaresOwnColors('<div style="background-color:#f4f4f4">hi</div>')).toBe(true);
    expect(declaresOwnColors('<div style="background:#fff url(x.png)">hi</div>')).toBe(true);
  });

  it('sees a colour in a style block', () => {
    expect(declaresOwnColors('<style>.wrap{color:#333}</style><div class="wrap">hi</div>')).toBe(true);
  });

  it('does not count a plain paragraph as declaring anything', () => {
    // The converted-plain-text case: the overwhelming majority of person-to-
    // person mail, and the case a white slab looks worst on.
    expect(declaresOwnColors('<p>Are you free around noon?</p>')).toBe(false);
  });

  it('does not count layout-only styles', () => {
    // `style` is not itself evidence — only a colour declaration is.
    expect(declaresOwnColors('<div style="margin:0;padding:8px;font-size:14px">hi</div>')).toBe(false);
  });

  it('ignores the word "color" inside text content', () => {
    // A message *about* colours declares nothing.
    expect(declaresOwnColors('<p>Please pick a background color for the logo.</p>')).toBe(false);
  });
});

describe('planEmailBodyTheme', () => {
  const styled = { declaresColors: true };
  const plain = { declaresColors: false };

  it('never darkens while the app is light', () => {
    expect(planEmailBodyTheme({ appTheme: 'light', override: null, ...styled })).toBe('light');
    expect(planEmailBodyTheme({ appTheme: 'light', override: null, ...plain })).toBe('light');
  });

  it('darkens an undeclared email natively', () => {
    // No colours of its own means nothing to preserve and nothing to break.
    expect(planEmailBodyTheme({ appTheme: 'dark', override: null, ...plain })).toBe('dark-native');
  });

  it('inverts an email that brought its own colours', () => {
    // Its palette is deliberate; native darkening would leave dark text on a
    // dark background wherever it set one and not the other.
    expect(planEmailBodyTheme({ appTheme: 'dark', override: null, ...styled })).toBe('dark-inverted');
  });

  it('lets the reader force one message back to light', () => {
    // The escape hatch that makes inversion acceptable: some mail will look
    // wrong, and the reader must be able to undo it per message.
    expect(planEmailBodyTheme({ appTheme: 'dark', override: 'light', ...styled })).toBe('light');
    expect(planEmailBodyTheme({ appTheme: 'dark', override: 'light', ...plain })).toBe('light');
  });

  it('lets the reader force a light-mode message dark', () => {
    expect(planEmailBodyTheme({ appTheme: 'light', override: 'dark', ...plain })).toBe('dark-native');
    expect(planEmailBodyTheme({ appTheme: 'light', override: 'dark', ...styled })).toBe('dark-inverted');
  });
});

describe('emailThemeCss', () => {
  it('adds nothing at all in light mode', () => {
    // The light path must stay byte-identical to what shipped before, so a
    // theme bug cannot reach the reader who never turned dark mode on.
    expect(emailThemeCss('light')).toBe('');
  });

  it('sets a dark surface and light text for an undeclared email', () => {
    const css = emailThemeCss('dark-native');
    expect(css).toMatch(/background:\s*#1e1e1e/);
    expect(css).toMatch(/color:\s*#e5e7eb/);
    expect(css).not.toContain('invert(');
  });

  it('inverts the document and counter-inverts media', () => {
    // Without the counter-inversion every photo and logo renders as a colour
    // negative — the single most obvious failure of this technique.
    const css = emailThemeCss('dark-inverted');
    expect(css).toMatch(/html\s*\{[^}]*invert\(1\)/);
    expect(css).toMatch(/img[^{]*\{[^}]*invert\(1\)/);
  });

  it('inverts from a light grey so the result matches the app surface', () => {
    // Inverting pure white yields pure black, which reads as a hole next to
    // the app's #1e1e1e chrome. #e1e1e1 inverts to #1e1e1e.
    expect(emailThemeCss('dark-inverted')).toMatch(/#e1e1e1/);
  });

  it('counter-inverts CSS background images too', () => {
    // Hero images are as often a `background-image` as an <img>.
    expect(emailThemeCss('dark-inverted')).toContain('background-image');
  });

  const allModes: EmailBodyTheme[] = ['light', 'dark-native', 'dark-inverted'];
  it('produces a string for every mode', () => {
    for (const mode of allModes) expect(typeof emailThemeCss(mode)).toBe('string');
  });
});
