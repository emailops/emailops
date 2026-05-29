import { useTranslation } from 'react-i18next';
import type { Account, AllQueuesState, QueueStateSnapshot, TaskHistoryEntry, TaskInfo } from '@/types';
import { formatTaskLabel } from './taskLabel';

function elapsed(startedAt: number): string {
  const sec = Math.max(0, Math.floor(Date.now() / 1000 - startedAt));
  if (sec < 60) return `${sec}s`;
  const m = Math.floor(sec / 60);
  const s = sec % 60;
  return `${m}m${s}s`;
}

function formatDuration(sec: number): string {
  if (sec < 1) return '<1s';
  if (sec < 60) return `${sec}s`;
  const m = Math.floor(sec / 60);
  const s = sec % 60;
  return `${m}m${s}s`;
}

function HistoryRow({ entry, accounts }: { entry: TaskHistoryEntry; accounts: Account[] }) {
  const ok = entry.status === 'ok';
  return (
    <li className="flex items-center justify-between gap-2">
      <span className="truncate text-gray-300">{formatTaskLabel(entry.name, accounts)}</span>
      <span className="flex items-center gap-1.5 shrink-0">
        <span className="text-gray-400">{formatDuration(entry.durationSecs)}</span>
        <span
          className={`px-1 rounded text-[10px] font-semibold ${
            ok ? 'bg-emerald-900 text-emerald-300' : 'bg-red-900 text-red-300'
          }`}
        >
          {ok ? 'OK' : 'KO'}
        </span>
      </span>
    </li>
  );
}

function TaskRow({ task, accounts, suffix }: { task: TaskInfo; accounts: Account[]; suffix?: string }) {
  return (
    <li className="text-gray-200 flex justify-between gap-2">
      <span className="truncate">{formatTaskLabel(task.name, accounts)}</span>
      {suffix && <span className="text-gray-400 shrink-0">{suffix}</span>}
    </li>
  );
}

function QueueColumn({
  snapshot,
  label,
  accounts,
}: {
  snapshot: QueueStateSnapshot;
  label: string;
  accounts: Account[];
}) {
  const { t } = useTranslation(['dashboard']);
  return (
    <div className="border border-gray-800 bg-gray-900 rounded p-3 text-xs font-mono">
      <div className="flex items-center justify-between mb-2">
        <div className="text-gray-200 font-semibold">{label}</div>
        <div className="text-gray-400">concurrency {snapshot.concurrency}</div>
      </div>

      <div className="text-gray-400 mb-1">Running ({snapshot.running.length})</div>
      {snapshot.running.length === 0 ? (
        <div className="text-gray-500 italic mb-2">{t('dashboard:queues.idle')}</div>
      ) : (
        <ul className="mb-2 space-y-0.5">
          {snapshot.running.map((t) => (
            <TaskRow key={t.id} task={t} accounts={accounts} suffix={elapsed(t.startedAt)} />
          ))}
        </ul>
      )}

      <div className="text-gray-400 mb-1">Pending ({snapshot.pending.length})</div>
      {snapshot.pending.length === 0 ? (
        <div className="text-gray-500 italic">{t('dashboard:queues.empty')}</div>
      ) : (
        <ul className="space-y-0.5">
          {snapshot.pending.slice(0, 8).map((t) => (
            <TaskRow key={t.id} task={t} accounts={accounts} />
          ))}
          {snapshot.pending.length > 8 && <li className="text-gray-500 italic">+{snapshot.pending.length - 8} more</li>}
        </ul>
      )}

      <div className="text-gray-400 mb-1 mt-2">Past {snapshot.history.length}</div>
      {snapshot.history.length === 0 ? (
        <div className="text-gray-500 italic">{t('dashboard:queues.noHistory')}</div>
      ) : (
        <ul className="space-y-0.5">
          {snapshot.history.map((entry) => (
            <HistoryRow key={entry.id} entry={entry} accounts={accounts} />
          ))}
        </ul>
      )}
    </div>
  );
}

export function QueuePanel({ state, accounts }: { state: AllQueuesState; accounts: Account[] }) {
  const { t } = useTranslation(['dashboard']);
  return (
    <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-3">
      <QueueColumn snapshot={state.ai} label={t('dashboard:queues.aiInteractive')} accounts={accounts} />
      <QueueColumn snapshot={state.aiBackground} label={t('dashboard:queues.aiBackground')} accounts={accounts} />
      <QueueColumn snapshot={state.db} label={t('dashboard:queues.db')} accounts={accounts} />
      <QueueColumn snapshot={state.sync} label={t('dashboard:queues.sync')} accounts={accounts} />
    </div>
  );
}
