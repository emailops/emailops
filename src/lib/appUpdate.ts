import { getSafeExternalUrl } from '@/lib/emailFormatting';

/**
 * Payload of the backend `app-update-available` event. Mirrors
 * `services::updates::UpdateAvailableEvent` on the Rust side.
 */
export interface UpdateAvailablePayload {
  /** Normalized dotted version, e.g. "0.7.0". */
  version: string;
  /** GitHub release page URL to open in the external browser. */
  url: string;
}

/** The exact `notifications` namespace keys this module resolves. Typing
 *  `Translate` against this literal union (rather than plain `string`) keeps
 *  it compatible with i18next's key-typed `t`. */
type UpdateKey = 'notifications:updates.available' | 'notifications:updates.download';

/** Minimal shape of the i18next translator this module needs. */
type Translate = (key: UpdateKey, options?: Record<string, string>) => string;

export interface UpdateAvailableDeps {
  /** `useToastStore.getState().addToast`. */
  addToast: (toast: { message: string; actionLabel?: string; onAction?: () => void; sticky?: boolean }) => number;
  /** i18next translator resolving the `notifications` namespace. */
  t: Translate;
  /** Opens the url in the external browser (`@tauri-apps/plugin-shell` open). */
  openUrl: (url: string) => void;
  /** Mirror the validated update into persistent UI state (the sidebar
   *  download link), so the notice outlives the toast. */
  onAvailable?: (update: UpdateAvailablePayload) => void;
}

/**
 * Validate an update payload (backend event or `get_available_update` result)
 * before any UI trusts it. Returns `null` — drop the update entirely — unless
 * the shape is well-formed AND the url is an https github.com link: a
 * download link must never hand an attacker-shaped URL to the OS opener.
 */
export function sanitizeAvailableUpdate(payload: unknown): UpdateAvailablePayload | null {
  const p = payload as UpdateAvailablePayload | null | undefined;
  if (!p || typeof p !== 'object' || typeof p.version !== 'string' || typeof p.url !== 'string') {
    return null;
  }
  const safeUrl = getSafeExternalUrl(p.url);
  if (!safeUrl || new URL(safeUrl).hostname !== 'github.com') {
    return null;
  }
  return { version: p.version, url: safeUrl };
}

/**
 * Pure handler for a single `app-update-available` event payload.
 *
 * Extracted from `App.tsx`'s listener (same pattern as `chatToolEffects.ts`)
 * so the validation + toast routing is testable without the Tauri runtime.
 * The toast is sticky — an update notice must not vanish after 8 seconds —
 * and the update is also mirrored into `onAvailable` for the persistent
 * sidebar link.
 */
export function handleUpdateAvailable(payload: unknown, deps: UpdateAvailableDeps): void {
  const update = sanitizeAvailableUpdate(payload);
  if (!update) {
    console.error('Ignoring malformed or unsafe app-update-available payload', payload);
    return;
  }
  deps.onAvailable?.(update);
  deps.addToast({
    message: deps.t('notifications:updates.available', { version: update.version }),
    actionLabel: deps.t('notifications:updates.download'),
    onAction: () => deps.openUrl(update.url),
    sticky: true,
  });
}
