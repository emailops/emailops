import { useTranslation } from 'react-i18next';
import { deleteForeverFolderOf, deleteForeverSupport } from '@/lib/deleteForever';
import { isUnifiedMode, useAccountStore } from '@/stores/accountStore';
import { useDeleteForeverStore } from '@/stores/deleteForeverStore';
import { useEmailStore } from '@/stores/emailStore';
import { DeleteForeverDialog } from './DeleteForeverDialog';

/**
 * Sits above the Trash and Spam lists of one account: "Empty Trash / Spam"
 * when the account can delete forever, or why it cannot (Gmail). Also hosts
 * the dialog of a "Delete forever" request, whether it came from here or
 * from the bulk toolbar.
 */
export function FolderCleanup() {
  const { t } = useTranslation(['inbox']);
  const listScope = useEmailStore((s) => s.listScope);
  const activeAccountId = useAccountStore((s) => s.activeAccountId);
  const accounts = useAccountStore((s) => s.accounts);
  const request = useDeleteForeverStore((s) => s.request);
  const open = useDeleteForeverStore((s) => s.open);
  const close = useDeleteForeverStore((s) => s.close);

  const folder = deleteForeverFolderOf(listScope);
  const account =
    activeAccountId && !isUnifiedMode(activeAccountId) ? accounts.find((a) => a.id === activeAccountId) : undefined;
  const support = folder && account ? deleteForeverSupport(account.provider) : 'unavailable';

  return (
    <>
      {folder && account && support === 'available' && (
        <div className="flex items-center gap-3 border-b border-gray-200 bg-white px-4 py-2 text-xs text-gray-600">
          <span className="min-w-0 flex-1">{t(`inbox:deleteForever.barText.${folder}`)}</span>
          <button
            type="button"
            data-testid="empty-folder"
            onClick={() => open({ accountId: account.id, folder, emailIds: null })}
            className="shrink-0 rounded px-2 py-1 font-medium text-red-600 transition-colors hover:bg-red-50"
          >
            {t(`inbox:deleteForever.empty.${folder}`)}
          </button>
        </div>
      )}
      {folder && support === 'gmail' && (
        <p
          data-testid="empty-folder-gmail-note"
          className="border-b border-gray-200 bg-white px-4 py-2 text-xs text-gray-600"
        >
          {t(`inbox:deleteForever.gmailNote.${folder}`)}
        </p>
      )}
      {request && <DeleteForeverDialog request={request} onClose={close} />}
    </>
  );
}
