import { useEffect, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { DraftRefPill } from '@/components/Chat/DraftRefPill';
import { InlineError } from '@/components/common/InlineError';
import { EmailPreviewById } from '@/components/shared/EmailPreviewById';
import { deleteDraft, getAccountSignature, getDraft, reviewAgentDraft, saveDraft, sendReply } from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';
import { useOutboxStore } from '@/stores/outboxStore';
import type { AgentAction, AgentRun, Draft } from '@/types';
import { buildReplyMessage } from './agentFeed';

interface AgentReviewPaneProps {
  action: AgentAction;
  /** The run the action belongs to; undefined when it left the feed. */
  run: AgentRun | undefined;
  busy: boolean;
  onBack: () => void;
  onApprove: (id: string) => void;
  onReject: (id: string) => void;
  /** The backend state changed: reload the view. */
  onChanged: () => void;
  onOpenEmail: () => void;
}

/**
 * Where the user checks one piece of the agent's work without leaving the
 * Agent view: the email it acted on, what it did or proposes, and the
 * controls to finish it — send or discard a reply draft, approve or reject a
 * mailbox action.
 */
export function AgentReviewPane({
  action,
  run,
  busy,
  onBack,
  onApprove,
  onReject,
  onChanged,
  onOpenEmail,
}: AgentReviewPaneProps) {
  const { t } = useTranslation(['agent']);
  const addLog = useLogStore((s) => s.addLog);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [text, setText] = useState('');
  const [working, setWorking] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const draftId = action.kind === 'draftReply' && action.status === 'done' ? action.result : null;
  const accountId = run?.accountId ?? '';

  useEffect(() => {
    setDraft(null);
    setNotice(null);
    setError(null);
    if (!draftId || !accountId) return;
    let stale = false;
    getDraft(accountId, draftId)
      .then((d) => {
        if (stale) return;
        setDraft(d);
        setText(d?.body ?? '');
      })
      .catch((err) => {
        if (!stale) setError(`${t('agent:review.error')}: ${errorText(err)}`);
      });
    return () => {
      stale = true;
    };
  }, [draftId, accountId, t]);

  const finish = async (what: string, op: () => Promise<void>) => {
    setWorking(true);
    setError(null);
    try {
      await op();
      onChanged();
    } catch (err) {
      const msg = `${t('agent:review.error')}: ${errorText(err)}`;
      setError(msg);
      addLog('error', 'ai', `${what}: ${errorText(err)}`);
    } finally {
      setWorking(false);
    }
  };

  const save = () =>
    finish('Saving the agent draft', async () => {
      if (!draft) return;
      const saved = await saveDraft({
        id: draft.id,
        emailId: draft.emailId,
        accountId: draft.accountId,
        toAddresses: draft.toAddresses,
        ccAddresses: draft.ccAddresses,
        subject: draft.subject,
        body: text,
        providerDraftId: draft.providerDraftId,
      });
      setDraft(saved);
      setNotice(t('agent:review.saved'));
    });

  const send = () =>
    finish('Sending the agent draft', async () => {
      if (!draft || !run) return;
      const emailId = draft.emailId ?? run.triggerRef;
      const signature = await getAccountSignature(run.accountId);
      const message = buildReplyMessage({ accountId: run.accountId, emailId, draft, text, signature });
      await useOutboxStore.getState().send(message, {
        draftId: draft.id,
        sendDirect: async () => {
          await sendReply(
            emailId,
            message.body,
            run.accountId,
            message.to,
            message.cc,
            message.bodyHtml ?? undefined,
            message.inlineImages,
            [],
          );
          await deleteDraft(draft.id, run.accountId);
        },
      });
      await reviewAgentDraft(action.id, 'sent');
      addLog('success', 'ai', `Agent draft sent to ${message.to.join(', ')}`);
      setNotice(t('agent:review.sentNotice'));
    });

  const discard = () =>
    finish('Discarding the agent draft', async () => {
      if (!draft) return;
      await deleteDraft(draft.id, draft.accountId);
      await reviewAgentDraft(action.id, 'discarded');
      setNotice(t('agent:review.discardedNotice'));
    });

  const outcome = action.reviewOutcome;
  const canEditDraft = action.needsReview && draft !== null && !notice;

  return (
    <div data-testid="agent-review" className="mx-auto max-w-3xl space-y-4">
      <button type="button" onClick={onBack} className="text-sm text-primary-400 hover:underline">
        ← {t('agent:review.back')}
      </button>
      <div>
        <h3 className="text-lg font-semibold text-gray-100">{t(`agent:actions.kind.${action.kind}`)}</h3>
        <p className="text-xs text-gray-400">
          {t('agent:actions.rule', { name: action.ruleName })} · {action.runTitle}
        </p>
      </div>
      {run?.summary && <p className="whitespace-pre-wrap text-sm text-gray-200">{run.summary}</p>}
      <InlineError message={error} />

      {action.status === 'pending' && (
        <div className="space-y-2 rounded-lg border border-amber-800 bg-amber-900/20 p-3">
          <p className="text-sm text-amber-200">{t('agent:review.approveHint')}</p>
          <div className="flex gap-2">
            <button
              type="button"
              data-testid="agent-review-approve"
              disabled={busy}
              onClick={() => onApprove(action.id)}
              className="rounded bg-primary-600 px-3 py-1.5 text-sm text-white hover:bg-primary-500 disabled:opacity-50"
            >
              {t('agent:actions.approve')}
            </button>
            <button
              type="button"
              data-testid="agent-review-reject"
              disabled={busy}
              onClick={() => onReject(action.id)}
              className="rounded bg-gray-700 px-3 py-1.5 text-sm text-gray-200 hover:bg-gray-600 disabled:opacity-50"
            >
              {t('agent:actions.reject')}
            </button>
          </div>
        </div>
      )}

      {action.kind === 'draftReply' && action.status === 'done' && (
        <section className="space-y-2">
          <h4 className="text-xs font-semibold uppercase text-gray-400">{t('agent:review.draft')}</h4>
          {notice && <p className="text-sm text-emerald-300">{notice}</p>}
          {!notice && outcome && <p className="text-sm text-gray-300">{t(`agent:review.outcome.${outcome}`)}</p>}
          {!notice && !outcome && !action.needsReview && (
            <p className="text-sm text-gray-400">{t('agent:review.goneNotice')}</p>
          )}
          {canEditDraft && (
            <>
              <textarea
                data-testid="agent-review-draft"
                value={text}
                rows={10}
                onChange={(e) => setText(e.target.value)}
                className="w-full rounded border border-gray-600 bg-gray-800 px-3 py-2 text-sm text-gray-100"
              />
              <div className="flex flex-wrap items-center gap-2">
                <button
                  type="button"
                  data-testid="agent-review-send"
                  disabled={working || !text.trim()}
                  onClick={() => void send()}
                  className="rounded bg-primary-600 px-3 py-1.5 text-sm text-white hover:bg-primary-500 disabled:opacity-50"
                >
                  {t('agent:review.send')}
                </button>
                <button
                  type="button"
                  data-testid="agent-review-save"
                  disabled={working || text === draft.body}
                  onClick={() => void save()}
                  className="rounded bg-gray-700 px-3 py-1.5 text-sm text-gray-200 hover:bg-gray-600 disabled:opacity-50"
                >
                  {t('agent:review.save')}
                </button>
                <button
                  type="button"
                  data-testid="agent-review-discard"
                  disabled={working}
                  onClick={() => void discard()}
                  className="rounded px-3 py-1.5 text-sm text-red-300 hover:bg-red-900/30 disabled:opacity-50"
                >
                  {t('agent:review.discard')}
                </button>
                <span className="flex-1" />
                {run && (
                  <DraftRefPill
                    draftId={draft.id}
                    accountId={run.accountId}
                    label={t('agent:review.openInComposer')}
                    onOpenEmail={onOpenEmail}
                  />
                )}
              </div>
            </>
          )}
        </section>
      )}

      {action.kind === 'createTask' && action.status === 'done' && (
        <section className="space-y-1">
          <h4 className="text-xs font-semibold uppercase text-gray-400">{t('agent:review.task')}</h4>
          <p className="text-sm text-gray-200">{action.detail}</p>
        </section>
      )}

      {action.kind === 'runSkill' && action.result && (
        <section className="space-y-1">
          <h4 className="text-xs font-semibold uppercase text-gray-400">{t('agent:review.skill')}</h4>
          <p className="whitespace-pre-wrap rounded bg-gray-900/60 p-3 text-sm text-gray-200">{action.result}</p>
        </section>
      )}

      {action.error && <p className="text-sm text-red-300 break-words">{action.error}</p>}

      {run && run.trigger === 'email' && (
        <section className="space-y-1">
          <h4 className="text-xs font-semibold uppercase text-gray-400">{t('agent:review.original')}</h4>
          <div className="overflow-hidden rounded-lg border border-gray-700 bg-white">
            <EmailPreviewById accountId={run.accountId} emailId={run.triggerRef} emptyMessage="" />
          </div>
        </section>
      )}
      {run && run.trigger === 'event' && (
        <section className="space-y-1">
          <h4 className="text-xs font-semibold uppercase text-gray-400">{t('agent:review.event')}</h4>
          <p className="text-sm text-gray-200">{run.title}</p>
        </section>
      )}
    </div>
  );
}
