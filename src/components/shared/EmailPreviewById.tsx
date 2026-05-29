// Reusable side-panel that renders a single email given its ID. Used by the
// Tasks panel (originating email for a PendingTask), the Memory view (origin
// email for a MemoryFact), and the waiting-on-reply list.
//
// Callers supply the email id + fallback messages for the two "not loaded"
// states (no selection yet, selection has no source email). Loading and
// error states are handled internally so every panel looks consistent.

import { format } from 'date-fns';
import { useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import { sanitizeEmailHtml } from '@/lib/emailFormatting';
import { errorText } from '@/lib/errors';
import type { Email } from '@/types';
import { EmailHtmlFrame } from './EmailHtmlFrame';

export interface EmailPreviewByIdProps {
  /** Account that owns the email — backend rejects cross-account lookups. */
  accountId: string;
  /** Email ID to render. `null` shows `emptyMessage`. */
  emailId: string | null;
  /** Shown when `emailId` is null and no row is selected yet. */
  emptyMessage: string;
  /** Shown when a row is selected but it has no originating email. */
  missingSourceMessage?: string;
  /**
   * When true, the caller has a selection but its source email id is null
   * (e.g. a manually-created task). Renders `missingSourceMessage` instead
   * of the generic `emptyMessage`.
   */
  hasSelection?: boolean;
}

export function EmailPreviewById({
  accountId,
  emailId,
  emptyMessage,
  missingSourceMessage,
  hasSelection = false,
}: EmailPreviewByIdProps) {
  const { t } = useTranslation(['inbox']);
  const [email, setEmail] = useState<Email | null>(null);
  const [body, setBody] = useState<string>('');
  const [isLoading, setIsLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);

  const safeHtml = useMemo(() => (email ? sanitizeEmailHtml(body || email.snippet || '') : ''), [body, email]);

  useEffect(() => {
    if (!emailId) {
      setEmail(null);
      setBody('');
      setLoadError(null);
      return;
    }
    let cancelled = false;
    setIsLoading(true);
    setLoadError(null);
    (async () => {
      try {
        const [fetchedEmail, fetchedBody] = await Promise.all([
          api.getEmailById(accountId, emailId),
          api.getEmailBody(accountId, emailId),
        ]);
        if (cancelled) return;
        setEmail(fetchedEmail);
        setBody(fetchedBody ?? '');
      } catch (e) {
        if (cancelled) return;
        setLoadError(errorText(e));
        setEmail(null);
        setBody('');
      } finally {
        if (!cancelled) setIsLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [accountId, emailId]);

  if (!emailId) {
    const msg = hasSelection && missingSourceMessage ? missingSourceMessage : emptyMessage;
    return (
      <div className="flex flex-col h-full items-center justify-center text-sm text-gray-500 bg-gray-50 px-8 text-center">
        {msg}
      </div>
    );
  }

  if (isLoading) {
    return (
      <div className="flex flex-col h-full items-center justify-center text-sm text-gray-500 bg-gray-50">
        {t('inbox:loadingEmail')}
      </div>
    );
  }

  if (loadError || !email) {
    return (
      <div className="flex flex-col h-full items-center justify-center text-sm text-gray-500 bg-gray-50 px-8 text-center">
        {loadError ?? 'Email no longer available.'}
      </div>
    );
  }

  return (
    <div className="flex flex-col h-full overflow-hidden">
      <div className="px-6 py-4 border-b border-gray-200 flex-shrink-0">
        <h2 className="text-base font-semibold text-gray-900 truncate">{email.subject || '(no subject)'}</h2>
        <div className="text-xs text-gray-500 mt-1">
          <span className="font-medium text-gray-700">{email.sender}</span>
          <span> &lt;{email.senderEmail}&gt;</span>
          <span> · {format(new Date(email.timestamp * 1000), 'MMM d, yyyy · h:mm a')}</span>
        </div>
      </div>
      <div className="flex-1 overflow-y-auto px-6 py-4 email-body-content">
        <EmailHtmlFrame html={safeHtml} />
      </div>
    </div>
  );
}
