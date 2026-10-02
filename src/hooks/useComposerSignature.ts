import { useCallback, useEffect, useRef } from 'react';
import { applyAccountSignature, type ComposeKind, signatureFor } from '@/lib/signature';
import { useSignatureStore } from '@/stores/signatureStore';

interface ComposerSignatureOptions {
  /** The From account. A change swaps the signature block. */
  accountId: string;
  kind: ComposeKind;
  /**
   * The composer starts fresh: insert the account's signature once it loads.
   * False for a body that already exists (a reopened draft, a message taken
   * back from the outbox, a maximized composer): its signature is already in
   * it, edits included, and is only swapped if the From account changes.
   */
  insertOnOpen: boolean;
  setBodyHtml: (update: (current: string) => string) => void;
}

/**
 * Keeps the From account's signature in a composer body: inserted when a
 * fresh composer opens, swapped when the account changes. Returns a stable
 * getter for the cached signature of the current account, for a composer
 * that rebuilds its body (an AI draft, a forward) and must put it back.
 */
export function useComposerSignature({ accountId, kind, insertOnOpen, setBodyHtml }: ComposerSignatureOptions) {
  const load = useSignatureStore((s) => s.load);
  const kindRef = useRef(kind);
  kindRef.current = kind;
  const setBodyRef = useRef(setBodyHtml);
  setBodyRef.current = setBodyHtml;
  // The account whose signature the body holds: none yet for a fresh
  // composer, the opening account for an existing body.
  const appliedAccount = useRef<string | null>(insertOnOpen ? null : accountId);

  useEffect(() => {
    if (appliedAccount.current === accountId) return;
    let cancelled = false;
    void load(accountId).then((signature) => {
      if (cancelled) return;
      appliedAccount.current = accountId;
      const html = signatureFor(signature, kindRef.current);
      setBodyRef.current((current) =>
        applyAccountSignature(current, html, kindRef.current, { insertIfMissing: insertOnOpen }),
      );
    });
    return () => {
      cancelled = true;
    };
  }, [accountId, insertOnOpen, load]);

  // Stable, so a composer can call it from an effect without re-running
  // that effect when the account changes.
  const accountRef = useRef(accountId);
  accountRef.current = accountId;
  return useCallback(
    (): string | null => signatureFor(useSignatureStore.getState().byAccount[accountRef.current], kindRef.current),
    [],
  );
}
