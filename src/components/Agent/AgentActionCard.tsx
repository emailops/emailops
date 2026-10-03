import { useState } from 'react';
import { useTranslation } from 'react-i18next';
import { DraftRefPill } from '@/components/Chat/DraftRefPill';
import type { AgentAction, AgentRun } from '@/types';

interface AgentActionCardProps {
  action: AgentAction;
  /** The run it belongs to, when still in the feed — needed to open a draft. */
  run: AgentRun | undefined;
  busy: boolean;
  onApprove: (id: string) => void;
  onReject: (id: string) => void;
  onOpenEmail: () => void;
}

const STATUS_CLASS: Record<AgentAction['status'], string> = {
  pending: 'bg-amber-900/40 text-amber-300',
  done: 'bg-emerald-900/40 text-emerald-300',
  failed: 'bg-red-900/40 text-red-300',
  rejected: 'bg-gray-700 text-gray-400',
};

/** One action in the side panel: what, for which item, and — while pending — Approve / Reject. */
export function AgentActionCard({ action, run, busy, onApprove, onReject, onOpenEmail }: AgentActionCardProps) {
  const { t } = useTranslation(['agent']);
  const [showOutput, setShowOutput] = useState(false);
  const isSkillOutput = action.kind === 'runSkill' && action.status === 'done' && action.result;

  return (
    <li
      data-testid={`agent-action-${action.id}`}
      className="rounded-lg border border-gray-700 bg-gray-800/60 px-3 py-2 text-sm"
    >
      <div className="flex items-center gap-2">
        <span className="font-medium text-gray-100">{t(`agent:actions.kind.${action.kind}`)}</span>
        <span className={`ml-auto rounded px-1.5 py-0.5 text-[10px] uppercase ${STATUS_CLASS[action.status]}`}>
          {t(`agent:actions.status.${action.status}`)}
        </span>
      </div>
      <p className="mt-0.5 truncate text-xs text-gray-400" title={action.runTitle}>
        {action.runTitle}
      </p>
      {action.detail && action.kind !== 'runSkill' && (
        <p className="mt-1 text-xs text-gray-300 line-clamp-3">{action.detail}</p>
      )}
      {action.kind === 'runSkill' && <p className="mt-1 text-xs text-gray-300">/{action.detail}</p>}
      <p className="mt-1 text-[11px] text-gray-500">{t('agent:actions.rule', { name: action.ruleName })}</p>
      {action.error && <p className="mt-1 text-xs text-red-300 break-words">{action.error}</p>}
      {isSkillOutput && (
        <div className="mt-1">
          <button
            type="button"
            onClick={() => setShowOutput((v) => !v)}
            className="text-xs text-primary-400 hover:underline"
          >
            {t('agent:actions.output')}
          </button>
          {showOutput && (
            <p className="mt-1 whitespace-pre-wrap rounded bg-gray-900/60 p-2 text-xs text-gray-200">{action.result}</p>
          )}
        </div>
      )}
      {action.kind === 'draftReply' && action.status === 'done' && action.result && run && (
        <div className="mt-1">
          <DraftRefPill
            draftId={action.result}
            accountId={run.accountId}
            label={t('agent:actions.openDraft')}
            onOpenEmail={onOpenEmail}
          />
        </div>
      )}
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
    </li>
  );
}
