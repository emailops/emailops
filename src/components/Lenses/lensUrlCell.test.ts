import { describe, expect, it } from 'vitest';
import { planUrlCell } from './lensUrlCell';

// A `url` column's value is whatever the extractor pulled out of an email — i.e.
// attacker-influenced text. Every other link surface in the app (MarkdownContent,
// EmailHtmlFrame) gates on scheme before rendering an href; this cell did not,
// so it rendered `javascript:`/`data:`/`file:` URLs as live links in the app's
// own webview.
describe('planUrlCell', () => {
  it('renders http and https URLs as links', () => {
    expect(planUrlCell('https://example.com/invoice/1')).toEqual({
      kind: 'link',
      href: 'https://example.com/invoice/1',
    });
    expect(planUrlCell('http://example.com/x')).toEqual({
      kind: 'link',
      href: 'http://example.com/x',
    });
  });

  it('refuses javascript: URLs and falls back to plain text', () => {
    expect(planUrlCell('javascript:alert(document.cookie)')).toEqual({
      kind: 'text',
      text: 'javascript:alert(document.cookie)',
    });
  });

  it('refuses data:, file: and custom schemes', () => {
    for (const value of [
      'data:text/html,<script>alert(1)</script>',
      'file:///etc/passwd',
      'vbscript:msgbox(1)',
      'emailops://do-something',
    ]) {
      expect(planUrlCell(value), `${value} must not become a link`).toEqual({
        kind: 'text',
        text: value,
      });
    }
  });

  it('treats unparseable values as text rather than throwing', () => {
    expect(planUrlCell('not a url at all')).toEqual({ kind: 'text', text: 'not a url at all' });
    expect(planUrlCell('')).toEqual({ kind: 'text', text: '' });
  });

  it('stringifies non-string values before deciding', () => {
    expect(planUrlCell(42)).toEqual({ kind: 'text', text: '42' });
    expect(planUrlCell(null)).toEqual({ kind: 'text', text: 'null' });
  });

  // Case-mangled schemes are the classic bypass for a naive `startsWith` check.
  it('is not fooled by scheme casing or leading whitespace', () => {
    expect(planUrlCell('JaVaScRiPt:alert(1)').kind).toBe('text');
    expect(planUrlCell('  javascript:alert(1)').kind).toBe('text');
  });
});
