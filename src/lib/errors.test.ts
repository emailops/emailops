import { beforeAll, describe, expect, it } from 'vitest';

import { initI18n } from '../i18n';
import { errorText, isAppErrorPayload } from './errors';

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
