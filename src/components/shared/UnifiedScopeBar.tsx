import { useTranslation } from 'react-i18next';
import { isUnifiedMode, useAccountStore } from '@/stores/accountStore';
import { AccountScopeChip } from './AccountScopeChip';

interface UnifiedScopeBarProps {
  /** The concrete account the surrounding view fell back to (effective account). */
  accountId: string | null;
}

/** Self-gating scope indicator for single-account views: renders the
 *  [`AccountScopeChip`] only while the unified ("All accounts") entry is
 *  selected, and nothing at all otherwise — callers can mount it
 *  unconditionally above any per-account pane (tasks, memory, contacts,
 *  drafts, attachments). Chat renders its own chip with a chat-specific hint. */
export function UnifiedScopeBar({ accountId }: UnifiedScopeBarProps) {
  const { t } = useTranslation(['common']);
  const isUnified = useAccountStore((s) => isUnifiedMode(s.activeAccountId));
  const email = useAccountStore((s) => s.accounts.find((a) => a.id === accountId)?.email);
  if (!isUnified || !accountId || !email) return null;
  return <AccountScopeChip accountId={accountId} email={email} hint={t('common:unifiedScope.hint', { email })} />;
}
