// The output panel as a full-screen view, for layouts with no room to dock it.
//
// `LogPanel` is shaped like a dock: it collapses, it is a fixed 176px tall, and
// its header carries desktop chrome (model selector, AI settings). On a phone
// that is the wrong shape and, more to the point, the dock is not rendered at
// all (`!isStacked` in App.tsx) — which left backend progress and errors with
// nowhere to surface on the one platform where no terminal, DB browser or dev
// tools are available either.
//
// The copy button is the reason this exists rather than a screenshot: a log is
// only useful in a bug report if it can be pasted as text.

import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { formatLogsForCopy } from '@/lib/logExport';
import { ALL_SOURCES, type LogSource, useLogStore } from '@/stores/logStore';
import { Select } from '../shared/Select';
import { LogEntryList } from './LogEntryList';

export function LogView() {
  const { t } = useTranslation(['dashboard']);
  const { entries, clear } = useLogStore();
  const [sourceFilter, setSourceFilter] = useState<LogSource | 'all'>('all');
  const [copied, setCopied] = useState(false);

  const visibleEntries = sourceFilter === 'all' ? entries : entries.filter((e) => e.source === sourceFilter);

  const copy = async () => {
    if (visibleEntries.length === 0) return;
    try {
      await navigator.clipboard.writeText(formatLogsForCopy(visibleEntries));
      setCopied(true);
      window.setTimeout(() => setCopied(false), 2000);
    } catch (error) {
      // Never silent: a copy that did nothing looks identical to a copy that
      // worked until the paste comes up empty, and this is a diagnostic tool.
      useLogStore.getState().addLog('error', 'system', `Could not copy logs: ${error}`);
    }
  };

  return (
    // `flex-1 min-w-0` is load-bearing: the parent <main> is a flex row, so
    // without them this column sizes to its content — and one long log line (a
    // model path, a URL) then sets the width, pushing the header's buttons off
    // the side of a portrait phone. `min-h-0` lets the list scroll instead of
    // growing the column past the screen.
    <div className="flex h-full min-h-0 min-w-0 flex-1 flex-col bg-[#1e1e1e] font-mono text-xs text-gray-300">
      <div className="flex shrink-0 flex-wrap items-center justify-between gap-2 border-b border-gray-700 bg-[#252526] px-3 py-2">
        <Select
          value={sourceFilter}
          onChange={setSourceFilter}
          options={[
            { value: 'all' as const, label: t('dashboard:log.allModules') },
            ...ALL_SOURCES.map((s) => ({ value: s, label: s })),
          ]}
          ariaLabel={t('dashboard:log.filterByModule')}
          size="xs"
        />
        <div className="flex items-center gap-1">
          <button
            type="button"
            onClick={copy}
            disabled={visibleEntries.length === 0}
            className="rounded px-2 py-1.5 text-[11px] text-gray-300 transition-colors active:bg-gray-600/50 disabled:opacity-40"
          >
            {copied ? t('dashboard:log.copied') : t('dashboard:log.copy')}
          </button>
          <button
            type="button"
            onClick={clear}
            className="rounded px-2 py-1.5 text-[11px] text-gray-300 transition-colors active:bg-gray-600/50"
          >
            {t('dashboard:log.clearLogs')}
          </button>
        </div>
      </div>

      <LogEntryList
        entries={visibleEntries}
        emptyLabel={entries.length === 0 ? t('dashboard:log.empty') : undefined}
        className="min-h-0 flex-1"
      />
    </div>
  );
}
