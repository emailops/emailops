import { useState } from 'react';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useEmailStore } from '@/stores/emailStore';
import { useLogStore } from '@/stores/logStore';

interface EmailRefPillProps {
  /** Email id from the `email://EMAIL_ID` markdown link the LLM emitted.
   *  Caller already validated it against the message's allowlist before
   *  rendering this pill — drop-and-warn happens in `MarkdownContent`. */
  emailId: string;
  accountId: string;
  /** Label text the LLM wrapped in the link (subject, sender, "the kickoff
   *  email"…). Falls back to a generic glyph-only chip if empty. */
  label: string;
  onOpenEmail?: () => void;
}

/**
 * Inline chip rendered where the LLM dropped an `[label](email://ID)` link.
 * Mirrors `CitationPill`'s open-the-email behaviour but is visually
 * distinguishable: a tiny envelope glyph + the natural-language label
 * instead of a numbered pill, since these references come from tool
 * results rather than from the numbered `[n]` Sources block.
 */
export function EmailRefPill({ emailId, accountId, label, onOpenEmail }: EmailRefPillProps) {
  const [isOpening, setIsOpening] = useState(false);
  const openTab = useEmailStore((s) => s.openTab);
  const addLog = useLogStore((s) => s.addLog);

  const handleClick = async () => {
    if (isOpening) return;
    setIsOpening(true);
    try {
      const email = await api.getEmailById(accountId, emailId);
      await openTab(email);
      onOpenEmail?.();
    } catch (e) {
      // Validator only checks the id was tool-produced. Fetch can still
      // fail (email deleted between the chat turn and this click, sync
      // gap, …) — surface it via the output panel like CitationPill.
      addLog('error', 'chat', `Failed to open email ${emailId}: ${errorText(e)}`);
    } finally {
      setIsOpening(false);
    }
  };

  const displayLabel = label.trim() || 'Open email';

  return (
    <button
      type="button"
      onClick={handleClick}
      disabled={isOpening}
      className="inline-flex items-center gap-1 px-1.5 py-0.5 mx-0.5 rounded bg-primary-50 border border-primary-200 text-primary-700 text-xs font-medium hover:bg-primary-100 transition-colors align-baseline disabled:opacity-60 dark:bg-primary-900/20 dark:border-primary-800 dark:text-primary-300 dark:hover:bg-primary-900/30"
      title={`Open email: ${displayLabel}`}
    >
      <svg className="w-3 h-3 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M3 8l7.89 5.26a2 2 0 002.22 0L21 8M5 19h14a2 2 0 002-2V7a2 2 0 00-2-2H5a2 2 0 00-2 2v10a2 2 0 002 2z"
        />
      </svg>
      <span className="truncate max-w-[240px]">{displayLabel}</span>
    </button>
  );
}
