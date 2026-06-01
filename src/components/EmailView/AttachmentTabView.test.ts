import { describe, expect, test } from 'vitest';
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
});
