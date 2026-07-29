import { describe, expect, it } from 'vitest';
import { formatShortcut, modKeyLabel } from './platform';

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
