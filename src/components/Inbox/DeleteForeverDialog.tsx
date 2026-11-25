import { listen } from '@tauri-apps/api/event';
import { useEffect, useReducer, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { InlineError } from '@/components/common/InlineError';
import { Modal } from '@/components/common/Modal';
import * as api from '@/lib/api';
import {
  type DeleteForeverFinished,
  type DeleteForeverFolder,
  type DeleteForeverState,
  deleteForeverReducer,
  failureReasons,
  isDeleteForeverFinished,
  isDeleteForeverProgress,
} from '@/lib/deleteForever';
import { errorText } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';

export interface DeleteForeverRequest {
  accountId: string;
  folder: DeleteForeverFolder;
  /** The selected messages, or `null` to empty the whole folder. */
  emailIds: string[] | null;
}

interface DeleteForeverDialogProps {
  request: DeleteForeverRequest;
  onClose: () => void;
}

/** Reasons listed in the summary before the rest are left out. */
const MAX_REASONS = 5;

/**
 * Count → confirm → progress (with Cancel) → summary of what was removed and
 * what is still there. The run happens in the backend; this follows its
 * `delete-forever-*` events for the run id it chose. It cannot be closed
 * while the run is going: Cancel is the way out, and it stops between
 * batches.
 */
export function DeleteForeverDialog({ request, onClose }: DeleteForeverDialogProps) {
  const { t } = useTranslation(['inbox', 'common']);
  const addLog = useLogStore((s) => s.addLog);
  const { accountId, folder, emailIds } = request;
  const [state, dispatch] = useReducer(
    deleteForeverReducer,
    emailIds ? { phase: 'confirm', count: emailIds.length } : ({ phase: 'counting' } as DeleteForeverState),
  );
  // The run's events must not arrive before anyone listens for them.
  const [listening, setListening] = useState(false);

  useEffect(() => {
    if (emailIds) return;
    let stale = false;
    api.getEmailCount(accountId, folder === 'trash' ? 'deleted' : 'spam').then(
      (count) => {
        if (!stale) dispatch({ type: 'counted', count });
      },
      (error) => {
        if (stale) return;
        const message = t('inbox:deleteForever.countFailed', { detail: errorText(error) });
        addLog('error', 'sync', message);
        dispatch({ type: 'failed', message });
      },
    );
    return () => {
      stale = true;
    };
  }, [accountId, folder, emailIds, addLog, t]);

  useEffect(() => {
    let active = true;
    const progress = listen('delete-forever-progress', (event) => {
      if (isDeleteForeverProgress(event.payload)) dispatch({ type: 'progress', progress: event.payload });
    });
    const finished = listen('delete-forever-finished', (event) => {
      if (isDeleteForeverFinished(event.payload)) dispatch({ type: 'finished', report: event.payload });
      else addLog('debug', 'sync', 'Ignored a malformed delete-forever event');
    });
    void Promise.all([progress, finished]).then(() => {
      if (active) setListening(true);
    });
    return () => {
      active = false;
      void progress.then((unlisten) => unlisten());
      void finished.then((unlisten) => unlisten());
    };
  }, [addLog]);

  const running = state.phase === 'running';

  const start = () => {
    const runId = crypto.randomUUID();
    dispatch({ type: 'started', runId });
    api.deleteForever(accountId, folder, emailIds, runId).catch((error) => {
      const message = t('inbox:deleteForever.startFailed', { detail: errorText(error) });
      addLog('error', 'sync', message);
      dispatch({ type: 'failed', message });
    });
  };

  const cancel = () => {
    if (state.phase !== 'running') return;
    dispatch({ type: 'cancelRequested' });
    api.cancelDeleteForever(state.runId).catch((error) => {
      addLog('error', 'sync', t('inbox:deleteForever.cancelFailed', { detail: errorText(error) }));
    });
  };

  const close = () => {
    if (!running) onClose();
  };

  const title = emailIds ? t('inbox:deleteForever.titleSelected') : t(`inbox:deleteForever.empty.${folder}`);

  return (
    <Modal
      open
      onClose={close}
      title={title}
      size="md"
      disableBackdropClose
      disableEscape={running}
      footer={<DialogFooter state={state} listening={listening} onStart={start} onCancelRun={cancel} onClose={close} />}
    >
      <DialogBody state={state} folder={folder} />
    </Modal>
  );
}

function DialogBody({ state, folder }: { state: DeleteForeverState; folder: DeleteForeverFolder }) {
  const { t } = useTranslation(['inbox']);
  switch (state.phase) {
    case 'counting':
      return <p className="text-sm text-gray-600">{t('inbox:deleteForever.counting')}</p>;
    case 'confirm':
      return (
        <p data-testid="delete-forever-confirm-text" className="text-sm text-gray-700">
          {state.count === 0
            ? t(`inbox:deleteForever.alreadyEmpty.${folder}`)
            : folder === 'trash'
              ? t('inbox:deleteForever.confirmTrash', { count: state.count })
              : t('inbox:deleteForever.confirmSpam', { count: state.count })}
        </p>
      );
    case 'running': {
      const percent = state.total > 0 ? Math.round((state.processed / state.total) * 100) : 0;
      return (
        <div className="space-y-2">
          <p className="text-sm text-gray-700">
            {t('inbox:deleteForever.progress', { processed: state.processed, total: state.total })}
          </p>
          <div
            data-testid="delete-forever-progress"
            role="progressbar"
            aria-valuemin={0}
            aria-valuemax={state.total}
            aria-valuenow={state.processed}
            className="h-2 w-full overflow-hidden rounded bg-gray-200"
          >
            <div className="h-full bg-red-600 transition-all" style={{ width: `${percent}%` }} />
          </div>
          {state.cancelling && <p className="text-xs text-gray-500">{t('inbox:deleteForever.cancelling')}</p>}
        </div>
      );
    }
    case 'done':
      return <Summary report={state.report} />;
    case 'failed':
      return (
        <div data-testid="delete-forever-error">
          <InlineError message={state.message} />
        </div>
      );
  }
}

/** What the run did. Why it stopped comes first, so it is seen without
 *  scrolling; the list of reasons scrolls on its own. */
function Summary({ report }: { report: DeleteForeverFinished }) {
  const { t } = useTranslation(['inbox']);
  const reasons = failureReasons(report.failed, errorText);
  return (
    <div className="space-y-3">
      {report.error && (
        <div data-testid="delete-forever-error">
          <InlineError message={errorText(report.error)} />
        </div>
      )}
      <div data-testid="delete-forever-summary" className="space-y-2 text-sm text-gray-700">
        <p>{t('inbox:deleteForever.removed', { count: report.removed })}</p>
        {report.failed.length > 0 && (
          <div>
            <p>{t('inbox:deleteForever.failed', { count: report.failed.length })}</p>
            <ul className="mt-1 max-h-32 list-disc space-y-0.5 overflow-y-auto pl-5 text-xs text-gray-600">
              {reasons.slice(0, MAX_REASONS).map((reason) => (
                <li key={reason.text}>
                  {reason.count > 1
                    ? t('inbox:deleteForever.reasonCount', { reason: reason.text, count: reason.count })
                    : reason.text}
                </li>
              ))}
            </ul>
          </div>
        )}
        {report.notAttempted > 0 && (
          <p>
            {report.cancelled
              ? t('inbox:deleteForever.cancelled', { count: report.notAttempted })
              : t('inbox:deleteForever.stopped', { count: report.notAttempted })}
          </p>
        )}
      </div>
    </div>
  );
}

function DialogFooter({
  state,
  listening,
  onStart,
  onCancelRun,
  onClose,
}: {
  state: DeleteForeverState;
  listening: boolean;
  onStart: () => void;
  onCancelRun: () => void;
  onClose: () => void;
}) {
  const { t } = useTranslation(['inbox', 'common']);
  const secondary =
    'px-3 py-1.5 text-sm text-gray-700 hover:text-gray-900 hover:bg-gray-100 rounded transition-colors disabled:opacity-50';
  if (state.phase === 'running') {
    return (
      <button
        type="button"
        data-testid="delete-forever-cancel-run"
        onClick={onCancelRun}
        disabled={state.cancelling}
        className={secondary}
      >
        {t('common:actions.cancel')}
      </button>
    );
  }
  if (state.phase === 'confirm' && state.count > 0) {
    return (
      <>
        <button type="button" onClick={onClose} className={secondary}>
          {t('common:actions.cancel')}
        </button>
        <button
          type="button"
          data-testid="delete-forever-confirm"
          onClick={onStart}
          disabled={!listening}
          className="px-3 py-1.5 text-sm bg-red-600 text-white rounded hover:bg-red-500 transition-colors disabled:opacity-50"
        >
          {t('inbox:deleteForever.confirmButton')}
        </button>
      </>
    );
  }
  return (
    <button type="button" data-testid="delete-forever-close" onClick={onClose} className={secondary}>
      {state.phase === 'counting' ? t('common:actions.cancel') : t('common:actions.close')}
    </button>
  );
}
