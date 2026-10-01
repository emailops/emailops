import { beforeAll, describe, expect, it } from 'vitest';

import { initI18n } from '../i18n';
import { errorText, isAppErrorPayload, isAuthError, isDataPolicyError } from './errors';

beforeAll(async () => {
  await initI18n('en');
});

describe('isAppErrorPayload', () => {
  it('accepts the backend {code, params, message} shape', () => {
    expect(isAppErrorPayload({ code: 'sync', params: {}, message: 'boom' })).toBe(true);
  });

  it('rejects plain strings, Errors, null, and arbitrary objects', () => {
    expect(isAppErrorPayload('nope')).toBe(false);
    expect(isAppErrorPayload(new Error('x'))).toBe(false);
    expect(isAppErrorPayload(null)).toBe(false);
    expect(isAppErrorPayload({ message: 'no code' })).toBe(false);
  });
});

describe('isDataPolicyError', () => {
  it('recognises only the backend data-policy refusal', () => {
    expect(isDataPolicyError({ code: 'ai_data_policy', params: { model: 'vendor/embed' }, message: 'x' })).toBe(true);
    expect(isDataPolicyError({ code: 'ai', params: { detail: 'data policy' }, message: 'x' })).toBe(false);
    expect(isDataPolicyError('No endpoints found matching your data policy')).toBe(false);
  });
});

describe('isAuthError', () => {
  const cases: Array<{ name: string; e: unknown; message: string; expected: boolean }> = [
    { name: 'needs_reauth code', e: { code: 'needs_reauth', params: {}, message: 'x' }, message: 'x', expected: true },
    { name: 'auth code', e: { code: 'auth', params: {}, message: 'x' }, message: 'x', expected: true },
    { name: 'oauth code', e: { code: 'oauth', params: {}, message: 'x' }, message: 'x', expected: true },
    {
      name: 'other code, plain message',
      e: { code: 'sync', params: {}, message: 'boom' },
      message: 'boom',
      expected: false,
    },
    { name: 'auth-flavored message text', e: new Error('invalid token'), message: 'invalid token', expected: true },
    { name: '"sign in" message text', e: 'Please sign-in again', message: 'Please sign-in again', expected: true },
    { name: 'consent message text', e: 'consent required', message: 'consent required', expected: true },
    { name: 'unrelated failure', e: new Error('network down'), message: 'network down', expected: false },
  ];
  it.each(cases)('$name → $expected', ({ e, message, expected }) => {
    expect(isAuthError(e, message)).toBe(expected);
  });
});

describe('errorText', () => {
  it('localizes a known code, interpolating params', () => {
    const msg = errorText({ code: 'sync', params: { detail: 'Gmail 503' }, message: 'Sync error: Gmail 503' });
    expect(msg).toBe('Sync failed: Gmail 503');
  });

  it('renders parameterless codes', () => {
    expect(errorText({ code: 'ai_disabled', params: {}, message: 'AI off' })).toBe(
      'AI is disabled. Enable it in Settings.',
    );
  });

  it('falls back to the backend message for an unmapped code', () => {
    expect(errorText({ code: 'totally_new_code', params: {}, message: 'raw backend text' })).toBe('raw backend text');
  });

  it('passes Error instances through by message', () => {
    expect(errorText(new Error('plain error'))).toBe('plain error');
  });

  it('passes strings through unchanged', () => {
    expect(errorText('already a string')).toBe('already a string');
  });

  it('never produces "[object Object]" for the new error shape', () => {
    const msg = errorText({ code: 'needs_reauth', params: { accountId: 'acct-1' }, message: 'x' });
    expect(msg).not.toContain('[object Object]');
  });
});

describe('errorText for errors whose detail stays in the backend', () => {
  // The backend drops raw library/OS detail (SQL text, file paths, HTTP
  // internals) from these codes before they reach the webview (CASA/DASA
  // 1.8.1), so no locale may leave an empty `{{detail}}` slot in their text.
  const INTERNAL_CODES = ['database', 'http', 'json', 'io', 'keyring'];

  it.each(['en', 'es', 'fr', 'de'])('no %s template for an internal code interpolates detail', async (lng) => {
    const codes = (await import(`../locales/${lng}/errors.json`)).default.codes as Record<string, string>;
    for (const code of INTERNAL_CODES) {
      expect(codes[code], `${lng}:${code}`).toBeTruthy();
      expect(codes[code], `${lng}:${code}`).not.toContain('{{detail}}');
    }
  });
});
