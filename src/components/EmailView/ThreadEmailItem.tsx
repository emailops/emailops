import { format } from 'date-fns';
import { useEffect, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import * as api from '@/lib/api';
import type { Email, EmailAttachmentMeta } from '@/types';
import { EmailAttachments } from './EmailAttachments';
import { EmailBody } from './EmailBody';

export interface ThreadEmailItemProps {
  email: Email;
  isExpanded: boolean;
  isLast: boolean;
  isFocused?: boolean;
  isSearchMatch?: boolean;
  highlightQuery: string | null;
  onToggle: () => void;
  onOpenAttachment: (meta: EmailAttachmentMeta) => void;
}

export function ThreadEmailItem({
  email,
  isExpanded,
  isLast,
  isFocused,
  isSearchMatch,
  highlightQuery,
  onToggle,
  onOpenAttachment,
}: ThreadEmailItemProps) {
  const { t } = useTranslation(['common']);
  const itemRef = useRef<HTMLDivElement>(null);
  // Body is fetched lazily on expand. The store pre-loads the body for the selected
  // email so it is available immediately; collapsed older emails fetch on demand.
  const [body, setBody] = useState<string | null>(email.body || null);

  useEffect(() => {
    if (!isExpanded || body !== null) return;
    api
      .getEmailBody(email.accountId, email.id)
      .then(setBody)
      .catch(() => setBody(''));
  }, [isExpanded, email.accountId, email.id, body]);

  useEffect(() => {
    if ((isFocused || isSearchMatch) && itemRef.current) {
      itemRef.current.scrollIntoView({ behavior: 'smooth', block: 'start' });
    }
  }, [isFocused, isSearchMatch]);

  const formattedDate = format(new Date(email.timestamp * 1000), 'PPpp');
  const shortDate = format(new Date(email.timestamp * 1000), 'MMM d');

  if (!isExpanded) {
    return (
      <div
        ref={itemRef}
        className="border-b border-gray-100 hover:bg-gray-50 cursor-pointer transition-colors"
        onClick={onToggle}
      >
        <div className="px-6 py-3 flex items-center gap-3">
          <div className="flex-shrink-0 w-8 h-8 bg-gray-200 rounded-full flex items-center justify-center">
            <span className="text-sm font-medium text-gray-600">{email.sender.charAt(0).toUpperCase()}</span>
          </div>
          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2">
              <span className="font-medium text-sm text-gray-900 truncate">{email.sender}</span>
              <span className="text-xs text-gray-400">{shortDate}</span>
            </div>
            <p className="text-sm text-gray-500 truncate">{email.snippet || '(No preview)'}</p>
          </div>
          <svg className="w-5 h-5 text-gray-400 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
          </svg>
        </div>
      </div>
    );
  }

  return (
    <div ref={itemRef} className={`border-b border-gray-200 ${isLast ? '' : 'bg-gray-50/50'}`}>
      <div
        className={`px-6 py-4 ${!isLast ? 'cursor-pointer hover:bg-gray-100/50' : ''}`}
        onClick={!isLast ? onToggle : undefined}
      >
        <div className="flex items-start justify-between">
          <div className="flex items-start gap-3">
            <div className="flex-shrink-0 w-10 h-10 bg-primary-100 rounded-full flex items-center justify-center">
              <span className="text-sm font-medium text-primary-700">{email.sender.charAt(0).toUpperCase()}</span>
            </div>
            <div>
              <div className="flex items-center gap-2">
                <span className="font-medium text-gray-900">{email.sender}</span>
                <span className="text-gray-400 text-sm">&lt;{email.senderEmail}&gt;</span>
              </div>
              <div className="text-xs text-gray-500 mt-0.5">{formattedDate}</div>
              {email.recipients.length > 0 && (
                <div className="text-xs text-gray-400 mt-1">To: {email.recipients.join(', ')}</div>
              )}
              {email.cc.length > 0 && <div className="text-xs text-gray-400 mt-0.5">Cc: {email.cc.join(', ')}</div>}
            </div>
          </div>
          {!isLast && (
            <svg
              className="w-5 h-5 text-gray-400 flex-shrink-0 mt-1"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
            >
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M5 15l7-7 7 7" />
            </svg>
          )}
        </div>
      </div>

      <div className="px-6 pb-6">
        {body === null ? (
          <div className="flex items-center gap-2 py-4 text-sm text-gray-400">
            <div className="animate-spin rounded-full h-4 w-4 border-b-2 border-gray-300" />
            {t('common:state.loading')}
          </div>
        ) : (
          <EmailBody
            html={body}
            highlightQuery={highlightQuery}
            scrollToFirstMatch={isSearchMatch ?? false}
            accountId={email.accountId}
            senderEmail={email.senderEmail}
          />
        )}
        <EmailAttachments emailId={email.id} accountId={email.accountId} onOpenAttachment={onOpenAttachment} />
      </div>
    </div>
  );
}
