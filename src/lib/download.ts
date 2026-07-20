import { i18n } from '@/i18n';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';
import { useToastStore } from '@/stores/toastStore';

/** Strip the `data:<mime>;base64,` prefix, leaving the raw base64 payload. */
export function dataUrlToBase64(dataUrl: string): string {
  const comma = dataUrl.indexOf(',');
  return comma === -1 ? dataUrl : dataUrl.slice(comma + 1);
}

/**
 * Save attachment bytes to the user's Downloads folder via the backend, then
 * toast a completion notification with a "Show in Finder" action. Failures
 * are logged to the output panel; the returned promise never rejects, so
 * callers only manage their own busy state.
 */
export async function saveToDownloads(filename: string, base64Data: string): Promise<void> {
  try {
    const path = await api.saveAttachmentToDownloads(filename, base64Data);
    notifyDownloadComplete(path, filename);
  } catch (err) {
    useLogStore.getState().addLog('error', 'attachments', `Failed to download ${filename}: ${errorText(err)}`);
  }
}

/** Toast a "saved" notification whose action reveals the file in the OS file manager. */
export function notifyDownloadComplete(path: string, filename: string): void {
  useToastStore.getState().addToast({
    message: i18n.t('attachments:download.savedTo', { filename }),
    actionLabel: i18n.t('attachments:download.showInFinder'),
    onAction: () => {
      void api.revealInFinder(path).catch((err) => {
        useLogStore.getState().addLog('error', 'attachments', `Failed to reveal ${path}: ${errorText(err)}`);
      });
    },
  });
}
