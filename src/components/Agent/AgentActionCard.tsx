import { useTranslation } from 'react-i18next';
import type { AgentAction } from '@/types';

interface AgentActionCardProps {
  action: AgentAction;
  selected: boolean;
  busy: boolean;
  onSelect: (id: string) => void;
  onApprove: (id: string) => void;
  onReject: (id: string) => void;
}

const BADGE_CLASS: Record<string, string> = {
  pending: 'bg-amber-900/40 text-amber-300',
  review: 'bg-amber-900/40 text-amber-300',
  done: 'bg-emerald-900/40 text-emerald-300',
  sent: 'bg-emerald-900/40 text-emerald-300',
  failed: 'bg-red-900/40 text-red-300',
  rejected: 'bg-gray-700 text-gray-400',
  discarded: 'bg-gray-700 text-gray-400',
};

/** One action in the side panel. A click opens it in the review pane; a
 *  pending one can also be approved or rejected right here. */
export function AgentActionCard({ action, selected, busy, onSelect, onApprove, onReject }: AgentActionCardProps) {
  const { t } = useTranslation(['agent']);
  // What the badge says: a draft waiting for review, what the user did with a
  // reviewed draft, or the action's status.
  const badgeKey = action.needsReview ? 'review' : (action.reviewOutcome ?? action.status);
  const badgeLabel = action.needsReview
    ? t('agent:review.review')
    : action.reviewOutcome
      ? t(`agent:review.outcome.${action.reviewOutcome}`)
      : t(`agent:actions.status.${action.status}`);

  return (
    <li
      data-testid={`agent-action-${action.id}`}
      className={`rounded-lg border px-3 py-2 text-sm ${
        selected ? 'border-primary-500 bg-gray-700/70' : 'border-gray-700 bg-gray-800/60'
      }`}
    >
      <button
        type="button"
        data-testid={`agent-open-${action.id}`}
        onClick={() => onSelect(action.id)}
        className="block w-full text-left"
      >
        <span className="flex items-center gap-2">
          <span className="font-medium text-gray-100">{t(`agent:actions.kind.${action.kind}`)}</span>
          <span className={`ml-auto rounded px-1.5 py-0.5 text-[10px] uppercase ${BADGE_CLASS[badgeKey]}`}>
            {badgeLabel}
          </span>
        </span>
        <span className="mt-0.5 block truncate text-xs text-gray-400" title={action.runTitle}>
          {action.runTitle}
        </span>
        {action.detail && (
          <span className="mt-1 block text-xs text-gray-300 line-clamp-2">
            {action.kind === 'runSkill' ? `/${action.detail}` : action.detail}
          </span>
        )}
        {action.error && <span className="mt-1 block text-xs text-red-300 line-clamp-2">{action.error}</span>}
      </button>
      {action.status === 'pending' && (
        <div className="mt-2 flex gap-2">
          <button
            type="button"
            data-testid={`agent-approve-${action.id}`}
            disabled={busy}
            onClick={() => onApprove(action.id)}
            className="rounded bg-primary-600 px-2.5 py-1 text-xs text-white hover:bg-primary-500 disabled:opacity-50"
          >
            {t('agent:actions.approve')}
          </button>
          <button
            type="button"
            data-testid={`agent-reject-${action.id}`}
            disabled={busy}
            onClick={() => onReject(action.id)}
            className="rounded bg-gray-700 px-2.5 py-1 text-xs text-gray-200 hover:bg-gray-600 disabled:opacity-50"
          >
            {t('agent:actions.reject')}
          </button>
        </div>
      )}
      {action.needsReview && (
        <button
          type="button"
          data-testid={`agent-review-${action.id}`}
          onClick={() => onSelect(action.id)}
          className="mt-2 rounded bg-primary-600 px-2.5 py-1 text-xs text-white hover:bg-primary-500"
        >
          {t('agent:review.review')}
        </button>
      )}
    </li>
  );
}
