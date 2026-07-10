import { accountColorClass } from '@/lib/colors';

interface AccountScopeChipProps {
  /** Account the view is bound to — drives the deterministic color. */
  accountId: string;
  /** Account address shown in the chip. */
  email: string;
  /** Full explanation (e.g. "In All accounts view, this section shows X only") — tooltip + aria label. */
  hint: string;
}

/** Slim centered bar at the top of a single-account pane, shown in unified
 *  ("All accounts") mode: some views (chat, tasks, memory, contacts, drafts,
 *  attachments) are hard-scoped to ONE account, and this makes that scope
 *  visible at a glance. The color dot matches the account's indicator color
 *  in the unified inbox rows. */
export function AccountScopeChip({ accountId, email, hint }: AccountScopeChipProps) {
  return (
    <div className="flex items-center justify-center px-4 py-2 border-b border-gray-100 bg-gray-50/70">
      <span
        className="inline-flex items-center gap-2 max-w-full px-4 py-1 rounded-full border border-gray-200 bg-white text-sm text-gray-700 shadow-sm"
        title={hint}
      >
        <span className={`w-2.5 h-2.5 rounded-full flex-shrink-0 ${accountColorClass(accountId)}`} aria-hidden="true" />
        <span className="truncate">{email}</span>
      </span>
    </div>
  );
}
