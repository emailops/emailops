// The confirmations behind "Unsubscribe", "Block sender" and "Unblock": one
// dialog at a time, requested through `useSenderStore.openDialog` from the
// reading pane, the ⋮ menu or Settings, and rendered once at the app root.

import { open as openExternal } from '@tauri-apps/plugin-shell';
import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { Modal } from '@/components/common/Modal';
import { getSafeExternalUrl } from '@/lib/emailFormatting';
import { errorText } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';
import { type SenderDialogRequest, statusKey, useSenderStore } from '@/stores/senderStore';
import { useToastStore } from '@/stores/toastStore';

const PRIMARY =
  'rounded bg-primary-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-primary-700 disabled:opacity-50';
const DANGER = 'rounded bg-red-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-red-700 disabled:opacity-50';
const SECONDARY = 'rounded px-3 py-1.5 text-sm text-gray-300 hover:bg-gray-700';

export function SenderDialogs() {
  const dialog = useSenderStore((s) => s.dialog);
  const closeDialog = useSenderStore((s) => s.closeDialog);
  if (!dialog) return null;
  // Keyed so switching from one request to another (unsubscribe → block)
  // starts the next dialog with fresh state.
  if (dialog.type === 'unsubscribe') {
    return <UnsubscribeDialog key={`u:${dialog.emailId}`} request={dialog} onClose={closeDialog} />;
  }
  return <BlockDialog key={`${dialog.type}:${dialog.address}`} request={dialog} onClose={closeDialog} />;
}

type UnsubscribeRequest = Extract<SenderDialogRequest, { type: 'unsubscribe' }>;

function UnsubscribeDialog({ request, onClose }: { request: UnsubscribeRequest; onClose: () => void }) {
  const { t } = useTranslation(['inbox', 'common']);
  const status = useSenderStore((s) => s.statusByEmail[statusKey(request.accountId, request.emailId)]);
  const unsubscribe = useSenderStore((s) => s.unsubscribe);
  const openDialog = useSenderStore((s) => s.openDialog);
  const addLog = useLogStore((s) => s.addLog);
  const [phase, setPhase] = useState<'confirm' | 'working' | 'done'>('confirm');
  const [error, setError] = useState<string | null>(null);

  const option = status?.unsubscribe ?? null;
  if (!status || !option) return null;
  const name = request.senderName || status.address;

  const handleConfirm = async () => {
    setPhase('working');
    setError(null);
    try {
      if (option.kind === 'link') {
        // The backend validated it as https; re-checked here because this is
        // the boundary where a URL leaves the app.
        const safe = option.url ? getSafeExternalUrl(option.url) : null;
        if (!safe?.startsWith('https:')) throw new Error(t('inbox:sender.unsubscribeFailed'));
        await openExternal(safe);
      }
      await unsubscribe(request.accountId, request.emailId);
      addLog('success', 'account', `Unsubscribe requested for ${status.address}`);
      setPhase('done');
    } catch (err) {
      const message = `${t('inbox:sender.unsubscribeFailed')}: ${errorText(err)}`;
      addLog('error', 'account', message);
      setError(message);
      setPhase('confirm');
    }
  };

  const explanation =
    option.kind === 'oneClick'
      ? t('inbox:sender.unsubscribeOneClick', { target: option.target })
      : option.kind === 'mailto'
        ? t('inbox:sender.unsubscribeMailto', { target: option.target })
        : t('inbox:sender.unsubscribeLink', { target: option.target });

  if (phase === 'done') {
    return (
      <Modal
        open
        onClose={onClose}
        size="md"
        zIndex={60}
        title={t('inbox:sender.unsubscribed')}
        footer={
          <div className="flex justify-end gap-2">
            {!status.blocked && (
              <button
                type="button"
                className={SECONDARY}
                data-testid="unsubscribe-also-block"
                onClick={() => openDialog({ type: 'block', accountId: request.accountId, address: status.address })}
              >
                {t('inbox:sender.alsoBlock')}
              </button>
            )}
            <button type="button" className={PRIMARY} onClick={onClose}>
              {t('common:actions.close')}
            </button>
          </div>
        }
      >
        <p className="text-sm text-gray-300">
          {option.kind === 'link' ? t('inbox:sender.unsubscribeLinkDone') : t('inbox:sender.unsubscribeDone', { name })}
        </p>
      </Modal>
    );
  }

  return (
    <Modal
      open
      onClose={onClose}
      size="md"
      zIndex={60}
      title={t('inbox:sender.unsubscribeTitle', { name })}
      footer={
        <div className="flex justify-end gap-2">
          <button type="button" className={SECONDARY} onClick={onClose}>
            {t('common:actions.cancel')}
          </button>
          <button
            type="button"
            className={PRIMARY}
            data-testid="unsubscribe-confirm"
            disabled={phase === 'working'}
            onClick={() => void handleConfirm()}
          >
            {option.kind === 'link' ? t('inbox:sender.openPage') : t('inbox:sender.unsubscribeConfirm')}
          </button>
        </div>
      }
    >
      {error && (
        <p role="alert" className="mb-3 rounded border border-red-800 bg-red-950/50 p-2 text-sm text-red-300">
          {error}
        </p>
      )}
      <p className="text-sm text-gray-300">{explanation}</p>
    </Modal>
  );
}

type BlockRequest = Extract<SenderDialogRequest, { type: 'block' | 'unblock' }>;

function BlockDialog({ request, onClose }: { request: BlockRequest; onClose: () => void }) {
  const { t } = useTranslation(['inbox', 'common']);
  const block = useSenderStore((s) => s.block);
  const unblock = useSenderStore((s) => s.unblock);
  const addLog = useLogStore((s) => s.addLog);
  const addToast = useToastStore((s) => s.addToast);
  // Default on both ways: blocking files their mail in Spam, unblocking
  // brings it back — the two are each other's inverse.
  const [moveMail, setMoveMail] = useState(true);
  const [working, setWorking] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const isBlock = request.type === 'block';
  const { address } = request;

  const handleConfirm = async () => {
    setWorking(true);
    setError(null);
    try {
      if (isBlock) {
        const report = await block(request.accountId, address, moveMail);
        const lines = [
          report.moved > 0
            ? t('inbox:sender.blockedMoved', { address, count: report.moved })
            : t('inbox:sender.blocked', { address }),
        ];
        if (report.localOnly > 0) lines.push(t('inbox:sender.blockLocalOnly', { count: report.localOnly }));
        if (report.failed > 0) lines.push(t('inbox:sender.blockPartial', { count: report.failed }));
        addLog(report.failed > 0 ? 'error' : 'success', 'account', lines.join(' · '));
        addToast({ message: lines.join(' · '), sticky: report.failed > 0 });
      } else {
        const report = await unblock(request.accountId, address, moveMail);
        const message = t('inbox:sender.unblocked', { address });
        addLog('success', 'account', message);
        const partial = report.failed > 0 ? ` · ${t('inbox:sender.blockPartial', { count: report.failed })}` : '';
        addToast({ message: message + partial, sticky: report.failed > 0 });
      }
      onClose();
    } catch (err) {
      const message = `${isBlock ? t('inbox:sender.blockFailed') : t('inbox:sender.unblockFailed')}: ${errorText(err)}`;
      addLog('error', 'account', message);
      setError(message);
      setWorking(false);
    }
  };

  return (
    <Modal
      open
      onClose={onClose}
      size="md"
      zIndex={60}
      title={isBlock ? t('inbox:sender.blockTitle', { address }) : t('inbox:sender.unblockTitle', { address })}
      footer={
        <div className="flex justify-end gap-2">
          <button type="button" className={SECONDARY} onClick={onClose}>
            {t('common:actions.cancel')}
          </button>
          <button
            type="button"
            className={isBlock ? DANGER : PRIMARY}
            data-testid="sender-block-confirm"
            disabled={working}
            onClick={() => void handleConfirm()}
          >
            {isBlock ? t('inbox:sender.blockConfirm') : t('inbox:sender.unblockConfirm')}
          </button>
        </div>
      }
    >
      {error && (
        <p role="alert" className="mb-3 rounded border border-red-800 bg-red-950/50 p-2 text-sm text-red-300">
          {error}
        </p>
      )}
      <p className="text-sm text-gray-300">{isBlock ? t('inbox:sender.blockBody') : t('inbox:sender.unblockBody')}</p>
      <label className="mt-3 flex items-center gap-2 text-sm text-gray-300">
        <input type="checkbox" checked={moveMail} onChange={(e) => setMoveMail(e.target.checked)} />
        {isBlock ? t('inbox:sender.blockMoveExisting') : t('inbox:sender.unblockRestore')}
      </label>
    </Modal>
  );
}
