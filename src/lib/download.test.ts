import { describe, expect, it } from 'vitest';
import { dataUrlToBase64 } from './download';

describe('dataUrlToBase64', () => {
  it('strips the data-URL prefix', () => {
    expect(dataUrlToBase64('data:application/pdf;base64,aGk=')).toBe('aGk=');
  });

  it('returns raw base64 unchanged when there is no prefix', () => {
    expect(dataUrlToBase64('aGk=')).toBe('aGk=');
  });

  it('only splits on the first comma so base64 payloads stay intact', () => {
    expect(dataUrlToBase64('data:text/plain;base64,YSxi')).toBe('YSxi');
  });
});
