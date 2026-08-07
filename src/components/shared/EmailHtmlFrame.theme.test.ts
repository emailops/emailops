// The email body does not simply inherit the app theme — its HTML and CSS
// belong to the sender.
//
// An earlier version of this file pinned "always a light card", on the grounds
// that most mail sets a dark text colour without setting a background, so a
// dark surface underneath renders the message invisible. That reasoning still
// holds, and is exactly why the darkening is conditional rather than global:
// an email that declares no colours is darkened natively, one that brought a
// palette is inverted instead, and the reader can force either back to light.
// See `lib/emailTheme.ts` for the decision itself.

import { describe, expect, it } from 'vitest';
import { emailThemeCss, planEmailBodyTheme } from '@/lib/emailTheme';
import { FRAME_BASE_CSS } from './EmailHtmlFrame';

describe('FRAME_BASE_CSS', () => {
  it('gives the email an opaque light background by default', () => {
    // The base sheet is what a light-mode reader gets, and it must not be
    // transparent: `background: transparent` let the app's surface show
    // through, which under dark mode put the sender's dark text on it.
    expect(FRAME_BASE_CSS).not.toMatch(/background:\s*transparent/);
    expect(FRAME_BASE_CSS).toMatch(/background:\s*(#fff|#ffffff|white)/i);
  });

  it('still sets a dark text colour to go with it', () => {
    expect(FRAME_BASE_CSS).toMatch(/color:\s*#1f2937/);
  });

  it('carries no dark-mode rule of its own', () => {
    // A `prefers-color-scheme` block here would darken the body on any machine
    // set to dark, independent of the app's own setting and of the per-message
    // override — the two things that make this safe.
    expect(FRAME_BASE_CSS).not.toContain('prefers-color-scheme');
  });
});

describe('the base sheet and the theme sheet together', () => {
  it('leave a light-mode email byte-identical to before dark mode existed', () => {
    // The reader who never turns dark mode on cannot be affected by any of it.
    expect(emailThemeCss(planEmailBodyTheme({ appTheme: 'light', override: null, declaresColors: true }))).toBe('');
    expect(emailThemeCss(planEmailBodyTheme({ appTheme: 'light', override: null, declaresColors: false }))).toBe('');
  });

  it('override the base white when the body is darkened', () => {
    // Both dark modes must restate the background, since the base sheet has
    // already set white and the theme sheet is appended after it.
    for (const declaresColors of [true, false]) {
      const css = emailThemeCss(planEmailBodyTheme({ appTheme: 'dark', override: null, declaresColors }));
      expect(css).toMatch(/html,\s*body\s*\{[^}]*background:/);
    }
  });
});
