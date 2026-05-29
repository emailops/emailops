import { useState } from 'react';
import * as api from '@/lib/api';
import { plainTextToHtml } from '@/lib/composeHtml';
import { errorText } from '@/lib/errors';
import { useEmailStore } from '@/stores/emailStore';
import { useLogStore } from '@/stores/logStore';

interface DraftRefPillProps {
  /** Draft id from the `draft://DRAFT_ID` markdown link. Caller validated
   *  it against the message's allowlist before rendering this pill. */
  draftId: string;
  accountId: string;
  /** Label the LLM wrapped in the link (subject, "the reply I just drafted",
   *  etc.). */
  label: string;
  /** Same hook `EmailRefPill` uses — `ChatView` wires this to
   *  `setViewMode('inbox')` so the user actually sees the result of the
   *  click. Without it, clicking the pill mutates the inbox store but the
   *  user stays on the chat view and the action looks like a no-op. */
  onOpenEmail?: () => void;
}

/**
 * Inline chip rendered where the LLM dropped `[label](draft://ID)`. Click
 * behaviour mirrors how the chat-tool-effect dispatcher already opens
 * AI-generated drafts:
 *
 *   - status='draft' + emailId set (reply) → seed `pendingChatDraft` and
 *     navigate to the inbound thread; `EmailView` opens the inline
 *     `ReplyCompose` with the saved body prepended on top of the quoted
 *     template (same flow as the AI Draft button).
 *   - status='draft' + emailId null (new mail) → open a standalone
 *     `Compose` tab with the draft contents.
 *   - status='sent' + emailId set → just navigate to the original thread
 *     (the sent message lives there).
 *   - status='sent' + emailId null → no thread to route to; surface a
 *     friendly notice via the output panel.
 *   - draft not found (deleted / wrong account) → warn-log and no-op.
 */
export function DraftRefPill({ draftId, accountId, label, onOpenEmail }: DraftRefPillProps) {
  const [isOpening, setIsOpening] = useState(false);
  const setPendingChatDraft = useEmailStore((s) => s.setPendingChatDraft);
  const navigateToEmail = useEmailStore((s) => s.navigateToEmail);
  const openComposeTab = useEmailStore((s) => s.openComposeTab);
  const addLog = useLogStore((s) => s.addLog);

  const handleClick = async () => {
    if (isOpening) return;
    setIsOpening(true);
    try {
      const drafts = await api.listDrafts(accountId);
      const draft = drafts.find((d) => d.id === draftId);
      if (!draft) {
        addLog('warn', 'chat', `Draft ${draftId} not found (deleted or wrong account).`);
        return;
      }
      if (draft.status === 'sent') {
        if (draft.emailId) {
          await navigateToEmail(accountId, draft.emailId);
          // Switch view AFTER the store has been mutated so the inbox
          // re-renders against the latest thread on its first paint.
          onOpenEmail?.();
        } else {
          addLog('info', 'chat', `Draft ${draftId} was sent (new-mail draft — no thread to navigate to).`);
        }
        return;
      }
      // status === 'draft'
      if (draft.emailId) {
        // Reply path — same mechanism as the chat-tool-effect dispatcher
        // uses when a freshly-generated reply arrives. Seed the pending
        // draft BEFORE navigating so `EmailView` consumes it on the same
        // render pass that mounts the thread.
        setPendingChatDraft({ emailId: draft.emailId, body: draft.body });
        await navigateToEmail(accountId, draft.emailId);
        onOpenEmail?.();
      } else {
        // New-mail path — pop the standalone Compose tab. The compose tab
        // only shows in inbox-family views, so the view-switch is what
        // makes the click visibly do something.
        openComposeTab(draft.accountId, draft.toAddresses, draft.subject, plainTextToHtml(draft.body));
        onOpenEmail?.();
      }
    } catch (e) {
      addLog('error', 'chat', `Failed to open draft ${draftId}: ${errorText(e)}`);
    } finally {
      setIsOpening(false);
    }
  };

  const displayLabel = label.trim() || 'Open draft';

  return (
    <button
      type="button"
      onClick={handleClick}
      disabled={isOpening}
      className="inline-flex items-center gap-1 px-1.5 py-0.5 mx-0.5 rounded bg-amber-50 border border-amber-200 text-amber-800 text-xs font-medium hover:bg-amber-100 transition-colors align-baseline disabled:opacity-60"
      title={`Re-open draft: ${displayLabel}`}
    >
      <svg className="w-3 h-3 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M11 5H6a2 2 0 00-2 2v11a2 2 0 002 2h11a2 2 0 002-2v-5m-1.414-9.414a2 2 0 112.828 2.828L11.828 15H9v-2.828l8.586-8.586z"
        />
      </svg>
      <span className="truncate max-w-[240px]">{displayLabel}</span>
    </button>
  );
}
