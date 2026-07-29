// Platform-aware keyboard-shortcut labels.
//
// Every shortcut *handler* in the app already accepts either modifier
// (`e.metaKey || e.ctrlKey`), so the keys themselves have always worked on
// Linux and Windows. The labels did not: `⌘K` was hardcoded in all four locale
// files and in the rich-text toolbar, so a Windows user was told to press a key
// their keyboard does not have.
//
// These helpers are pure and take the platform as an argument, so they can be
// tested for macOS, Windows and Linux from any host. Callers get the live value
// from `api.currentPlatform()`.

/** Symbol/word naming the primary modifier on a given platform. */
export type ModKeyLabel = '⌘' | 'Ctrl';

/** Raw Tauri platform code for macOS. */
const MACOS = 'macos';

/**
 * The primary modifier's label: the Command glyph on macOS, `Ctrl` elsewhere.
 *
 * Unknown platforms fall back to `Ctrl`, which is right for every non-Apple
 * desktop and is also the safer default in a browser test environment where the
 * platform cannot be probed.
 */
export function modKeyLabel(platform: string): ModKeyLabel {
  return platform === MACOS ? '⌘' : 'Ctrl';
}

/**
 * Render a modifier shortcut the way the platform's own UI conventions do:
 * `⌘K` on macOS (glyph, no separator), `Ctrl+K` on Windows and Linux.
 *
 * `key` is displayed verbatim, so pass it already cased the way it should read
 * (`'K'`, not `'k'`).
 */
export function formatShortcut(platform: string, key: string): string {
  const mod = modKeyLabel(platform);
  return mod === '⌘' ? `${mod}${key}` : `${mod}+${key}`;
}

/** i18n keys naming each platform's OS credential store. */
export type CredentialStoreKey =
  | 'auth:onboarding.credentialStore.macos'
  | 'auth:onboarding.credentialStore.windows'
  | 'auth:onboarding.credentialStore.linux';

/**
 * Which credential store the `keyring` crate actually writes to on a given
 * platform, as an i18n key.
 *
 * The onboarding copy used to say "macOS Keychain" unconditionally, which was
 * simply untrue for the other two platforms — and this is a privacy claim, so
 * being precise matters more here than in ordinary UI copy.
 *
 * Non-macOS, non-Windows platforms map to the Secret Service wording, matching
 * `services::keychain`'s backend selection.
 */
export function credentialStoreKey(platform: string): CredentialStoreKey {
  if (platform === MACOS) return 'auth:onboarding.credentialStore.macos';
  if (platform === 'windows') return 'auth:onboarding.credentialStore.windows';
  return 'auth:onboarding.credentialStore.linux';
}
