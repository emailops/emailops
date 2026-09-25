import { listen } from '@tauri-apps/api/event';
import { useEffect } from 'react';
import { useTranslation } from 'react-i18next';
import type { Activity } from '@/lib/backgroundActivity';
import { useActivityStore } from '@/stores/activityStore';
import { useChatStore } from '@/stores/chatStore';
import { useLensStore } from '@/stores/lensStore';

/** How often the status bar re-reads the task queues. */
const POLL_MS = 2000;

interface ClassificationProgressEvent {
  accountId: string;
  status: string;
  current: number;
  total: number;
}
interface EmbeddingProgressEvent {
  status: string;
  current: number;
  total: number;
}
interface ModelDownloadProgressEvent {
  modelId: string;
  downloadedBytes: number;
  totalBytes: number;
  status: string;
}

/** Poll the queues and follow the processes that report progress by event. */
function useActivityFeed() {
  const refresh = useActivityStore((s) => s.refresh);
  const setProgress = useActivityStore((s) => s.setProgress);
  useEffect(() => {
    void refresh();
    const timer = setInterval(() => void refresh(), POLL_MS);
    const unlisten = [
      listen<ClassificationProgressEvent>('classification-progress', ({ payload }) => {
        setProgress(
          'classification',
          null,
          payload.status === 'complete' ? null : { current: payload.current, total: payload.total },
        );
      }),
      listen<EmbeddingProgressEvent>('embedding-progress', ({ payload }) => {
        const done = payload.status === 'complete' || payload.status === 'error';
        setProgress('embeddings', null, done ? null : { current: payload.current, total: payload.total });
      }),
      listen<ModelDownloadProgressEvent>('model-download-progress', ({ payload }) => {
        const done = payload.status !== 'downloading';
        setProgress(
          'modelDownload',
          payload.modelId,
          done ? null : { current: payload.downloadedBytes, total: payload.totalBytes, unit: 'bytes' },
        );
      }),
    ];
    return () => {
      clearInterval(timer);
      for (const p of unlisten) void p.then((off) => off());
    };
  }, [refresh, setProgress]);
}

function useActivityText() {
  const { t } = useTranslation(['dashboard', 'chat']);
  const lenses = useLensStore((s) => s.lenses);
  return (a: Activity): string => {
    const lensName =
      a.kind === 'lensBackfill' || a.kind === 'lensRun' ? lenses.find((l) => l.id === a.target)?.name : undefined;
    let text = t(`dashboard:activity.${a.kind}` as const);
    if (lensName) text += ` «${lensName}»`;
    const p = a.progress;
    if (p) {
      if (p.unit === 'remaining') text += ` · ${t('dashboard:activity.remaining', { n: p.current })}`;
      else if (p.unit === 'bytes' && p.total) text += ` · ${Math.round((p.current / p.total) * 100)}%`;
      else if (p.total) text += ` · ${p.current}/${p.total}`;
    }
    return text;
  };
}

/** One line in the status bar for the long-running AI work in progress; a
 *  tooltip lists everything when more than one process runs. */
export function BackgroundActivityStatus() {
  const { t } = useTranslation(['chat']);
  useActivityFeed();
  const research = useChatStore((s) => s.runningResearch);
  const activities = useActivityStore((s) => s.activities);
  const describe = useActivityText();

  const lines: string[] = [];
  if (research) {
    const step = t(`chat:processing.research.${research.stage}` as const, {
      read: research.emailsRead.toLocaleString(),
      total: research.emailsTotal.toLocaleString(),
      batch: research.batch,
      batches: research.batches,
    });
    lines.push(t('chat:research.statusBar', { step }));
  }
  lines.push(...activities.map(describe));
  if (lines.length === 0) return null;

  return (
    <span
      data-testid="background-activity"
      title={lines.join('\n')}
      className="flex min-w-0 items-center gap-1.5 text-[11px] text-sky-300"
    >
      <svg className="h-3 w-3 shrink-0 animate-spin" viewBox="0 0 24 24" fill="none" aria-hidden="true">
        <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
        <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
      </svg>
      <span className="truncate">{lines[0]}</span>
      {lines.length > 1 && <span className="shrink-0 text-gray-400">+{lines.length - 1}</span>}
    </span>
  );
}
