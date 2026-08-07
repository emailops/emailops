// How an email body is rendered under the app's dark theme.
//
// The body is the sender's HTML with the sender's CSS, so it is the one surface
// that cannot simply inherit `dark:` classes. Two cases behave very differently:
//
//   * An email that declares no colours of its own (a converted plain-text
//     message, a bare <p> newsletter) has nothing to preserve and nothing to
//     break — it can be darkened natively, and looks native doing it.
//
//   * An email that brought a palette (bgcolor tables, styled divs) must not be
//     darkened natively: it sets a background OR a colour but rarely both on
//     every element, so a dark surface underneath leaves dark text on dark. It
//     is inverted instead, with media counter-inverted so photos and logos stay
//     positive. That is a lossy transform and some mail will look wrong, which
//     is why the reader can override any single message.
//
// Pure and DOM-free: the decision and the stylesheet are both table-testable,
// and `EmailHtmlFrame` only stamps the result into the frame's <head>.

/** The app's resolved theme. */
export type AppTheme = 'light' | 'dark';

/** How to render one email body. */
export type EmailBodyTheme = 'light' | 'dark-native' | 'dark-inverted';

/** A reader's per-message override, if they set one. */
export type EmailThemeOverride = 'light' | 'dark' | null;

/** Colour declarations, in the three shapes email actually uses. Deliberately
 *  not a CSS parser: a false positive costs an inversion instead of a native
 *  darken (both readable), while a false negative costs dark-on-dark text. */
const COLOUR_DECLARATION = /(<[^>]+\sbgcolor\s*=)|(background(-color)?\s*:\s*[^;"']*#?\w)|(\bcolor\s*:\s*[^;"']*#?\w)/i;

/**
 * Whether the email brings colours of its own.
 *
 * Only markup is examined — a `color:` inside visible text is prose, not a
 * declaration, so the match must sit inside a tag or a style block.
 */
export function declaresOwnColors(html: string): boolean {
  // `<style>` contents are the one text node that IS a declaration, so they are
  // lifted out before the rest of the text is dropped — stripping text nodes
  // wholesale would throw away the stylesheet along with the prose.
  const styleBlocks = html.match(/<style\b[^>]*>([\s\S]*?)<\/style>/gi)?.join(' ') ?? '';
  const withoutStyles = html.replace(/<style\b[^>]*>[\s\S]*?<\/style>/gi, '');
  // Text nodes cannot declare anything; drop them so prose about colours does
  // not read as CSS.
  const markupOnly = withoutStyles.replace(/>([^<]*)</g, '><');
  return COLOUR_DECLARATION.test(markupOnly) || COLOUR_DECLARATION.test(styleBlocks);
}

export interface EmailThemePlan {
  appTheme: AppTheme;
  override: EmailThemeOverride;
  declaresColors: boolean;
}

/** Decide how one email body renders. */
export function planEmailBodyTheme({ appTheme, override, declaresColors }: EmailThemePlan): EmailBodyTheme {
  const wantsDark = override === 'dark' || (override === null && appTheme === 'dark');
  if (!wantsDark) return 'light';
  return declaresColors ? 'dark-inverted' : 'dark-native';
}

/**
 * The stylesheet to add inside the frame for a given mode.
 *
 * Empty for `light`, so the light path stays byte-identical to what shipped
 * before dark mode existed.
 */
export function emailThemeCss(mode: EmailBodyTheme): string {
  if (mode === 'light') return '';
  if (mode === 'dark-native') {
    return `
  html, body { background: #1e1e1e; color: #e5e7eb; }
  a { color: #7dd3fc; }
  /* Quoted-reply chrome and hairlines read as black boxes otherwise. */
  blockquote { border-color: #4b5563; color: #d1d5db; }
  hr { border-color: #374151; }
  table, td, th { border-color: #374151; }
`;
  }
  return `
  /* Invert from a light grey rather than white: inverting #ffffff yields pure
     black, which reads as a hole beside the app's #1e1e1e chrome, whereas
     #e1e1e1 inverts to exactly #1e1e1e. hue-rotate puts hues back where the
     inversion moved them, so brand colours stay recognisable. */
  html, body { background: #e1e1e1; }
  html { filter: invert(1) hue-rotate(180deg); }
  /* Counter-invert anything carrying real imagery, or every photo and logo
     renders as a colour negative — the most obvious failure of this technique.
     The nested filter cancels the outer one exactly. */
  img, video, picture, svg, canvas, iframe, [style*="background-image"] {
    filter: invert(1) hue-rotate(180deg);
  }
`;
}
