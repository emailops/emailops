import { describe, expect, it } from 'vitest';
import {
  composeSurfaceClasses,
  credentialStoreKey,
  formatShortcut,
  isMobilePlatform,
  modKeyLabel,
  shouldUseStackedLayout,
} from './platform';

describe('modKeyLabel', () => {
  it('uses the Command glyph on macOS', () => {
    expect(modKeyLabel('macos')).toBe('⌘');
  });

  it('uses Ctrl on Windows and Linux', () => {
    expect(modKeyLabel('windows')).toBe('Ctrl');
    expect(modKeyLabel('linux')).toBe('Ctrl');
  });

  it('falls back to Ctrl for an unknown or unavailable platform', () => {
    // Reached in a browser/test environment where the Tauri OS plugin cannot
    // be queried. Ctrl is right for every non-Apple desktop.
    expect(modKeyLabel('')).toBe('Ctrl');
    expect(modKeyLabel('freebsd')).toBe('Ctrl');
  });
});

describe('formatShortcut', () => {
  it('follows each platform’s own convention', () => {
    // macOS glyphs are written without a separator; Windows and Linux use `+`.
    const cases: Array<[string, string, string]> = [
      ['macos', 'K', '⌘K'],
      ['macos', 'F', '⌘F'],
      ['windows', 'K', 'Ctrl+K'],
      ['linux', 'F', 'Ctrl+F'],
      ['linux', 'B', 'Ctrl+B'],
    ];
    for (const [platform, key, expected] of cases) {
      expect(formatShortcut(platform, key)).toBe(expected);
    }
  });

  it('never emits the Command glyph off macOS', () => {
    // The regression this module exists to prevent.
    for (const platform of ['windows', 'linux', '', 'freebsd']) {
      expect(formatShortcut(platform, 'K')).not.toContain('⌘');
    }
  });

  it('renders the key verbatim', () => {
    expect(formatShortcut('macos', '0')).toBe('⌘0');
    expect(formatShortcut('windows', 'Enter')).toBe('Ctrl+Enter');
  });
});

describe('isMobilePlatform', () => {
  it('recognises iOS', () => {
    expect(isMobilePlatform('ios')).toBe(true);
  });

  it('recognises Android, so the same branches serve a future Android build', () => {
    expect(isMobilePlatform('android')).toBe(true);
  });

  it('treats every desktop platform as non-mobile', () => {
    for (const platform of ['macos', 'windows', 'linux']) {
      expect(isMobilePlatform(platform)).toBe(false);
    }
  });

  it('defaults to non-mobile when the platform cannot be probed', () => {
    // A blank platform means the Tauri OS plugin was unavailable. Desktop is
    // the safer default: it keeps the full-featured layout rather than
    // silently degrading a desktop user to the phone UI.
    expect(isMobilePlatform('')).toBe(false);
    expect(isMobilePlatform('freebsd')).toBe(false);
  });
});

describe('credentialStoreKey', () => {
  it('names each desktop platform’s real credential store', () => {
    expect(credentialStoreKey('macos')).toBe('auth:onboarding.credentialStore.macos');
    expect(credentialStoreKey('windows')).toBe('auth:onboarding.credentialStore.windows');
    expect(credentialStoreKey('linux')).toBe('auth:onboarding.credentialStore.linux');
  });

  it('names iOS Keychain Services rather than falling through to the keyring wording', () => {
    // This is a privacy claim shown during onboarding, so it has to be true:
    // on iOS the `keyring` crate writes to Keychain Services, not to a Linux
    // Secret Service daemon, which is what the fallback branch would have said.
    expect(credentialStoreKey('ios')).toBe('auth:onboarding.credentialStore.ios');
  });
});

describe('shouldUseStackedLayout', () => {
  it('always stacks on a mobile platform, regardless of viewport width', () => {
    // An iPad in landscape is wide, but the interaction model is still touch,
    // and the split layout's hover affordances have no touch equivalent.
    expect(shouldUseStackedLayout('ios', 1366)).toBe(true);
    expect(shouldUseStackedLayout('ios', 390)).toBe(true);
  });

  it('stacks on a desktop platform only when the window is genuinely narrow', () => {
    expect(shouldUseStackedLayout('macos', 600)).toBe(true);
    expect(shouldUseStackedLayout('macos', 1200)).toBe(false);
  });

  it('uses the split layout at exactly the breakpoint', () => {
    // 768 is the boundary: at the breakpoint there is room for both panes.
    expect(shouldUseStackedLayout('macos', 768)).toBe(false);
    expect(shouldUseStackedLayout('macos', 767)).toBe(true);
  });
});

describe('composeSurfaceClasses', () => {
  it('keeps the desktop compose window a centered, rounded card', () => {
    const { panel } = composeSurfaceClasses(false);
    expect(panel).toContain('max-w-2xl');
    expect(panel).toContain('max-h-[90vh]');
    expect(panel).toContain('rounded-xl');
  });

  it('fills the whole screen when stacked', () => {
    // A 2xl-capped card with `mx-4` margins wastes the little width a phone
    // has, and `max-h-[90vh]` leaves a strip of dimmed backdrop the user can
    // tap by accident while typing. Compose is the whole screen instead.
    const { panel } = composeSurfaceClasses(true);
    expect(panel).toContain('w-full');
    expect(panel).toContain('h-full');
    expect(panel).not.toContain('max-w-2xl');
    expect(panel).not.toContain('max-h-[90vh]');
    expect(panel).not.toContain('rounded-xl');
  });

  it('insets the full-screen surface from the notch and home indicator', () => {
    // The compose modal is portalled outside `#root`, so it does not inherit
    // the app's safe-area padding and would otherwise run under the status bar.
    const { overlay } = composeSurfaceClasses(true);
    expect(overlay).toContain('env(safe-area-inset-top)');
    expect(overlay).toContain('env(safe-area-inset-bottom)');
    expect(composeSurfaceClasses(false).overlay).not.toContain('safe-area-inset');
  });
});
