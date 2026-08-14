import { useTranslation } from 'react-i18next';
import { accountColorClass } from '@/lib/colors';
import { useAccountStore } from '@/stores/accountStore';

interface ChatAccountPickerProps {
  /** Account the chat currently searches. */
  accountId: string | null;
  /** Called with the newly picked account id. */
  onChange: (accountId: string) => void;
  /** Compact styling for the panel header, where space is tight. */
  compact?: boolean;
}

/**
 * Which account this chat answers from — shown, and changeable.
 *
 * Chat is scoped to a single account: retrieval and the tools all take one
 * account id. In unified ("All accounts") mode the parent hands it the first
 * enabled account, which is rarely the one the user has in mind — a question
 * about mail that lives in another account came back "no matching emails",
 * indistinguishable from genuinely having none.
 *
 * A read-only chip stated the scope but left no way out of it. This makes the
 * scope selectable, so the answer's blind spot is both visible and fixable.
 * Changing the account clears the active conversation (see the account effect
 * in `ChatPanel` / `ChatView`), because a conversation belongs to the account
 * it was created under — carrying its history across would cite emails the new
 * account cannot see.
 */
export function ChatAccountPicker({ accountId, onChange, compact = false }: ChatAccountPickerProps) {
  const { t } = useTranslation(['chat', 'common']);
  const accounts = useAccountStore((s) => s.accounts);
  const enabled = accounts.filter((a) => a.enabled);

  // Nothing to choose between — a one-option dropdown is just noise.
  if (enabled.length <= 1) return null;

  const select = (
    <select
      value={accountId ?? ''}
      onChange={(e) => e.target.value && onChange(e.target.value)}
      title={t('chat:accountPicker.hint')}
      aria-label={t('chat:accountPicker.label')}
      className={
        compact
          ? 'min-w-0 max-w-[11rem] truncate rounded border-none bg-transparent px-1 py-0.5 text-xs text-gray-600 hover:bg-gray-100 focus:outline-none'
          : 'min-w-0 max-w-full truncate rounded-full border border-gray-200 bg-white py-1 pl-2 pr-6 text-sm text-gray-700 shadow-sm focus:outline-none'
      }
    >
      {enabled.map((a) => (
        <option key={a.id} value={a.id}>
          {a.email}
        </option>
      ))}
    </select>
  );

  // The colour dot matches the account's indicator in the unified inbox rows,
  // so the chat's scope is recognisable at the same glance as a mail row's.
  const dot = accountId ? (
    <span className={`h-2.5 w-2.5 flex-shrink-0 rounded-full ${accountColorClass(accountId)}`} aria-hidden="true" />
  ) : null;

  if (compact) {
    return (
      <span className="flex min-w-0 items-center gap-1">
        {dot}
        {select}
      </span>
    );
  }

  return (
    <div className="flex items-center justify-center gap-2 border-b border-gray-100 bg-gray-50/70 px-4 py-2">
      {dot}
      {select}
    </div>
  );
}
