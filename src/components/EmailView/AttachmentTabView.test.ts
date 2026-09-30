import { describe, expect, test } from 'vitest';
import { normalizeMimeType } from '@/lib/mimeType';
import { getAttachmentIframeSandbox } from './AttachmentTabView';

describe('getAttachmentIframeSandbox', () => {
  test('sandboxes html attachments to block scripts (untrusted markup)', () => {
    expect(getAttachmentIframeSandbox('text/html')).toBe('');
  });

  // Regression: PDFs (and other binary previews) are rendered by the WebView's
  // native viewer, which a fully-restrictive `sandbox=""` blocks — leaving the
  // tab blank. They must NOT be sandboxed (undefined → attribute omitted) so the
  // viewer can render the opaque-origin data: URI.
  test('does not sandbox pdf previews so the native viewer can render them', () => {
    expect(getAttachmentIframeSandbox('application/pdf')).toBeUndefined();
  });

  // The MIME type is whatever the sender declared. Only PDF needs the native
  // viewer; anything else that can carry script (SVG, XHTML, an unknown type
  // the WebView sniffs as HTML) must stay fully sandboxed.
  test.each(['image/svg+xml', 'application/xhtml+xml', 'application/octet-stream', 'text/xml', 'image/png'])(
    'sandboxes %s',
    (mime) => {
      expect(getAttachmentIframeSandbox(mime)).toBe('');
    },
  );
});

describe('normalizeMimeType', () => {
  test('lowercases and drops parameters', () => {
    expect(normalizeMimeType(' Text/HTML; charset=utf-8 ')).toBe('text/html');
  });

  test('keeps a well-formed type', () => {
    expect(normalizeMimeType('application/pdf')).toBe('application/pdf');
    expect(normalizeMimeType('image/svg+xml')).toBe('image/svg+xml');
  });

  // It is interpolated into a data: URL, so anything that could smuggle a
  // second parameter, a comma or markup falls back to an opaque type.
  test.each(['text/html,<script>alert(1)</script>', 'no-slash', '', 'text/ html', 'text/html"onload'])(
    'falls back to application/octet-stream for %j',
    (raw) => {
      expect(normalizeMimeType(raw)).toBe('application/octet-stream');
    },
  );
});
