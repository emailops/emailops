import { describe, expect, it } from 'vitest';
import { docxToPreviewHtml } from '@/lib/docxPreview';
import { bytesFromBase64 } from '@/lib/yjsBytes';

// A synthetic Word file: a bold heading, a two-row table, an embedded PNG and
// an external hyperlink (built with Python's zipfile; no real document).
const FIXTURE_DOCX_B64 = [
  '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',
  'bmdQSwUGAAAAAAUABQBGAQAAxAUAAAAA',
].join('');

function fixture(): ArrayBuffer {
  const bytes = bytesFromBase64(FIXTURE_DOCX_B64);
  return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.length) as ArrayBuffer;
}

describe('docxToPreviewHtml', () => {
  it('renders the text, the table and the styles of the document', async () => {
    const html = await docxToPreviewHtml(fixture());
    const doc = new DOMParser().parseFromString(html, 'text/html');
    expect(doc.body.textContent).toContain('Quarterly review');
    expect([...doc.querySelectorAll('td')].map((td) => td.textContent)).toEqual(['Owner', 'Due', 'Dana', '9 May']);
    expect(doc.querySelector('style')).not.toBeNull();
  });

  it('shows no text of its own outside the document, such as the style comments of the library', async () => {
    // docx-preview puts an HTML comment before each stylesheet; it must not
    // reach the page as text.
    const doc = new DOMParser().parseFromString(await docxToPreviewHtml(fixture()), 'text/html');
    const wrapper = doc.querySelector('.docx-wrapper');
    expect(wrapper).not.toBeNull();
    const outside = [...doc.body.childNodes].filter((n) => n !== wrapper && n.nodeName !== 'STYLE');
    expect(outside.map((n) => n.textContent).join('')).toBe('');
  });

  it('inlines images as data: URLs, which the sandboxed frame can load, never blob: URLs', async () => {
    // The preview renders in a null-origin iframe: a blob: URL minted by the
    // app's origin would not load there.
    const html = await docxToPreviewHtml(fixture());
    const img = new DOMParser().parseFromString(html, 'text/html').querySelector('img');
    expect(img?.getAttribute('src')).toMatch(/^data:image\/png;base64,/);
    expect(html).not.toContain('blob:');
  });

  it('keeps an external link as a plain href for the frame to route', async () => {
    const html = await docxToPreviewHtml(fixture());
    const link = new DOMParser().parseFromString(html, 'text/html').querySelector('a');
    expect(link?.getAttribute('href')).toBe('https://example.com/project');
  });

  it('rejects bytes that are not a Word file', async () => {
    await expect(docxToPreviewHtml(new TextEncoder().encode('not a zip').buffer as ArrayBuffer)).rejects.toThrow();
  });
});
