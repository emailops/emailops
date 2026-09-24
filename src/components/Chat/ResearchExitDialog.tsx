import { useTranslation } from 'react-i18next';
import { Modal } from '@/components/common/Modal';
import * as api from '@/lib/api';
import { useChatStore } from '@/stores/chatStore';

/** Asks before quitting while a research run reads: the backend held the
 *  close / Cmd+Q (`research-exit-requested`), and quitting loses the run. */
export function ResearchExitDialog() {
  const { t } = useTranslation(['chat']);
  const open = useChatStore((s) => s.researchExitRequested);
  const running = useChatStore((s) => s.runningResearch);
  const dismiss = useChatStore((s) => s.dismissResearchExit);
  if (!open) return null;
  const step = running
    ? t(`chat:processing.research.${running.stage}` as const, {
        read: running.emailsRead.toLocaleString(),
        total: running.emailsTotal.toLocaleString(),
        batch: running.batch,
        batches: running.batches,
      })
    : t('chat:processing.researching');
  return (
    <Modal
      open
      onClose={dismiss}
      size="sm"
      title={t('chat:research.exit.title')}
      footer={
        <div className="flex justify-end gap-2">
          <button
            type="button"
            data-testid="research-exit-keep"
            onClick={dismiss}
            className="rounded-md bg-primary-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-primary-700"
          >
            {t('chat:research.exit.keep')}
          </button>
          <button
            type="button"
            data-testid="research-exit-quit"
            onClick={() => {
              api.confirmExit().catch(() => dismiss());
            }}
            className="rounded-md border border-red-300 px-3 py-1.5 text-sm text-red-700 hover:bg-red-50"
          >
            {t('chat:research.exit.quit')}
          </button>
        </div>
      }
    >
      <p className="text-sm">{t('chat:research.exit.body')}</p>
      <p className="mt-2 text-sm text-gray-500">{step}</p>
    </Modal>
  );
}
