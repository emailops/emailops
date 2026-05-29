import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { useConnectivityStore } from '@/stores/connectivityStore';

/**
 * Thin top-of-app banner shown when we lose internet connectivity.
 *
 * All read paths (cached emails, local search, threads) keep working offline
 * because the SQLite DB is local — the banner exists to communicate why
 * network-dependent actions (sync, send, possibly AI) are unavailable, not to
 * block the UI.
 *
 * The AI half of the message is conditional: `llamacpp` and `ollama` run on
 * the user's device and keep working offline, so claiming "AI features are
 * paused" would be wrong. Only remote providers (OpenRouter) are
 * actually blocked by lack of internet.
 *
 * Subscribes to `useConnectivityStore` via destructuring so React re-renders
 * on transitions (see CLAUDE.md "Zustand Store Subscriptions" — `getState()`
 * reads in memo deps silently miss updates).
 */
const LOCAL_AI_PROVIDERS = new Set(['llamacpp', 'ollama']);

export function OfflineBanner() {
  const { t } = useTranslation(['common', 'notifications']);
  const { isOnline, initialized, init } = useConnectivityStore();
  const [aiIsRemote, setAiIsRemote] = useState<boolean | null>(null);

  // Idempotent: the store guards against double-init internally.
  useEffect(() => {
    void init();
  }, [init]);

  // Re-read every time we transition into "offline" — the user may have
  // changed their AI provider since the last time the banner was shown, and
  // the banner only mounts a wording-affecting query when actually visible.
  useEffect(() => {
    if (isOnline) return;
    let cancelled = false;
    void (async () => {
      try {
        const provider = await api.getPref('ai_provider');
        if (cancelled) return;
        // Default provider is `llamacpp` — anything missing is local.
        setAiIsRemote(provider != null && !LOCAL_AI_PROVIDERS.has(provider));
      } catch {
        if (!cancelled) setAiIsRemote(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [isOnline]);

  // Don't render until we have a real signal — avoids flashing "Offline" on
  // cold start before the first backend probe returns.
  if (!initialized || isOnline) return null;

  const message = aiIsRemote
    ? 'Showing cached emails. Sync, sending, and AI features are paused until you reconnect.'
    : 'Showing cached emails. Sync and sending are paused until you reconnect. Local AI still works.';

  return (
    <div
      role="status"
      aria-live="polite"
      className="bg-amber-50 border-b border-amber-200 px-4 py-2 flex items-center gap-2 text-sm text-amber-900"
    >
      <svg className="h-4 w-4 flex-shrink-0" viewBox="0 0 20 20" fill="currentColor">
        <path
          fillRule="evenodd"
          d="M10 18a8 8 0 100-16 8 8 0 000 16zM9 9a1 1 0 012 0v4a1 1 0 11-2 0V9zm1-5a1 1 0 100 2 1 1 0 000-2z"
          clipRule="evenodd"
        />
      </svg>
      <span>
        <strong>{t('common:state.offline')}.</strong> {message}
      </span>
    </div>
  );
}
