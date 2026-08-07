import { useState } from 'react';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useEmailStore } from '@/stores/emailStore';
import { useLogStore } from '@/stores/logStore';
import type { ChatMessageSource } from '@/types';

interface CitationPillProps {
  source: ChatMessageSource;
  accountId: string;
  onOpenEmail?: () => void;
}

/**
 * A small numbered pill rendered inline inside assistant replies. Clicking it
 * loads the referenced email by id and opens it in a new inbox tab.
 */
export function CitationPill({ source, accountId, onOpenEmail }: CitationPillProps) {
  const [isOpening, setIsOpening] = useState(false);
  const openTab = useEmailStore((s) => s.openTab);
  const addLog = useLogStore((s) => s.addLog);

  const handleClick = async () => {
    if (isOpening) return;
    setIsOpening(true);
    try {
      const email = await api.getEmailById(accountId, source.emailId);
      await openTab(email);
      onOpenEmail?.();
    } catch (e) {
      addLog('error', 'system', `Failed to open cited email: ${errorText(e)}`);
    } finally {
      setIsOpening(false);
    }
  };

  return (
    <button
      type="button"
      onClick={handleClick}
      disabled={isOpening}
      className="inline-flex items-center justify-center min-w-[1.5rem] h-5 px-1.5 mx-0.5 text-[11px] font-semibold text-primary-700 bg-primary-100 hover:bg-primary-200 rounded-full transition-colors align-middle disabled:opacity-60 dark:text-primary-300 dark:bg-primary-900/30"
      title={`Open source email ${source.citationNumber}`}
    >
      {source.citationNumber}
    </button>
  );
}
