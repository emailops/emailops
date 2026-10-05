import { describe, expect, it } from 'vitest';
import type { SharedDoc } from '@/types';
import { DOC_REF_MIME, eoDocAttachment, isEoDocAttachment } from './eoDocAttachment';

const doc = { id: '6f1c2a7e-3b4d-4e5f-8a9b-0c1d2e3f4a5b', title: 'Plan' } as SharedDoc;

describe('eoDocAttachment', () => {
  it('is a placeholder naming the document, which the backend swaps at send time', () => {
    const att = eoDocAttachment(doc);
    expect(att.mimeType).toBe(DOC_REF_MIME);
    expect(atob(att.data)).toBe(doc.id);
    expect(att.filename).toBe('Plan.eodoc');
    expect(isEoDocAttachment(att)).toBe(true);
    expect(isEoDocAttachment({ ...att, mimeType: 'application/pdf' })).toBe(false);
  });
});
