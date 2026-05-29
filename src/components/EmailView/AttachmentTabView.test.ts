import { describe, expect, test } from 'vitest';
import { getAttachmentIframeSandbox } from './AttachmentTabView';

describe('getAttachmentIframeSandbox', () => {
  test('returns restrictive sandbox for html attachments', () => {
    expect(getAttachmentIframeSandbox('text/html')).toBe('');
  });

  test('returns restrictive sandbox for non-html iframe previews', () => {
    expect(getAttachmentIframeSandbox('application/pdf')).toBe('');
  });
});
