import { describe, expect, it } from 'vitest';
import { isDocxAttachment } from '@/lib/mimeType';

const DOCX_MIME = 'application/vnd.openxmlformats-officedocument.wordprocessingml.document';

describe('isDocxAttachment', () => {
  it.each([
    [DOCX_MIME, 'minutes.bin', true],
    [DOCX_MIME.toUpperCase(), 'minutes', true],
    // Mail clients often send Word files as a generic type; the name decides.
    ['application/octet-stream', 'Minutes.DOCX', true],
    // The old binary format and macro-enabled files are not read in the app.
    ['application/msword', 'minutes.doc', false],
    ['application/vnd.ms-word.document.macroenabled.12', 'minutes.docm', false],
    ['application/pdf', 'minutes.pdf', false],
    ['application/octet-stream', 'minutes.docx.exe', false],
  ])('%s named %s → %s', (mimeType, filename, expected) => {
    expect(isDocxAttachment(mimeType, filename)).toBe(expected);
  });
});
