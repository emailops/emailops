import { describe, expect, it } from 'vitest';
import { dataUrlToBase64, decodeDataUrlText } from './download';

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

describe('decodeDataUrlText', () => {
  it('reads utf-8', () => {
    const utf8 = btoa(String.fromCharCode(...new TextEncoder().encode('año 2026')));
    expect(decodeDataUrlText(`data:text/plain;base64,${utf8}`)).toBe('año 2026');
  });

  it('reads text that is not utf-8 the way a browser reads unlabelled text', () => {
    expect(decodeDataUrlText(`data:text/html;base64,${btoa('a\xf1o 2026')}`)).toBe('año 2026');
  });

  it('is empty when the payload is not base64', () => {
    expect(decodeDataUrlText('data:text/plain;base64,%%%')).toBe('');
  });
});
