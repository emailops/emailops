import { describe, expect, it } from 'vitest';
import { bytesFromBase64, bytesToBase64 } from './yjsBytes';

describe('yjs bytes over the Tauri boundary', () => {
  it('round-trips every byte value', () => {
    const bytes = Uint8Array.from({ length: 256 }, (_, i) => i);
    expect(bytesFromBase64(bytesToBase64(bytes))).toEqual(bytes);
  });

  it('handles a document larger than one argument list', () => {
    const bytes = new Uint8Array(300_000).fill(7);
    expect(bytesFromBase64(bytesToBase64(bytes))).toEqual(bytes);
  });

  it('matches the standard alphabet the backend decodes', () => {
    expect(bytesToBase64(Uint8Array.from([0xfb, 0xff]))).toBe('+/8=');
  });
});
