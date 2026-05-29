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
