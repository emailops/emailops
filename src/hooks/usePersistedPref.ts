// Persist a piece of UI state to the backend `user_preferences` SQLite table.
//
// Solves a race that recurs every time someone writes ad-hoc load/save effects:
// the "save" effect fires on mount with the default value and can land in
// SQLite before the async "load" effect reads the previously stored value,
// wiping out the user's preference.
//
// Usage:
//   const [layout, setLayout] = usePersistedPref<InboxLayout>(
//     'inbox_layout', 'split',
//     { parse: (raw) => (raw === 'split' || raw === 'full-width' ? raw : null) },
//   );
//
// The returned setter is a drop-in replacement for useState's — including the
// functional form `setLayout(prev => ...)`. Writes are only persisted once the
// initial load has completed, so the default never overwrites a stored value.
//
// The third tuple element (`isLoaded`) is rarely needed but exposed for call
// sites that want to delay rendering until the stored value is applied.

import { type Dispatch, type SetStateAction, useCallback, useEffect, useRef, useState } from 'react';
import * as api from '@/lib/api';
import { useLogStore } from '@/stores/logStore';

export interface UsePersistedPrefOptions<T> {
  /** Parse the raw string from SQLite into T. Return null to reject a malformed value. Defaults to JSON.parse. */
  parse?: (raw: string) => T | null;
  /** Serialize T to a string for SQLite. Defaults to JSON.stringify. */
  serialize?: (value: T) => string;
}

export function usePersistedPref<T>(
  key: string,
  initial: T,
  options: UsePersistedPrefOptions<T> = {},
): readonly [T, Dispatch<SetStateAction<T>>, boolean] {
  const { parse = defaultParse<T>, serialize = defaultSerialize<T> } = options;
  const [value, setValue] = useState<T>(initial);
  const loadedRef = useRef(false);
  const [isLoaded, setIsLoaded] = useState(false);
  const addLog = useLogStore((s) => s.addLog);

  // Load once on mount. We intentionally do not include `key` in deps — the
  // pref key is expected to be stable for a given call site.
  useEffect(() => {
    let cancelled = false;
    api
      .getPref(key)
      .then((raw) => {
        if (cancelled) return;
        if (raw != null) {
          try {
            const parsed = parse(raw);
            if (parsed !== null && parsed !== undefined) {
              setValue(parsed);
            }
          } catch {
            // Malformed pref — keep the default, leave the SQLite row as-is so
            // the next save overwrites it.
          }
        }
        loadedRef.current = true;
        setIsLoaded(true);
      })
      .catch((err) => {
        addLog('error', 'system', `Failed to load pref "${key}": ${err}`);
        // Still mark as loaded so the user's edits start persisting; otherwise
        // a transient read failure would silently disable persistence forever.
        loadedRef.current = true;
        setIsLoaded(true);
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);

  // Save on change — but only after the initial load has completed, so the
  // default never overwrites a stored value.
  useEffect(() => {
    if (!loadedRef.current) return;
    try {
      const serialized = serialize(value);
      api.setPref(key, serialized).catch((err) => {
        addLog('error', 'system', `Failed to save pref "${key}": ${err}`);
      });
    } catch (err) {
      addLog('error', 'system', `Failed to serialize pref "${key}": ${err}`);
    }
    // `serialize` is expected to be stable (defined at module scope or via
    // useCallback). We do not include it in deps to avoid churn from inline
    // closures; call sites should memoize if they close over reactive state.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, value, addLog]);

  const set = useCallback<Dispatch<SetStateAction<T>>>((next) => {
    setValue(next);
  }, []);

  return [value, set, isLoaded] as const;
}

function defaultParse<T>(raw: string): T | null {
  return JSON.parse(raw) as T;
}

function defaultSerialize<T>(value: T): string {
  return JSON.stringify(value);
}
