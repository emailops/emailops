// Resolves whether remote content (tracking pixels, hero images, `<video
// poster>`, …) may load for one rendered email.
//
// This lives in a hook rather than inside `EmailBody` because it had already
// drifted: `EmailBody` read the preference and the trusted-sender allowlist and
// sanitised with `sanitizeEmailHtmlFull`, while `EmailPreviewById` — the shared
// panel behind Tasks, Memory and the Lens row drawer — called
// `sanitizeEmailHtml`, which strips nothing remote. The preference defaults to
// OFF, so those three surfaces leaked read receipts the user had explicitly
// declined, with no banner to reveal it.
//
// Duplicating ~30 lines of privacy policy into a second component is what
// produced that gap; one hook with two callers keeps the answer identical
// everywhere.

import { useCallback, useEffect, useState } from 'react';
import * as api from '@/lib/api';
import { errorText } from '@/lib/errors';
import { useLogStore } from '@/stores/logStore';

export interface RemoteContentPolicy {
  /**
   * Both async checks have resolved. Callers must not render the body until
   * this is true: rendering early shows images under the default-true pref,
   * then flashes to "stripped" when the real preference arrives — which both
   * leaks the fetch and looks broken.
   */
  ready: boolean;
  /** The answer: may this email's remote content load? */
  allowRemote: boolean;
  /** `null` while the allowlist lookup is in flight. */
  isTrusted: boolean | null;
  /** The user pressed "show images" for this email only. */
  showImages: boolean;
  /** Allow remote content for this one email, without persisting anything. */
  allowOnce: () => void;
  /** Record that the sender is now trusted (after the backend call succeeds). */
  markSenderTrusted: () => void;
}

/**
 * @param accountId  Account that owns the email — the allowlist is per account.
 * @param senderEmail Sender to look up; `null`/empty resolves to "not trusted".
 * @param emailKey   Changes whenever a different email is shown, so the one-off
 *                   "show images" override does not leak across emails.
 */
export function useRemoteContentPolicy(
  accountId: string,
  senderEmail: string | null,
  emailKey: string | null,
): RemoteContentPolicy {
  const addLog = useLogStore((s) => s.addLog);
  const [allowRemoteContent, setAllowRemoteContent] = useState<boolean | null>(null);
  const [showImages, setShowImages] = useState(false);
  const [isTrusted, setIsTrusted] = useState<boolean | null>(null);

  // The preference is global, so it is read once per mount.
  useEffect(() => {
    let cancelled = false;
    api
      .getPref('privacy.allow_remote_content')
      .then((val) => {
        if (cancelled) return;
        // Default is OFF (block remote content). Only allow on explicit "true".
        setAllowRemoteContent(val === 'true');
      })
      .catch(() => {
        if (!cancelled) setAllowRemoteContent(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // Reset per-email overrides whenever a new email is shown, then check whether
  // this sender is on the allowlist for the current account.
  // biome-ignore lint/correctness/useExhaustiveDependencies: emailKey is a trigger, not a value read inside the effect — without it a one-off "show images" would carry over to the next email from the same sender.
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
  }, [emailKey, accountId, senderEmail, addLog]);

  const allowOnce = useCallback(() => setShowImages(true), []);
  const markSenderTrusted = useCallback(() => setIsTrusted(true), []);

  return {
    ready: allowRemoteContent !== null && isTrusted !== null,
    allowRemote: (allowRemoteContent ?? false) || showImages || isTrusted === true,
    isTrusted,
    showImages,
    allowOnce,
    markSenderTrusted,
  };
}
