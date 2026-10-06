import { describe, expect, it, vi } from 'vitest';
import {
  buildUpdateToast,
  parseUpdateDismissal,
  sanitizeAvailableUpdate,
  shouldShowUpdateToast,
  UPDATE_REMINDER_INTERVAL_SECS,
} from './appUpdate';

// t fake: renders the key plus interpolated version so assertions can check
// both the key routing and the interpolation without loading i18next.
const t = (key: string, opts?: Record<string, string>) => (opts?.version ? `${key}:${opts.version}` : key);

const UPDATE = { version: '0.7.0', url: 'https://github.com/emailops/emailops/releases/tag/v0.7.0' };
const NOW = 1_000_000;

describe('buildUpdateToast', () => {
  it('carries the translated message and a Download action that opens the release page', () => {
    const openUrl = vi.fn();
    const toast = buildUpdateToast(UPDATE, { t, openUrl, onDismiss: vi.fn() });

    expect(toast.message).toBe('notifications:updates.available:0.7.0');
    expect(toast.actionLabel).toBe('notifications:updates.download');
    expect(openUrl).not.toHaveBeenCalled();
    toast.onAction?.();
    expect(openUrl).toHaveBeenCalledWith(UPDATE.url);
  });

  it('is sticky so it never auto-dismisses', () => {
    expect(buildUpdateToast(UPDATE, { t, openUrl: vi.fn(), onDismiss: vi.fn() }).sticky).toBe(true);
  });

  it('reports its dismissal so the reminder can be snoozed', () => {
    const onDismiss = vi.fn();
    buildUpdateToast(UPDATE, { t, openUrl: vi.fn(), onDismiss }).onDismiss?.();
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });
});

describe('shouldShowUpdateToast', () => {
  it('shows nothing when no update is available', () => {
    expect(shouldShowUpdateToast(null, null, NOW)).toBe(false);
  });

  it('shows an update the user never dismissed', () => {
    expect(shouldShowUpdateToast(UPDATE, null, NOW)).toBe(true);
  });

  it('stays quiet within 24 hours of a dismissal', () => {
    const dismissal = { version: '0.7.0', at: NOW - UPDATE_REMINDER_INTERVAL_SECS + 60 };
    expect(shouldShowUpdateToast(UPDATE, dismissal, NOW)).toBe(false);
  });

  it('shows again once 24 hours have passed since the dismissal', () => {
    const dismissal = { version: '0.7.0', at: NOW - UPDATE_REMINDER_INTERVAL_SECS };
    expect(shouldShowUpdateToast(UPDATE, dismissal, NOW)).toBe(true);
  });

  it('shows a newer release at once even if an older one was just dismissed', () => {
    const dismissal = { version: '0.6.13', at: NOW - 60 };
    expect(shouldShowUpdateToast(UPDATE, dismissal, NOW)).toBe(true);
  });

  it('shows again when the dismissal lies in the future (clock rolled back)', () => {
    const dismissal = { version: '0.7.0', at: NOW + 3_600 };
    expect(shouldShowUpdateToast(UPDATE, dismissal, NOW)).toBe(true);
  });
});

describe('parseUpdateDismissal', () => {
  it('reads the stored version and unix-seconds timestamp', () => {
    expect(parseUpdateDismissal('0.7.0', '1000000')).toEqual({ version: '0.7.0', at: 1_000_000 });
  });

  it('returns null when either pref is missing or the timestamp is not a number', () => {
    expect(parseUpdateDismissal(null, '1000000')).toBeNull();
    expect(parseUpdateDismissal('0.7.0', null)).toBeNull();
    expect(parseUpdateDismissal('0.7.0', 'soon')).toBeNull();
  });
});

describe('sanitizeAvailableUpdate', () => {
  it('returns the update for a valid github release payload', () => {
    expect(sanitizeAvailableUpdate(UPDATE)).toEqual(UPDATE);
  });

  it('returns null for malformed shapes', () => {
    const malformed: unknown[] = [
      null,
      undefined,
      'v0.7.0',
      {},
      { version: '0.7.0' },
      { url: 'https://github.com/x' },
      { version: 7, url: 'https://github.com/x' },
      { version: '0.7.0', url: 42 },
    ];
    for (const payload of malformed) {
      expect(sanitizeAvailableUpdate(payload)).toBeNull();
    }
  });

  it('returns null when the url is unsafe or not a github.com release page', () => {
    const badUrls = [
      'javascript:alert(1)',
      'file:///etc/passwd',
      'https://evil.com/emailops/releases',
      'https://github.com.evil.com/releases',
      'not a url',
    ];
    for (const url of badUrls) {
      expect(sanitizeAvailableUpdate({ version: '0.7.0', url })).toBeNull();
    }
  });
});
