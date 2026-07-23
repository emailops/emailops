import { i18n } from '../i18n';

/**
 * Wire shape of a Rust `AppError` after it crosses the Tauri command boundary.
 * Mirrors `src-tauri/src/models/error.rs`'s `Serialize` impl: a stable `code`,
 * structured `params` for interpolation, and a pre-rendered English `message`
 * used as the fallback when no translation exists for the code.
 */
export interface AppErrorPayload {
  code: string;
  params: Record<string, string>;
  message: string;
}

export function isAppErrorPayload(e: unknown): e is AppErrorPayload {
  if (typeof e !== 'object' || e === null) return false;
  const o = e as Record<string, unknown>;
  return typeof o.code === 'string' && typeof o.message === 'string';
}

/** Error codes that mean "the account needs re-consent", not a transient failure. */
const AUTH_ERROR_CODES = new Set(['auth', 'oauth', 'needs_reauth', 'calendar_permission_denied']);

/**
 * Whether a failure is auth-class (expired/revoked consent, missing scopes)
 * rather than transient. Callers pass the already-normalized `message`
 * (usually `errorText(e)`) so string-shaped provider errors are classified too.
 */
export function isAuthError(e: unknown, message: string): boolean {
  if (isAppErrorPayload(e) && AUTH_ERROR_CODES.has(e.code)) return true;
  return /auth|consent|token|sign.?in/i.test(message);
}

/**
 * Normalize any thrown value into a localized, user-facing message.
 *
 * - Backend `AppError` payloads are translated via `errors:codes.<code>`,
 *   interpolating `params`, and fall back to the backend `message` when the
 *   code has no translation.
 * - `Error` instances surface their `.message`; strings pass through.
 *
 * Uses the i18next singleton directly so it works outside React (Zustand
 * stores, plain async helpers). Components that re-render on language change
 * should prefer the `t()` hook with `errors:codes.<code>` where they hold the
 * structured payload.
 */
export function errorText(e: unknown): string {
  if (isAppErrorPayload(e)) {
    return i18n.t(`errors:codes.${e.code}`, {
      ...e.params,
      defaultValue: e.message,
    });
  }
  if (e instanceof Error) return e.message;
  if (typeof e === 'string') return e;
  return String(e);
}
