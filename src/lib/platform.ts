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

/** Raw Tauri platform code for iOS. */
const IOS = 'ios';

/** Raw Tauri platform code for Android. */
const ANDROID = 'android';

/**
 * Viewport width (px) at and above which two panes fit side by side.
 *
 * Matches Tailwind's `md` breakpoint, which the rest of the UI already uses.
 */
const SPLIT_LAYOUT_MIN_WIDTH = 768;

/**
 * Whether this platform is a touch-first mobile OS.
 *
 * Android is included even though only iOS is built today: every branch that
 * keys off this answer (stacked layout, no hover affordances, no loopback
 * OAuth, no local file dialogs) is true for both, and a future Android target
 * should not have to revisit each call site.
 *
 * An unknown or empty platform is treated as desktop. That is the safer
 * default — mistaking a desktop for a phone strips the multi-pane layout from
 * a user who has room for it, whereas the reverse merely leaves a phone
 * showing a layout its viewport width will collapse anyway.
 */
export function isMobilePlatform(platform: string): boolean {
  return platform === IOS || platform === ANDROID;
}

/**
 * Whether the UI should stack its panes into a single-column navigation
 * (list → thread → compose) instead of showing them side by side.
 *
 * Two independent reasons to stack, hence two arguments:
 *
 *  - **Touch.** A mobile platform always stacks, even on a wide iPad, because
 *    the split layout's row-hover actions and drag-to-folder gestures have no
 *    touch equivalent. Width is not the deciding factor there.
 *  - **Width.** A desktop window dragged narrow stacks too, which is ordinary
 *    responsive behaviour and was previously absent entirely — there is not a
 *    single `@media` query in `src/`.
 */
export function shouldUseStackedLayout(platform: string, viewportWidth: number): boolean {
  if (isMobilePlatform(platform)) return true;
  return viewportWidth < SPLIT_LAYOUT_MIN_WIDTH;
}

/** i18n keys naming each platform's OS credential store. */
export type CredentialStoreKey =
  | 'auth:onboarding.credentialStore.macos'
  | 'auth:onboarding.credentialStore.windows'
  | 'auth:onboarding.credentialStore.linux'
  | 'auth:onboarding.credentialStore.ios';

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
  // iOS must be checked explicitly: `keyring` writes to iOS Keychain Services
  // there, so without this branch onboarding would fall through and promise a
  // "system keyring", which is the Linux Secret Service wording and simply
  // untrue on a phone.
  if (platform === IOS) return 'auth:onboarding.credentialStore.ios';
  return 'auth:onboarding.credentialStore.linux';
}
