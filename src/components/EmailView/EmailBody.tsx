import { useCallback, useEffect, useMemo, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { EmailHtmlFrame } from '@/components/shared/EmailHtmlFrame';
import * as api from '@/lib/api';
import { plainTextToHtml } from '@/lib/composeHtml';
import { type ParsedMailto, sanitizeEmailHtmlFull } from '@/lib/emailFormatting';
import { errorText } from '@/lib/errors';
import { useEmailStore } from '@/stores/emailStore';
import { useLogStore } from '@/stores/logStore';

export function EmailBody({
  html,
  highlightQuery,
  scrollToFirstMatch,
  accountId,
  senderEmail,
}: {
  html: string;
  highlightQuery?: string | null;
  scrollToFirstMatch?: boolean;
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
  // Both async checks start unresolved. We defer rendering the iframe until
  // both have completed — otherwise images would briefly load with the default
  // pref (true), then flash to "stripped" once the pref resolves to false, and
  // finally flash back to "loaded" once the trust check returns true. That
  // sequence both leaks privacy (the initial flash) and produces a confusing
  // "no banner, no images" state for trusted senders.
  const [allowRemoteContent, setAllowRemoteContent] = useState<boolean | null>(null);
  const [showImages, setShowImages] = useState(false);
  const [isTrusted, setIsTrusted] = useState<boolean | null>(null);

  // Load the remote-content preference once when the email body mounts.
  useEffect(() => {
    let cancelled = false;
    api
      .getPref('privacy.allow_remote_content')
      .then((val) => {
        if (cancelled) return;
        // Default is OFF (block remote content). Only allow if explicitly "true".
        setAllowRemoteContent(val === 'true');
      })
      .catch(() => {
        if (!cancelled) setAllowRemoteContent(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // Reset per-email overrides whenever a new email is loaded, then check
  // whether this sender is on the trusted allowlist for the current account.
  useEffect(() => {
    setShowImages(false);
    setIsTrusted(null);
    if (!senderEmail || !accountId) {
      setIsTrusted(false);
      return;
    }
    let cancelled = false;
    api
      .isSenderTrusted(accountId, senderEmail)
      .then((trusted) => {
        if (!cancelled) setIsTrusted(trusted);
      })
      .catch((err) => {
        if (cancelled) return;
        // Surface the failure so the user can see it in the output panel rather
        // than silently treating the sender as untrusted forever.
        addLog('error', 'system', `Trusted-sender check failed for ${senderEmail}: ${errorText(err)}`);
        setIsTrusted(false);
      });
    return () => {
      cancelled = true;
    };
  }, [html, accountId, senderEmail, addLog]);

  const ready = allowRemoteContent !== null && isTrusted !== null;
  const effectiveAllowRemote = (allowRemoteContent ?? false) || showImages || isTrusted === true;

  const { html: sanitizedHtml, hasBlockedImages } = useMemo(
    () => sanitizeEmailHtmlFull(html, effectiveAllowRemote),
    [html, effectiveAllowRemote],
  );

  const handleTrustSender = useCallback(async () => {
    try {
      await api.addTrustedSender(accountId, senderEmail);
      setIsTrusted(true);
      addLog('success', 'system', `Trusted ${senderEmail} — remote images will auto-load on future emails.`);
    } catch (err) {
      addLog('error', 'system', `Failed to trust ${senderEmail}: ${errorText(err)}`);
      // Fall back to a one-off image load so the user still gets the images
      // they asked for, even if persistence failed.
      setShowImages(true);
    }
  }, [accountId, senderEmail, addLog]);

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
          <button
            type="button"
            onClick={() => setShowImages(true)}
            className="ml-1 underline hover:no-underline font-medium"
          >
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
          scrollToFirstMatch={scrollToFirstMatch}
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
