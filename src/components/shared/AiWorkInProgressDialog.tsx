// Shown before a change to the AI provider, the chat model or the embedding
// model is saved while background AI work the change cuts across is running
// or queued. That work keeps the provider it started with until its batch
// ends, so the user chooses: stop it, wait for it, or change nothing.

import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { type AiChange, affectedKinds, affectedWork, sendsMailToOpenRouter, summarizeWork } from '@/lib/aiProviderWork';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';
import type { AiProviderActivity } from '@/types';

/** How often the dialog re-reads the queue while stopping or waiting. */
const POLL_MS = 1000;

interface AiWorkInProgressDialogProps {
  /** What the save is about to change. */
  change: AiChange;
  /** The work found when the save was asked for. */
  activity: AiProviderActivity;
  /** The affected work is gone: carry on with the save. */
  onProceed: () => void;
  /** Nothing is saved. */
  onCancel: () => void;
}

export function AiWorkInProgressDialog({
  change,
  activity: initial,
  onProceed,
  onCancel,
}: AiWorkInProgressDialogProps) {
  const { t } = useTranslation(['common', 'settings']);
  const addLog = useLogStore((s) => s.addLog);
  const [activity, setActivity] = useState(initial);
  const [phase, setPhase] = useState<'ask' | 'stopping' | 'waiting'>('ask');
  const [error, setError] = useState<string | null>(null);
  // Read by the polling effect, which must not restart when the parent
  // re-renders with new callbacks.
  const latest = useRef({ change, onProceed, addLog, t });
  latest.current = { change, onProceed, addLog, t };

  // Stopping or waiting: re-read the queue until nothing the change cuts
  // across is left, then carry on. Polled rather than driven by an event so
  // work that a sync queues meanwhile is seen too — and, when stopping,
  // stopped as well (the request is idempotent).
  useEffect(() => {
    if (phase === 'ask') return;
    let done = false;
    const tick = async () => {
      const { change, onProceed, addLog, t } = latest.current;
      try {
        if (phase === 'stopping') await api.cancelAiProviderWork(affectedKinds(change));
        const next = await api.getAiProviderActivity();
        if (done) return;
        if (affectedWork(change, next.items).length === 0) {
          done = true;
          onProceed();
          return;
        }
        setActivity(next);
      } catch (err) {
        if (done) return;
        done = true;
        const message = t('settings:aiWork.failed', { error: errorText(err) });
        setError(message);
        addLog('error', 'ai', message);
        setPhase('ask');
      }
    };
    void tick();
    const timer = setInterval(() => void tick(), POLL_MS);
    return () => {
      done = true;
      clearInterval(timer);
    };
  }, [phase]);

  const work = affectedWork(change, activity.items);
  const choose = (next: 'stopping' | 'waiting') => {
    setError(null);
    setPhase(next);
  };

  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center bg-black/60">
      <div className="bg-[#2d2d2e] border border-gray-600 rounded-lg p-6 shadow-xl max-w-md w-full mx-4">
        <h3 className="text-base font-semibold text-gray-100 mb-2">{t('settings:aiWork.title')}</h3>
        {error && (
          <div role="alert" className="mb-3 p-3 bg-red-900/30 border border-red-800 rounded text-red-300 text-sm">
            {error}
          </div>
        )}
        <p className="text-sm text-gray-300 mb-2">{t('settings:aiWork.body')}</p>
        <ul className="text-sm text-gray-200 mb-3 list-disc pl-5 space-y-0.5">
          {summarizeWork(work).map((line) => (
            <li key={line.kind}>
              {[
                t(`settings:aiWork.kinds.${line.kind}` as const),
                line.running && line.progress ? `${line.progress.current}/${line.progress.total}` : null,
                line.running && line.queued > 0 ? t('settings:aiWork.moreQueued', { n: line.queued }) : null,
                line.running ? null : t('settings:aiWork.queued', { n: line.queued }),
              ]
                .filter((part) => part !== null)
                .join(' · ')}
            </li>
          ))}
        </ul>
        {sendsMailToOpenRouter(activity.provider, work) && (
          <p className="text-sm text-amber-300 mb-3">{t('settings:aiWork.openRouter')}</p>
        )}
        <p className="text-sm text-gray-400">
          {phase === 'ask' && t('settings:aiWork.help')}
          {phase === 'stopping' && t('settings:aiWork.stopping')}
          {phase === 'waiting' && t('settings:aiWork.waiting')}
        </p>
        <div className="flex gap-2 justify-end mt-5">
          <button
            onClick={onCancel}
            className="px-3 py-1.5 text-sm text-gray-300 hover:text-white hover:bg-gray-700 rounded transition-colors"
          >
            {t('common:actions.cancel')}
          </button>
          {phase === 'ask' && (
            <>
              <button
                onClick={() => choose('waiting')}
                className="px-3 py-1.5 text-sm bg-gray-700 text-gray-200 rounded hover:bg-gray-600 transition-colors"
              >
                {t('settings:aiWork.wait')}
              </button>
              <button
                onClick={() => choose('stopping')}
                className="px-3 py-1.5 text-sm bg-red-600 text-white rounded hover:bg-red-500 transition-colors"
              >
                {t('settings:aiWork.stop')}
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
