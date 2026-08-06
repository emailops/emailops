// The scrolling list of log lines, shared by the desktop dock (`LogPanel`) and
// the phone's full-screen `LogView`.
//
// Extracted so the two cannot drift: a level colour or badge added in one place
// would otherwise be missing from the other, and the phone is where a log is
// read most carefully.

import { useEffect, useRef } from 'react';
import { useTranslation } from 'react-i18next';
import { useFormatters } from '@/hooks/useFormatters';
import type { LogEntry, LogLevel, LogSource } from '@/stores/logStore';

/** Options matching the log panel's fixed 24-hour HH:MM:SS time format. */
const LOG_TIME_OPTIONS: Intl.DateTimeFormatOptions = {
  hour12: false,
  hour: '2-digit',
  minute: '2-digit',
  second: '2-digit',
};

export function LevelIcon({ level }: { level: LogLevel }) {
  const { t } = useTranslation(['dashboard']);
  switch (level) {
    case 'error':
      return <span className="text-red-400 text-xs font-bold">{t('dashboard:log.levels.error')}</span>;
    case 'warn':
      return <span className="text-yellow-400 text-xs font-bold">{t('dashboard:log.levels.warn')}</span>;
    case 'success':
      return <span className="text-green-400 text-xs font-bold">{t('dashboard:log.levels.success')}</span>;
    case 'debug':
      return <span className="text-gray-500 text-xs font-bold">{t('dashboard:log.levels.debug')}</span>;
    default:
      return <span className="text-blue-400 text-xs font-bold">{t('dashboard:log.levels.info')}</span>;
  }
}

export function SourceBadge({ source }: { source: LogSource }) {
  const colors: Record<LogSource, string> = {
    sync: 'bg-indigo-900/50 text-indigo-300',
    ai: 'bg-purple-900/50 text-purple-300',
    search: 'bg-cyan-900/50 text-cyan-300',
    account: 'bg-amber-900/50 text-amber-300',
    system: 'bg-gray-700/50 text-gray-300',
    embeddings: 'bg-emerald-900/50 text-emerald-300',
    attachments: 'bg-orange-900/50 text-orange-300',
    chat: 'bg-sky-900/50 text-sky-300',
    memory: 'bg-pink-900/50 text-pink-300',
    tasks: 'bg-rose-900/50 text-rose-300',
    lens: 'bg-violet-900/50 text-violet-300',
  };

  return (
    <span className={`px-1.5 py-0.5 rounded text-[10px] font-medium uppercase tracking-wide ${colors[source]}`}>
      {source}
    </span>
  );
}

interface LogEntryListProps {
  entries: LogEntry[];
  /** Shown when there is nothing to list. */
  emptyLabel?: string;
  /** Extra classes for the scroll container — the dock pins a height, the
   *  full-screen view fills what is left. */
  className?: string;
}

export function LogEntryList({ entries, emptyLabel, className = 'flex-1' }: LogEntryListProps) {
  const fmt = useFormatters();
  const scrollRef = useRef<HTMLDivElement>(null);
  const wasAtBottomRef = useRef(true);

  // Follow the tail only while the reader is already at it. Scrolling up to
  // read an error and being yanked back down by the next line is the one thing
  // that makes a live log unusable.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const handleScroll = () => {
      wasAtBottomRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < 30;
    };
    el.addEventListener('scroll', handleScroll);
    return () => el.removeEventListener('scroll', handleScroll);
  }, []);

  useEffect(() => {
    if (wasAtBottomRef.current && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [entries]);

  return (
    // `overflow-x-hidden` with wrapping messages below, rather than a
    // horizontal scrollbar: a log is read by scanning down the left edge, and
    // one long path should not shift every other line out of view.
    <div ref={scrollRef} className={`overflow-y-auto overflow-x-hidden px-1 py-1 ${className}`}>
      {entries.length === 0 ? (
        <div className="flex h-full items-center justify-center text-gray-600">{emptyLabel}</div>
      ) : (
        entries.map((entry) => (
          <div key={entry.id} className="flex min-w-0 items-start gap-2 px-2 py-0.5 hover:bg-white/[0.03] leading-5">
            <span className="text-gray-400 shrink-0">{fmt.time(entry.timestamp / 1000, LOG_TIME_OPTIONS)}</span>
            <span className="shrink-0 w-7 text-center">
              <LevelIcon level={entry.level} />
            </span>
            <span className="shrink-0">
              <SourceBadge source={entry.source} />
            </span>
            {/* `break-all`, not `break-words`: log messages carry unbroken
                tokens (filesystem paths, URLs, ids) that word-breaking leaves
                overflowing. */}
            <span
              className={`min-w-0 break-all ${
                entry.level === 'error' ? 'text-red-300' : entry.level === 'warn' ? 'text-yellow-200' : 'text-gray-300'
              }`}
            >
              {entry.message}
            </span>
          </div>
        ))
      )}
    </div>
  );
}
