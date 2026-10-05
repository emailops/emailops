import { getSafeExternalUrl } from '@/lib/emailFormatting';
import type { Toast } from '@/stores/toastStore';

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

/** A closed update toast stays closed this long, then comes back while the
 *  release is still newer than the running build. */
export const UPDATE_REMINDER_INTERVAL_SECS = 86_400;

/** Which release the user last closed the update toast for, and when (unix
 *  seconds). Persisted as the `app_update_dismissed_version` /
 *  `app_update_dismissed_at` prefs so the snooze survives restarts. */
export interface UpdateDismissal {
  version: string;
  at: number;
}

export interface UpdateToastDeps {
  /** i18next translator resolving the `notifications` namespace. */
  t: Translate;
  /** Opens the url in the external browser (`@tauri-apps/plugin-shell` open). */
  openUrl: (url: string) => void;
  /** Called when the toast is closed (X or Download), to snooze it. */
  onDismiss: () => void;
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

/** Rebuild a dismissal from its two prefs; null unless both are usable. */
export function parseUpdateDismissal(version: string | null, at: string | null): UpdateDismissal | null {
  if (!version || !at || !/^\d+$/.test(at)) return null;
  return { version, at: Number(at) };
}

/**
 * Whether the update toast should be on screen now. An update is announced
 * until the user installs it: closing the toast snoozes it for
 * `UPDATE_REMINDER_INTERVAL_SECS`, and a release newer than the dismissed
 * one is announced at once.
 */
export function shouldShowUpdateToast(
  update: UpdateAvailablePayload | null,
  dismissal: UpdateDismissal | null,
  nowSecs: number,
): boolean {
  if (!update) return false;
  if (!dismissal || dismissal.version !== update.version) return true;
  // A dismissal in the future means the clock rolled back; don't let a bad
  // timestamp silence the reminder.
  return dismissal.at > nowSecs || nowSecs - dismissal.at >= UPDATE_REMINDER_INTERVAL_SECS;
}

/**
 * The update toast: sticky (an update notice must not vanish after 8
 * seconds), with a Download action opening the release page and an
 * `onDismiss` hook that snoozes the reminder.
 */
export function buildUpdateToast(update: UpdateAvailablePayload, deps: UpdateToastDeps): Omit<Toast, 'id'> {
  return {
    message: deps.t('notifications:updates.available', { version: update.version }),
    actionLabel: deps.t('notifications:updates.download'),
    onAction: () => deps.openUrl(update.url),
    onDismiss: deps.onDismiss,
    sticky: true,
  };
}
