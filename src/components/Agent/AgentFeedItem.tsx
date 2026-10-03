import { useTranslation } from 'react-i18next';
import { EmailRefPill } from '@/components/Chat/EmailRefPill';
import { formatDateTime } from '@/lib/intl';
import type { AgentRun } from '@/types';

interface AgentFeedItemProps {
  run: AgentRun;
  onOpenEmail: () => void;
}

const STATUS_DOT: Record<string, string> = {
  pending: 'bg-amber-400',
  done: 'bg-emerald-400',
  failed: 'bg-red-400',
  rejected: 'bg-gray-500',
};

/** One agent message: the email or event it looked at, its analysis, and what it did. */
export function AgentFeedItem({ run, onOpenEmail }: AgentFeedItemProps) {
  const { t, i18n } = useTranslation(['agent']);
  const isEmail = run.trigger === 'email';

  return (
    <li data-testid={`agent-run-${run.id}`} className="flex gap-3">
      <div className="mt-1 flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-full bg-primary-700/60 text-primary-200">
        <svg className="h-4 w-4" fill="none" stroke="currentColor" viewBox="0 0 24 24" aria-hidden="true">
          {isEmail ? (
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z"
            />
          ) : (
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M8 7V3m8 4V3m-9 8h10M5 21h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v12a2 2 0 002 2z"
            />
          )}
        </svg>
      </div>
      <div className="min-w-0 flex-1 rounded-2xl rounded-tl-sm bg-gray-800 px-4 py-3">
        <div className="flex flex-wrap items-baseline gap-x-2 text-xs text-gray-400">
          <span className="font-semibold text-gray-300">{isEmail ? t('agent:feed.email') : t('agent:feed.event')}</span>
          {run.sender && <span>{t('agent:feed.from', { sender: run.sender })}</span>}
          <span className="ml-auto">{formatDateTime(run.createdAt, i18n.language)}</span>
        </div>
        <div className="mt-1 text-sm text-gray-100">
          {isEmail ? (
            <EmailRefPill
              emailId={run.triggerRef}
              accountId={run.accountId}
              label={run.title}
              onOpenEmail={onOpenEmail}
            />
          ) : (
            <span className="font-medium">{run.title}</span>
          )}
        </div>
        {run.summary && <p className="mt-2 whitespace-pre-wrap text-sm text-gray-200">{run.summary}</p>}
        {run.status === 'failed' && <p className="mt-2 text-xs text-red-300">{run.error ?? t('agent:feed.failed')}</p>}
        {run.actions.length > 0 && (
          <ul className="mt-2 flex flex-wrap gap-1.5">
            {run.actions.map((a) => (
              <li
                key={a.id}
                className="flex items-center gap-1.5 rounded-full border border-gray-600 px-2 py-0.5 text-xs text-gray-300"
                title={t(`agent:actions.status.${a.status}`)}
              >
                <span className={`h-1.5 w-1.5 rounded-full ${STATUS_DOT[a.status]}`} />
                {t(`agent:actions.kind.${a.kind}`)}
              </li>
            ))}
          </ul>
        )}
      </div>
    </li>
  );
}
