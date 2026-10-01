import { useEffect, useRef } from 'react';
import * as api from '@/lib/api';
import { applySignature, NO_SIGNATURE, type Signature, stripSignature } from '@/lib/signature';

function isEmpty(sig: Signature): boolean {
  return !sig.text.trim() && !sig.image;
}

/**
 * Keeps the From account's signature (text + optional image) in a composer
 * body.
 *
 * On open and whenever `accountId` changes, the account's signature is loaded
 * and applied to the body through `setBodyHtml(updater)`: added once, swapped
 * for the new account's, or removed if that account has none. Replacing the
 * body afterwards (an AI draft arriving) drops the signature; call the
 * returned `withSignature(html)` on the new body to put it back.
 *
 * Stale loads are ignored, so switching accounts quickly never applies the
 * wrong account's signature. A failed load leaves the body untouched.
 */
export function useComposeSignature(
  accountId: string,
  setBodyHtml: (update: (current: string) => string) => void,
  enabled = true,
): {
  withSignature: (html: string) => string;
  /** For an AI draft: adds the signature only when the AI sign-off setting
   *  is "custom signature" (otherwise the AI already signed, or must not). */
  withDraftSignature: (html: string) => string;
} {
  const signatureRef = useRef<Signature>(NO_SIGNATURE);
  const draftModeRef = useRef<string | null>(null);
  const setBodyRef = useRef(setBodyHtml);
  setBodyRef.current = setBodyHtml;

  useEffect(() => {
    let cancelled = false;
    api
      .getPref('draft_signoff')
      .then((mode) => {
        if (!cancelled) draftModeRef.current = mode;
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!enabled || !accountId) return;
    let cancelled = false;
    api
      .getFullSignature(accountId)
      .then((signature) => {
        if (cancelled) return;
        const previous = signatureRef.current;
        signatureRef.current = signature;
        // Nothing to add and nothing to take away: leave the body alone so the
        // editor is not reset for accounts without a signature.
        if (isEmpty(signature) && isEmpty(previous)) return;
        setBodyRef.current((current) => applySignature(current, signature));
      })
      .catch(() => {
        // Non-fatal: the user can still type and send without a signature.
      });
    return () => {
      cancelled = true;
    };
  }, [accountId, enabled]);

  const withSignature = (html: string) =>
    isEmpty(signatureRef.current) ? html : applySignature(html, signatureRef.current);
  return {
    withSignature,
    // "name" / "none": the draft ends as the AI was told — no signature on top.
    withDraftSignature: (html: string) =>
      draftModeRef.current === 'name' || draftModeRef.current === 'none' ? stripSignature(html) : withSignature(html),
  };
}
