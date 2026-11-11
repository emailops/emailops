import { describe, expect, test } from 'vitest';
import { normalizeMimeType } from '@/lib/mimeType';

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
