import type { ReactNode } from 'react';

/**
 * Inline error/warning bubble — dark red, no dismiss button. For per-section
 * validation errors, save failures, etc. Distinct from the dismissible
 * `ErrorBanner` used for app-level / network errors.
 *
 * Renders nothing when `message` is falsy so callers can pass state directly:
 *   <InlineError message={error} />
 */
interface InlineErrorProps {
  message: string | null | undefined;
  /** Optional secondary action (e.g. "Retry"). */
  children?: ReactNode;
  className?: string;
}

export function InlineError({ message, children, className }: InlineErrorProps) {
  if (!message) return null;
  return (
    <div
      className={`rounded-lg border border-red-800 bg-red-900/20 px-3 py-2 text-sm text-red-300 ${className ?? ''}`}
      role="alert"
    >
      <div className="flex items-start justify-between gap-3">
        <span className="min-w-0 flex-1 break-words">{message}</span>
        {children}
      </div>
    </div>
  );
}
