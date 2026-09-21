import { useCallback, useMemo } from 'react';
import { useTranslation } from 'react-i18next';
import { EmailHtmlFrame } from '@/components/shared/EmailHtmlFrame';
import { useRemoteContentPolicy } from '@/hooks/useRemoteContentPolicy';
import * as api from '@/lib/api';
import { plainTextToHtml } from '@/lib/composeHtml';
import { type ParsedMailto, sanitizeEmailHtmlFull } from '@/lib/emailFormatting';
import { errorText } from '@/lib/errors';
import { useEmailStore } from '@/stores/emailStore';
import { useLogStore } from '@/stores/logStore';

export function EmailBody({
  html,
  highlightQuery,
  activeMatchIndex,
  onMatchesReported,
  accountId,
  senderEmail,
}: {
  html: string;
  highlightQuery?: string | null;
  activeMatchIndex?: number | null;
  onMatchesReported?: (count: number) => void;
  accountId: string;
  senderEmail: string;
}) {
  const { t } = useTranslation(['inbox']);
  const addLog = useLogStore((s) => s.addLog);
  const openComposeTab = useEmailStore((s) => s.openComposeTab);

  // mailto: links in the body open a compose tab pre-filled from the link,
  // sending from the account that received this email.
  const handleMailtoLink = useCallback(
    (mailto: ParsedMailto) => {
      openComposeTab(accountId, mailto.to, mailto.subject, mailto.body ? plainTextToHtml(mailto.body) : '');
    },
    [accountId, openComposeTab],
  );
  // The policy (preference + trusted-sender allowlist + the render gate that
  // stops images flashing in before both resolve) is shared with
  // `EmailPreviewById` — see `useRemoteContentPolicy`. It is one hook because
  // the two copies had already diverged, and the preview's copy did not strip
  // anything.
  const {
    ready,
    allowRemote: effectiveAllowRemote,
    isTrusted,
    showImages,
    allowOnce,
    markSenderTrusted,
  } = useRemoteContentPolicy(accountId, senderEmail, html);

  const { html: sanitizedHtml, hasBlockedImages } = useMemo(
    () => sanitizeEmailHtmlFull(html, effectiveAllowRemote),
    [html, effectiveAllowRemote],
  );

  const handleTrustSender = useCallback(async () => {
    try {
      await api.addTrustedSender(accountId, senderEmail);
      markSenderTrusted();
      addLog('success', 'system', `Trusted ${senderEmail} — remote images will auto-load on future emails.`);
    } catch (err) {
      addLog('error', 'system', `Failed to trust ${senderEmail}: ${errorText(err)}`);
      // Fall back to a one-off image load so the user still gets the images
      // they asked for, even if persistence failed.
      allowOnce();
    }
  }, [accountId, senderEmail, addLog, markSenderTrusted, allowOnce]);

  // Banner appears only when (a) trust resolved to false, (b) there ARE remote
  // images to block, and (c) the user hasn't already overridden with "Show images".
  const bannerVisible = ready && hasBlockedImages && !showImages && isTrusted === false;

  return (
    <>
      {bannerVisible && (
        <div className="flex items-center gap-2 px-4 py-2 bg-amber-50 border-b border-amber-200 text-xs text-amber-800 flex-shrink-0">
          <svg className="w-3.5 h-3.5 flex-shrink-0" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path
              strokeLinecap="round"
              strokeLinejoin="round"
              strokeWidth={2}
              d="M12 9v2m0 4h.01M21 12a9 9 0 11-18 0 9 9 0 0118 0z"
            />
          </svg>
          <span>{t('inbox:emailView.remoteImagesBlocked')}</span>
          <button type="button" onClick={allowOnce} className="ml-1 underline hover:no-underline font-medium">
            {t('inbox:emailView.showImages')}
          </button>
          {senderEmail && (
            <button
              type="button"
              onClick={handleTrustSender}
              className="ml-2 underline hover:no-underline font-medium"
              title={`Auto-load images from ${senderEmail} on future emails`}
            >
              {t('inbox:emailView.trustSender')}
            </button>
          )}
        </div>
      )}
      {ready ? (
        <EmailHtmlFrame
          html={sanitizedHtml}
          highlightQuery={highlightQuery}
          activeMatchIndex={activeMatchIndex}
          onMatchesReported={onMatchesReported}
          className="email-body"
          onMailtoLink={handleMailtoLink}
        />
      ) : (
        <div className="flex items-center gap-2 px-4 py-3 text-sm text-gray-400">
          <div className="animate-spin rounded-full h-3.5 w-3.5 border-b-2 border-gray-300" />
          {t('inbox:loadingEmail')}
        </div>
      )}
    </>
  );
}
