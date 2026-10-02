import { describe, expect, it } from 'vitest';
import {
  checkEncodedSignatureImage,
  MAX_SIGNATURE_IMAGE_BYTES,
  MAX_SIGNATURE_IMAGE_WIDTH,
  MAX_SIGNATURE_SOURCE_BYTES,
  planSignatureImage,
  prepareSignatureImage,
  type SignatureImageDeps,
  signatureImageHtml,
} from './signatureImage';

describe('planSignatureImage', () => {
  it.each([
    [{ type: 'image/png', bytes: 20_000, width: 300, height: 80 }, { action: 'accept' }],
    [{ type: 'image/gif', bytes: 20_000, width: 120, height: 40 }, { action: 'accept' }],
    [
      { type: 'image/png', bytes: 90_000, width: 1200, height: 400 },
      { action: 'downscale', width: 600, height: 200, outputType: 'image/png' },
    ],
    [
      { type: 'image/jpeg', bytes: 900_000, width: 3000, height: 1000 },
      { action: 'downscale', width: 600, height: 200, outputType: 'image/jpeg' },
    ],
    // Narrow but heavy: re-encoded at its own size.
    [
      { type: 'image/webp', bytes: 300_000, width: 500, height: 100 },
      { action: 'downscale', width: 500, height: 100, outputType: 'image/png' },
    ],
    [
      { type: 'image/svg+xml', bytes: 2_000, width: 100, height: 100 },
      { action: 'reject', code: 'unsupportedType' },
    ],
    [
      { type: 'application/pdf', bytes: 2_000, width: 0, height: 0 },
      { action: 'reject', code: 'unsupportedType' },
    ],
    [
      { type: 'image/png', bytes: MAX_SIGNATURE_SOURCE_BYTES + 1, width: 100, height: 100 },
      { action: 'reject', code: 'fileTooLarge' },
    ],
    [
      { type: 'image/png', bytes: 2_000, width: 0, height: 0 },
      { action: 'reject', code: 'unreadable' },
    ],
  ])('%j → %j', (input, plan) => {
    expect(planSignatureImage(input)).toEqual(plan);
  });

  it('never plans wider than the cap', () => {
    const plan = planSignatureImage({ type: 'image/png', bytes: 1, width: 5000, height: 1 });
    expect(plan.action === 'downscale' && plan.width).toBe(MAX_SIGNATURE_IMAGE_WIDTH);
    expect(plan.action === 'downscale' && plan.height).toBe(1);
  });
});

describe('checkEncodedSignatureImage', () => {
  it('accepts up to the cap and refuses beyond', () => {
    expect(checkEncodedSignatureImage(MAX_SIGNATURE_IMAGE_BYTES)).toBeNull();
    expect(checkEncodedSignatureImage(MAX_SIGNATURE_IMAGE_BYTES + 1)).toBe('tooLarge');
  });
});

describe('prepareSignatureImage', () => {
  const file = (type: string, size: number) => ({ name: 'logo', type, size }) as File;
  const deps = (over: Partial<SignatureImageDeps> = {}): SignatureImageDeps => ({
    readDataUrl: async () => 'data:image/png;base64,AAAA',
    measure: async () => ({ width: 300, height: 100 }),
    resize: async () => 'data:image/png;base64,BBBB',
    ...over,
  });

  it('keeps a small image as it is', async () => {
    expect(await prepareSignatureImage(file('image/png', 1000), deps())).toEqual({
      ok: true,
      dataUrl: 'data:image/png;base64,AAAA',
    });
  });

  it('downscales a wide image', async () => {
    const calls: unknown[] = [];
    const out = await prepareSignatureImage(
      file('image/jpeg', 1000),
      deps({
        measure: async () => ({ width: 2400, height: 600 }),
        resize: async (_src, width, height, type) => {
          calls.push([width, height, type]);
          return 'data:image/jpeg;base64,CCCC';
        },
      }),
    );
    expect(calls).toEqual([[600, 150, 'image/jpeg']]);
    expect(out).toEqual({ ok: true, dataUrl: 'data:image/jpeg;base64,CCCC' });
  });

  it('refuses an SVG without reading it', async () => {
    let read = false;
    const out = await prepareSignatureImage(
      file('image/svg+xml', 100),
      deps({
        readDataUrl: async () => {
          read = true;
          return '';
        },
      }),
    );
    expect(out).toEqual({ ok: false, code: 'unsupportedType' });
    expect(read).toBe(false);
  });

  it('refuses a result still over the size cap', async () => {
    const huge = `data:image/png;base64,${'A'.repeat(Math.ceil(((MAX_SIGNATURE_IMAGE_BYTES + 10) * 4) / 3))}`;
    const out = await prepareSignatureImage(
      file('image/png', 1000),
      deps({ measure: async () => ({ width: 900, height: 300 }), resize: async () => huge }),
    );
    expect(out).toEqual({ ok: false, code: 'tooLarge' });
  });

  it('reports a file the webview cannot decode', async () => {
    const out = await prepareSignatureImage(
      file('image/png', 1000),
      deps({
        measure: async () => {
          throw new Error('decode failed');
        },
      }),
    );
    expect(out).toEqual({ ok: false, code: 'unreadable' });
  });
});

describe('signatureImageHtml', () => {
  it('escapes the file name used as the alt text', () => {
    expect(signatureImageHtml('data:image/png;base64,AA==', 'a"b<c>.png')).toBe(
      '<p><img src="data:image/png;base64,AA==" alt="a&quot;b&lt;c&gt;.png"></p>',
    );
  });
});
