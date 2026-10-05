import { describe, expect, it } from 'vitest';
import { isDocDrag, readDocDrag, writeDocDrag } from './docDrag';

/** A minimal DataTransfer: jsdom has none. */
function transfer(): DataTransfer {
  const data = new Map<string, string>();
  return {
    setData: (type: string, value: string) => void data.set(type, value),
    getData: (type: string) => data.get(type) ?? '',
    get types() {
      return [...data.keys()];
    },
    effectAllowed: 'all',
  } as unknown as DataTransfer;
}

describe('docDrag', () => {
  it('carries a document id from a row to a folder', () => {
    const dt = transfer();
    writeDocDrag(dt, 'doc-1');
    expect(isDocDrag(dt)).toBe(true);
    expect(readDocDrag(dt)).toBe('doc-1');
  });

  it('ignores drags that are not documents', () => {
    const dt = transfer();
    dt.setData('text/plain', 'doc-1');
    expect(isDocDrag(dt)).toBe(false);
    expect(readDocDrag(dt)).toBeNull();
  });
});
