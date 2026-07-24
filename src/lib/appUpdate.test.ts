import { afterEach, describe, expect, it, vi } from 'vitest';
import { handleUpdateAvailable, sanitizeAvailableUpdate, type UpdateAvailableDeps } from './appUpdate';

// t fake: renders the key plus interpolated version so assertions can check
// both the key routing and the interpolation without loading i18next.
const t = (key: string, opts?: Record<string, string>) => (opts?.version ? `${key}:${opts.version}` : key);

function makeDeps() {
  return {
    addToast: vi.fn<UpdateAvailableDeps['addToast']>(() => 1),
    t,
    openUrl: vi.fn<UpdateAvailableDeps['openUrl']>(),
    onAvailable: vi.fn<NonNullable<UpdateAvailableDeps['onAvailable']>>(),
  };
}

describe('handleUpdateAvailable', () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('shows a toast with the translated message and a Download action that opens the release page', () => {
    const deps = makeDeps();
    handleUpdateAvailable({ version: '0.7.0', url: 'https://github.com/emailops/emailops/releases/tag/v0.7.0' }, deps);

    expect(deps.addToast).toHaveBeenCalledTimes(1);
    const toast = deps.addToast.mock.calls[0]?.[0];
    expect(toast?.message).toBe('notifications:updates.available:0.7.0');
    expect(toast?.actionLabel).toBe('notifications:updates.download');

    expect(deps.openUrl).not.toHaveBeenCalled();
    toast?.onAction?.();
    expect(deps.openUrl).toHaveBeenCalledWith('https://github.com/emailops/emailops/releases/tag/v0.7.0');
  });

  it('marks the toast sticky so it never auto-dismisses', () => {
    const deps = makeDeps();
    handleUpdateAvailable({ version: '0.7.0', url: 'https://github.com/emailops/emailops/releases/tag/v0.7.0' }, deps);
    expect(deps.addToast.mock.calls[0]?.[0]?.sticky).toBe(true);
  });

  it('mirrors the validated update into onAvailable for persistent UI state', () => {
    const deps = makeDeps();
    handleUpdateAvailable({ version: '0.7.0', url: 'https://github.com/emailops/emailops/releases/tag/v0.7.0' }, deps);
    expect(deps.onAvailable).toHaveBeenCalledWith({
      version: '0.7.0',
      url: 'https://github.com/emailops/emailops/releases/tag/v0.7.0',
    });
  });

  it('does not call onAvailable for malformed or unsafe payloads', () => {
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    for (const payload of [{ version: '0.7.0' }, { version: '0.7.0', url: 'https://evil.com/x' }]) {
      const deps = makeDeps();
      handleUpdateAvailable(payload, deps);
      expect(deps.onAvailable).not.toHaveBeenCalled();
    }
    expect(consoleError).toHaveBeenCalled();
  });

  it('ignores malformed payloads without toasting', () => {
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => undefined);
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
      const deps = makeDeps();
      handleUpdateAvailable(payload, deps);
      expect(deps.addToast).not.toHaveBeenCalled();
      expect(deps.openUrl).not.toHaveBeenCalled();
    }
    expect(consoleError).toHaveBeenCalled();
  });

  it('drops the event when the url is unsafe or not a github.com release page', () => {
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => undefined);
    const badUrls = [
      'javascript:alert(1)',
      'file:///etc/passwd',
      'https://evil.com/emailops/releases',
      'https://github.com.evil.com/releases',
      'not a url',
    ];
    for (const url of badUrls) {
      const deps = makeDeps();
      handleUpdateAvailable({ version: '0.7.0', url }, deps);
      expect(deps.addToast).not.toHaveBeenCalled();
      expect(deps.openUrl).not.toHaveBeenCalled();
    }
    expect(consoleError).toHaveBeenCalled();
  });
});

describe('sanitizeAvailableUpdate', () => {
  it('returns the update for a valid github release payload', () => {
    expect(
      sanitizeAvailableUpdate({ version: '0.7.0', url: 'https://github.com/emailops/emailops/releases/tag/v0.7.0' }),
    ).toEqual({ version: '0.7.0', url: 'https://github.com/emailops/emailops/releases/tag/v0.7.0' });
  });

  it('returns null for malformed shapes and non-github urls', () => {
    expect(sanitizeAvailableUpdate(null)).toBeNull();
    expect(sanitizeAvailableUpdate({ version: '0.7.0' })).toBeNull();
    expect(sanitizeAvailableUpdate({ version: '0.7.0', url: 'https://evil.com/x' })).toBeNull();
    expect(sanitizeAvailableUpdate({ version: '0.7.0', url: 'javascript:alert(1)' })).toBeNull();
  });
});
