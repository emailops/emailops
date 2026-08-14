import { useTranslation } from 'react-i18next';
import type { ChatContext } from '@/lib/chatContext';
import { accountColorClass } from '@/lib/colors';
import { useAccountStore } from '@/stores/accountStore';

interface OtherAccountContextNoticeProps {
  /** The open thread — belonging to an account chat is NOT scoped to. */
  context: ChatContext;
  /** Retarget chat at the thread's account. */
  onSwitchAccount: (accountId: string) => void;
}

/**
 * Shown in place of the context chip when the email on screen belongs to a
 * different account than chat answers from — only reachable in "All accounts",
 * where the list shows every account while chat is scoped to one.
 *
 * The alternative was to show nothing, which is how this went wrong in the
 * first place: asking about the email plainly on screen produced an answer
 * from a different mailbox ("no matching emails found") with nothing saying
 * why. Naming the owning account and offering the switch turns a silent wrong
 * answer into a one-click fix.
 */
export function OtherAccountContextNotice({ context, onSwitchAccount }: OtherAccountContextNoticeProps) {
  const { t } = useTranslation('chat');
  const ownerEmail = useAccountStore((s) => s.accounts.find((a) => a.id === context.accountId)?.email);

  // The account was removed or disabled under us — switching would go nowhere,
  // and a chip naming an account that no longer exists is worse than silence.
  if (!ownerEmail) return null;

  return (
    <div
      className="mb-2 flex items-center gap-1.5 rounded-md border border-amber-200 bg-amber-50 px-2 py-1 text-xs text-amber-900"
      title={t('panel.context.otherAccount.hint')}
    >
      <span
        className={`h-2.5 w-2.5 flex-shrink-0 rounded-full ${accountColorClass(context.accountId)}`}
        aria-hidden="true"
      />
      <span className="truncate">{t('panel.context.otherAccount.label', { email: ownerEmail })}</span>
      <button
        type="button"
        onClick={() => onSwitchAccount(context.accountId)}
        className="ml-auto flex-shrink-0 rounded border border-amber-300 px-1.5 py-0.5 font-medium hover:bg-amber-100"
      >
        {t('panel.context.otherAccount.action')}
      </button>
    </div>
  );
}
