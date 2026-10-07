import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import { DEFAULT_TOAST_MS, type Toast, useToastStore } from '@/stores/toastStore';

/** Above the Output bar, which publishes its height (collapsed or expanded)
 *  as `--log-panel-height`; 0 where there is no bar. */
const STACK_BOTTOM = 'calc(var(--log-panel-height, 0px) + 1rem)';

/** Renders the toast stack bottom-right; each toast auto-dismisses. */
export function ToastHost() {
  const toasts = useToastStore((s) => s.toasts);
  if (toasts.length === 0) return null;
  return (
    <div
      data-testid="toast-stack"
      className="fixed right-4 z-[60] flex flex-col gap-2 items-end"
      style={{ bottom: STACK_BOTTOM }}
    >
      {toasts.map((toast) => (
        <ToastCard key={toast.id} toast={toast} />
      ))}
    </div>
  );
}

function ToastCard({ toast }: { toast: Toast }) {
  const { t } = useTranslation(['common']);
  const dismissToast = useToastStore((s) => s.dismissToast);

  useEffect(() => {
    // Sticky toasts stay until the user closes them (X or the action button).
    if (toast.sticky) return;
    const timer = setTimeout(() => dismissToast(toast.id), toast.durationMs ?? DEFAULT_TOAST_MS);
    return () => clearTimeout(timer);
  }, [toast.id, toast.sticky, toast.durationMs, dismissToast]);

  return (
    <div className="flex items-start gap-3 pl-4 pr-2 py-2.5 bg-gray-900 text-white rounded-lg shadow-lg max-w-md">
      {/* Wraps rather than truncates: a toast can carry the fix the user needs. */}
      <span className="text-sm break-words min-w-0">{toast.message}</span>
      {toast.actionLabel && (
        <button
          type="button"
          onClick={() => {
            toast.onAction?.();
            dismissToast(toast.id);
          }}
          className="flex-shrink-0 text-sm font-medium text-primary-300 hover:text-primary-200 transition-colors"
        >
          {toast.actionLabel}
        </button>
      )}
      <button
        type="button"
        onClick={() => dismissToast(toast.id)}
        className="flex-shrink-0 p-1 text-white/50 hover:text-white transition-colors"
        aria-label={t('common:actions.close')}
      >
        <svg className="w-3.5 h-3.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
        </svg>
      </button>
    </div>
  );
}
