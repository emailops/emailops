import { describe, expect, it } from 'vitest';
import { htmlToPlainText, plainTextToHtml, prepareOutgoingHtml } from './composeHtml';

describe('prepareOutgoingHtml', () => {
  it('passes through HTML with no images unchanged (modulo serialization)', () => {
    const out = prepareOutgoingHtml('<p>hello <strong>world</strong></p>');
    expect(out.inlineImages).toHaveLength(0);
    expect(out.bodyHtml).toContain('<p>hello <strong>world</strong></p>');
  });

  it('extracts a data:image into an inline attachment with cid src', () => {
    const png = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=';
    const html = `<p>see this <img src="data:image/png;base64,${png}" alt="logo"></p>`;
    const out = prepareOutgoingHtml(html, 'fixed');
    expect(out.inlineImages).toHaveLength(1);
    expect(out.inlineImages[0]).toMatchObject({
      filename: 'inline-1.png',
      mimeType: 'image/png',
      data: png,
      contentId: 'fixed-1',
      isInline: true,
    });
    expect(out.bodyHtml).toContain('src="cid:fixed-1"');
    expect(out.bodyHtml).not.toContain('data:image/png');
  });

  it('handles multiple pasted images with unique cids', () => {
    const a = 'AAA=';
    const b = 'BBB=';
    const html = `<p><img src="data:image/jpeg;base64,${a}"><img src="data:image/gif;base64,${b}"></p>`;
    const out = prepareOutgoingHtml(html, 'p');
    expect(out.inlineImages).toHaveLength(2);
    expect(out.inlineImages[0].contentId).toBe('p-1');
    expect(out.inlineImages[1].contentId).toBe('p-2');
    expect(out.inlineImages[0].filename).toBe('inline-1.jpg');
    expect(out.inlineImages[1].filename).toBe('inline-2.gif');
    expect(out.bodyHtml).toContain('cid:p-1');
    expect(out.bodyHtml).toContain('cid:p-2');
  });

  it('leaves remote https images untouched', () => {
    const html = '<p><img src="https://example.com/logo.png"></p>';
    const out = prepareOutgoingHtml(html);
    expect(out.inlineImages).toHaveLength(0);
    expect(out.bodyHtml).toContain('https://example.com/logo.png');
  });

  it('rejects non-image data URLs (e.g. data:text/html)', () => {
    // Even if a hostile script slipped a `data:text/html` past the editor,
    // we should not turn it into an attachment with a fabricated MIME type.
    const html = '<p><img src="data:text/html;base64,PHNjcmlwdD4="></p>';
    const out = prepareOutgoingHtml(html);
    expect(out.inlineImages).toHaveLength(0);
    // The src is left as-is — backend ammonia sanitizer will strip `data:`
    // URLs that aren't valid image schemes when we tighten the allowlist.
    expect(out.bodyHtml).toContain('data:text/html');
  });

  it('produces a plaintext fallback that flattens block elements', () => {
    const out = prepareOutgoingHtml('<p>hello</p><p>world</p>');
    expect(out.plainText).toBe('hello\nworld');
  });
});

describe('htmlToPlainText', () => {
  it('replaces <br> with a single newline', () => {
    expect(htmlToPlainText('one<br>two')).toBe('one\ntwo');
  });

  it('treats <p> as a paragraph break', () => {
    expect(htmlToPlainText('<p>one</p><p>two</p>')).toBe('one\ntwo');
  });

  it('renders <a href> as "label (href)" when label differs', () => {
    expect(htmlToPlainText('<a href="https://example.com">click</a>')).toBe('click (https://example.com)');
  });

  it('renders <a href> as just the label when label equals href', () => {
    expect(htmlToPlainText('<a href="https://example.com">https://example.com</a>')).toBe('https://example.com');
  });

  it('replaces images with placeholders, preserving alt text when present', () => {
    expect(htmlToPlainText('see <img src="cid:x" alt="logo"> here')).toBe('see [image: logo] here');
    expect(htmlToPlainText('see <img src="cid:x"> here')).toBe('see [image] here');
  });

  it('flattens nested formatting cleanly', () => {
    expect(htmlToPlainText('<p>hello <strong>bold</strong> world</p>')).toBe('hello bold world');
  });

  it('collapses 3+ blank lines down to one blank line', () => {
    expect(htmlToPlainText('<p>a</p><p></p><p></p><p>b</p>')).toBe('a\n\nb');
  });
});

describe('plainTextToHtml', () => {
  it('wraps a single line in a <p>', () => {
    expect(plainTextToHtml('hello')).toBe('<p>hello</p>');
  });

  it('converts single newlines inside a paragraph to <br>', () => {
    expect(plainTextToHtml('line1\nline2')).toBe('<p>line1<br>line2</p>');
  });

  it('treats blank lines as paragraph breaks', () => {
    expect(plainTextToHtml('para1\n\npara2')).toBe('<p>para1</p><p>para2</p>');
  });

  it('escapes HTML special characters so they cannot inject markup', () => {
    expect(plainTextToHtml('<script>alert(1)</script>')).toBe('<p>&lt;script&gt;alert(1)&lt;/script&gt;</p>');
  });

  it('handles ampersands without double-escaping', () => {
    expect(plainTextToHtml('Tom & Jerry')).toBe('<p>Tom &amp; Jerry</p>');
  });
});
