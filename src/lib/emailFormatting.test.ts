import { describe, expect, it } from 'vitest';
import {
  getSafeExternalUrl,
  parseMailtoUrl,
  sanitizeCssValue,
  sanitizeEmailHtml,
  sanitizeEmailHtmlFull,
} from './emailFormatting';

// ---------------------------------------------------------------------------
// sanitizeEmailHtmlFull — remote-content gating
// ---------------------------------------------------------------------------

describe('sanitizeEmailHtmlFull remote-content gating', () => {
  // Regression: the blocking hook only inspected IMG and SOURCE, so a sender
  // could get a guaranteed read-receipt through <video>/<audio> even with
  // "load remote images" off. `poster` is the worst of them — it fetches on
  // render with no user interaction at all.
  it('strips remote media sources on video and audio, not just images', () => {
    const html =
      '<video src="https://tracker.example/x.mp4" poster="https://tracker.example/p.jpg"></video>' +
      '<audio src="https://tracker.example/a.mp3"></audio>';

    const { html: clean, hasBlockedImages } = sanitizeEmailHtmlFull(html, false);

    expect(clean).not.toContain('tracker.example');
    expect(hasBlockedImages).toBe(true);
  });

  it('strips a remote video poster even when the video has no src', () => {
    const { html: clean, hasBlockedImages } = sanitizeEmailHtmlFull(
      '<video poster="https://tracker.example/p.jpg"></video>',
      false,
    );

    expect(clean).not.toContain('tracker.example');
    expect(hasBlockedImages).toBe(true);
  });

  it('keeps remote media when the user has allowed remote content', () => {
    const html = '<video src="https://cdn.example/x.mp4" poster="https://cdn.example/p.jpg"></video>';

    const { html: clean, hasBlockedImages } = sanitizeEmailHtmlFull(html, true);

    expect(clean).toContain('https://cdn.example/x.mp4');
    expect(clean).toContain('https://cdn.example/p.jpg');
    expect(hasBlockedImages).toBe(false);
  });

  it('still blocks remote images (the original behaviour)', () => {
    const { html: clean, hasBlockedImages } = sanitizeEmailHtmlFull('<img src="https://tracker.example/i.png">', false);

    expect(clean).not.toContain('tracker.example');
    expect(hasBlockedImages).toBe(true);
  });

  it('leaves inline data: media alone — it discloses nothing to the sender', () => {
    const dataUri = 'data:image/gif;base64,R0lGODlhAQABAAAAACw=';
    const { html: clean, hasBlockedImages } = sanitizeEmailHtmlFull(`<img src="${dataUri}">`, false);

    expect(clean).toContain(dataUri);
    expect(hasBlockedImages).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// sanitizeCssValue
// ---------------------------------------------------------------------------

describe('sanitizeCssValue', () => {
  it('passes safe properties through', () => {
    const result = sanitizeCssValue('color: red; font-size: 14px; font-weight: bold');
    expect(result).toContain('color: red');
    expect(result).toContain('font-size: 14px');
    expect(result).toContain('font-weight: bold');
  });

  it('preserves layout properties (position, display) — iframe sandbox isolates them', () => {
    const result = sanitizeCssValue('position: absolute; display: none; color: blue');
    expect(result).toContain('position: absolute');
    expect(result).toContain('display: none');
    expect(result).toContain('color: blue');
  });

  it('strips the IE-only behavior property (can load .htc scripts)', () => {
    const result = sanitizeCssValue('behavior: url(evil.htc); color: red');
    expect(result).not.toContain('behavior');
    expect(result).toContain('color: red');
  });

  it('blocks remote url() when allowRemote is false', () => {
    const result = sanitizeCssValue('background-image: url(https://evil.com/tracker.png)');
    expect(result).toBe('');
  });

  it('allows remote url() when allowRemote is true', () => {
    const result = sanitizeCssValue('background-image: url(https://example.com/bg.png)', true);
    expect(result).toContain('background-image');
  });

  it('allows url() with raster data:image/ URIs', () => {
    const result = sanitizeCssValue('background-image: url(data:image/png;base64,abc)');
    expect(result).toContain('background-image');
  });

  it('blocks url(data:image/svg+xml,...) which can carry inline scripts', () => {
    const result = sanitizeCssValue('background-image: url(data:image/svg+xml;utf8,<svg onload="alert(1)"/>)');
    expect(result).toBe('');
  });

  it('blocks values that mix a safe substring with an unsafe url()', () => {
    // Old check used a substring match for `data:image/`, so a malicious
    // url() right next to a benign mention slipped through.
    const result = sanitizeCssValue('background-image: url(https://evil.com/x.png) /* data:image/png hint */');
    expect(result).toBe('');
  });

  it('blocks expression()', () => {
    const result = sanitizeCssValue('color: expression(alert(1))');
    expect(result).toBe('');
  });

  it('blocks javascript: in CSS values', () => {
    const result = sanitizeCssValue('background: javascript:alert(1)');
    expect(result).toBe('');
  });

  it('handles empty and whitespace-only input', () => {
    expect(sanitizeCssValue('')).toBe('');
    expect(sanitizeCssValue('   ')).toBe('');
  });

  it('handles declarations without a colon', () => {
    const result = sanitizeCssValue('color red; font-size: 14px');
    expect(result).not.toContain('color red');
    expect(result).toContain('font-size: 14px');
  });

  it('is case-insensitive for property names', () => {
    const result = sanitizeCssValue('COLOR: blue; FONT-SIZE: 12px');
    expect(result).toContain('COLOR: blue');
    expect(result).toContain('FONT-SIZE: 12px');
  });
});

// ---------------------------------------------------------------------------
// sanitizeEmailHtml
// ---------------------------------------------------------------------------

describe('sanitizeEmailHtml', () => {
  // --- XSS / dangerous elements ---

  it('removes <script> tags', () => {
    const result = sanitizeEmailHtml('<p>Hello</p><script>alert(1)</script>');
    expect(result).not.toContain('<script');
    expect(result).not.toContain('alert(1)');
    expect(result).toContain('<p>Hello</p>');
  });

  it('preserves <style> tags (needed for modern email layouts; iframe sandbox isolates them)', () => {
    const result = sanitizeEmailHtml('<style>.h-0{display:none}</style><p>Body</p>');
    expect(result).toContain('<style');
    expect(result).toContain('.h-0');
    expect(result).toContain('<p>Body</p>');
  });

  it('removes <iframe> tags', () => {
    const result = sanitizeEmailHtml('<iframe src="https://evil.com"></iframe><p>Safe</p>');
    expect(result).not.toContain('<iframe');
    expect(result).toContain('<p>Safe</p>');
  });

  it('removes onclick and other event handlers', () => {
    const result = sanitizeEmailHtml('<a href="https://example.com" onclick="alert(1)">Link</a>');
    expect(result).not.toContain('onclick');
    expect(result).toContain('href="https://example.com"');
  });

  it('removes onerror on images', () => {
    const result = sanitizeEmailHtml('<img src="x" onerror="alert(1)" />');
    expect(result).not.toContain('onerror');
  });

  it('strips javascript: hrefs', () => {
    const result = sanitizeEmailHtml('<a href="javascript:alert(1)">Click</a>');
    expect(result).not.toContain('javascript:');
  });

  it('strips data-* attributes', () => {
    const result = sanitizeEmailHtml('<p data-secret="token123">Hello</p>');
    expect(result).not.toContain('data-secret');
    expect(result).toContain('<p');
  });

  // --- Preserved structure elements ---

  it('preserves heading tags h1–h4', () => {
    const html = '<h1>Title</h1><h2>Section</h2><h3>Sub</h3><h4>Detail</h4>';
    const result = sanitizeEmailHtml(html);
    expect(result).toContain('<h1>Title</h1>');
    expect(result).toContain('<h2>Section</h2>');
    expect(result).toContain('<h3>Sub</h3>');
    expect(result).toContain('<h4>Detail</h4>');
  });

  it('preserves unordered and ordered lists', () => {
    const html = '<ul><li>One</li><li>Two</li></ul><ol><li>A</li></ol>';
    const result = sanitizeEmailHtml(html);
    expect(result).toContain('<ul>');
    expect(result).toContain('<li>One</li>');
    expect(result).toContain('<ol>');
  });

  it('preserves inline formatting: strong, em, b, i', () => {
    const html = '<p><strong>Bold</strong> and <em>italic</em> and <b>b</b> and <i>i</i></p>';
    const result = sanitizeEmailHtml(html);
    expect(result).toContain('<strong>Bold</strong>');
    expect(result).toContain('<em>italic</em>');
    expect(result).toContain('<b>b</b>');
    expect(result).toContain('<i>i</i>');
  });

  it('preserves <blockquote> for quoted replies', () => {
    const html = '<blockquote>On Monday Alice wrote: Hi</blockquote>';
    const result = sanitizeEmailHtml(html);
    expect(result).toContain('<blockquote>');
  });

  it('preserves https links', () => {
    const result = sanitizeEmailHtml('<a href="https://example.com">Visit</a>');
    expect(result).toContain('href="https://example.com"');
    expect(result).toContain('>Visit</a>');
  });

  it('preserves mailto links', () => {
    const result = sanitizeEmailHtml('<a href="mailto:user@example.com">Email</a>');
    expect(result).toContain('href="mailto:user@example.com"');
  });

  it('preserves inline styles including layout properties', () => {
    const html = '<p style="color: red; position: absolute">Text</p>';
    const result = sanitizeEmailHtml(html);
    expect(result).toContain('color: red');
    expect(result).toContain('position: absolute');
  });

  it('preserves img tags with src', () => {
    const result = sanitizeEmailHtml('<img src="https://example.com/img.png" alt="photo" />');
    expect(result).toContain('<img');
    expect(result).toContain('src="https://example.com/img.png"');
  });

  it('preserves table structure', () => {
    const html = '<table><tr><td>Cell</td></tr></table>';
    const result = sanitizeEmailHtml(html);
    expect(result).toContain('<table');
    expect(result).toContain('<td>Cell</td>');
  });

  // --- Realistic email scenarios ---

  it('handles a Zoom-style meeting summary with headings and lists', () => {
    const html = `
      <h2>Resumen rápido</h2>
      <p>Se discutieron los siguientes puntos.</p>
      <h3>Siguientes pasos</h3>
      <ul>
        <li>Revisar el documento</li>
        <li>Enviar el informe</li>
      </ul>
      <h3>Resumen</h3>
      <p>La reunión fue productiva.</p>
    `;
    const result = sanitizeEmailHtml(html);
    expect(result).toContain('<h2>');
    expect(result).toContain('Resumen rápido');
    expect(result).toContain('<h3>');
    expect(result).toContain('<ul>');
    expect(result).toContain('<li>Revisar el documento</li>');
  });

  it('keeps display:none on inline styles (iframe isolates email from app)', () => {
    // The previous renderer stripped `display:none` because email CSS leaked
    // into the app's Tailwind layer; the iframe sandbox removes that concern.
    const html = '<div style="display:none">Hidden</div><p>Visible</p>';
    const result = sanitizeEmailHtml(html);
    expect(result).toContain('display:none');
    expect(result).toContain('<p>Visible</p>');
  });
});

// ---------------------------------------------------------------------------
// getSafeExternalUrl
// ---------------------------------------------------------------------------

describe('getSafeExternalUrl', () => {
  it('returns https URLs unchanged', () => {
    expect(getSafeExternalUrl('https://example.com/path?q=1')).toBe('https://example.com/path?q=1');
  });

  it('returns http URLs unchanged', () => {
    expect(getSafeExternalUrl('http://example.com')).toBe('http://example.com/');
  });

  it('returns null for javascript: URLs', () => {
    expect(getSafeExternalUrl('javascript:alert(1)')).toBeNull();
  });

  it('returns null for file: URLs', () => {
    expect(getSafeExternalUrl('file:///etc/passwd')).toBeNull();
  });

  it('returns null for data: URLs', () => {
    expect(getSafeExternalUrl('data:text/html,<script>alert(1)</script>')).toBeNull();
  });

  it('returns null for malformed URLs', () => {
    expect(getSafeExternalUrl('not a url')).toBeNull();
    expect(getSafeExternalUrl('')).toBeNull();
  });

  it('returns null for mailto: URLs (not navigable externally)', () => {
    expect(getSafeExternalUrl('mailto:user@example.com')).toBeNull();
  });
});

describe('parseMailtoUrl', () => {
  it('parses a bare address', () => {
    expect(parseMailtoUrl('mailto:user@example.com')).toEqual({
      to: ['user@example.com'],
      subject: '',
      body: '',
    });
  });

  it('parses multiple comma-separated addresses', () => {
    expect(parseMailtoUrl('mailto:a@example.com,b@example.com')?.to).toEqual(['a@example.com', 'b@example.com']);
  });

  it('decodes percent-encoded addresses', () => {
    expect(parseMailtoUrl('mailto:user%40example.com')?.to).toEqual(['user@example.com']);
  });

  it('parses subject and body query params', () => {
    expect(parseMailtoUrl('mailto:user@example.com?subject=Hello%20there&body=First%20line%0ASecond')).toEqual({
      to: ['user@example.com'],
      subject: 'Hello there',
      body: 'First line\nSecond',
    });
  });

  it('merges to= query param addresses with the path address', () => {
    expect(parseMailtoUrl('mailto:a@example.com?to=b@example.com')?.to).toEqual(['a@example.com', 'b@example.com']);
  });

  it('deduplicates repeated addresses case-insensitively', () => {
    expect(parseMailtoUrl('mailto:A@Example.com,a@example.com')?.to).toEqual(['a@example.com']);
  });

  it('ignores entries that are not email addresses', () => {
    expect(parseMailtoUrl('mailto:not-an-address,user@example.com')?.to).toEqual(['user@example.com']);
  });

  it('returns null when no valid address is present', () => {
    expect(parseMailtoUrl('mailto:')).toBeNull();
    expect(parseMailtoUrl('mailto:?subject=hi')).toBeNull();
    expect(parseMailtoUrl('mailto:not-an-address')).toBeNull();
  });

  it('returns null for non-mailto URLs', () => {
    expect(parseMailtoUrl('https://example.com')).toBeNull();
    expect(parseMailtoUrl('javascript:alert(1)')).toBeNull();
    expect(parseMailtoUrl('')).toBeNull();
  });

  it('survives malformed percent-encoding without throwing', () => {
    expect(parseMailtoUrl('mailto:user@example.com?subject=%E0%A4%A')?.to).toEqual(['user@example.com']);
  });
});
