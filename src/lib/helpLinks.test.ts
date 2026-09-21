import { describe, expect, it } from 'vitest';
import { helpLinkToDocsUrl, parseHelpLink } from './helpLinks';

describe('parseHelpLink', () => {
  it('parses lang, page and anchor', () => {
    expect(parseHelpLink('help://es/ai-features#choosing-a-backend')).toEqual({
      lang: 'es',
      page: 'ai-features',
      anchor: 'choosing-a-backend',
    });
  });

  it('accepts a page link without anchor', () => {
    expect(parseHelpLink('help://en/cli')).toEqual({ lang: 'en', page: 'cli', anchor: null });
  });

  it('keeps accented slug anchors', () => {
    expect(parseHelpLink('help://fr/features#fonctionnalités-ia')?.anchor).toBe('fonctionnalités-ia');
  });

  it('rejects malformed links', () => {
    for (const bad of [
      'help://',
      'help://es',
      'help://es/../x',
      'help://es/ai features',
      'email://abc',
      'help://esp/page',
    ]) {
      expect(parseHelpLink(bad)).toBeNull();
    }
  });
});

describe('helpLinkToDocsUrl', () => {
  // Spanish is the site's default language and is served without a prefix:
  // /es/docs/… is a 404 on getemailops.com.
  it('maps a Spanish link to the unprefixed docs page', () => {
    expect(helpLinkToDocsUrl('help://es/ai-features#choosing-a-backend')).toBe(
      'https://getemailops.com/docs/ai-features/#choosing-a-backend',
    );
  });

  it('keeps the language prefix for the other languages', () => {
    expect(helpLinkToDocsUrl('help://en/ai-features#choosing-a-backend')).toBe(
      'https://getemailops.com/en/docs/ai-features/#choosing-a-backend',
    );
  });

  it('maps a page link to the page root', () => {
    expect(helpLinkToDocsUrl('help://de/troubleshooting')).toBe('https://getemailops.com/de/docs/troubleshooting/');
  });

  it('returns null for a non-help href', () => {
    expect(helpLinkToDocsUrl('https://example.com')).toBeNull();
  });
});
