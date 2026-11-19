// The draft on screen, for the chat panel.
//
// One slot: only one composer is ever visible (an active compose tab hides
// the thread, and an inline reply is only seen with its thread). A compose
// tab registers while it is the active tab, an inline reply (reply or reply
// all, never forward) while it is mounted. Each removes the entry on the way
// out only if it is still its own, so a late cleanup never clears a newer
// composer.
//
// The chip needs the subject and whether there is text on every render, so
// the composer publishes those two as the user types; the full snapshot is
// read only when a turn is sent.

import { useEffect, useLayoutEffect, useMemo, useRef } from 'react';
import { create } from 'zustand';
import { draftContextOf, userTextOf } from '@/lib/chatDraft';
import type { ChatDraftContext, DraftKind } from '@/types';

export interface DraftHandle {
  /** Fresh per composer mount: a remounted composer is another draft. */
  instanceId: string;
  kind: DraftKind;
  /** The account the reply's email lives in, or the compose tab's From
   *  account, read when a turn is sent (the From selector can change it). */
  accountId: () => string;
  /** The draft as chat context, or `null` while it holds no text of the user. */
  snapshot: () => ChatDraftContext | null;
}

export interface DraftPublished {
  subject: string;
  hasUserText: boolean;
}

interface DraftContextState extends DraftPublished {
  active: DraftHandle | null;
  register: (handle: DraftHandle, published: DraftPublished) => void;
  unregister: (instanceId: string) => void;
  publish: (instanceId: string, published: DraftPublished) => void;
}

export const useDraftContextStore = create<DraftContextState>((set) => ({
  active: null,
  subject: '',
  hasUserText: false,
  register: (handle, published) => set({ active: handle, ...published }),
  unregister: (instanceId) =>
    set((s) => (s.active?.instanceId === instanceId ? { active: null, subject: '', hasUserText: false } : s)),
  publish: (instanceId, published) => set((s) => (s.active?.instanceId === instanceId ? published : s)),
}));

/** What a composer holds, as its own state has it. */
export interface DraftSource {
  kind: DraftKind;
  /** The account the reply's email lives in, or the compose tab's From account. */
  accountId: string;
  subject: string;
  to: string[];
  cc: string[];
  bodyHtml: string;
  replyToEmailId: string | null;
}

/**
 * Offer the calling composer to the chat panel as the draft on screen while
 * `enabled` (an active compose tab; an inline reply or reply-all). The chip's
 * fields follow the user's typing; the snapshot is taken only when a turn is
 * sent, from the latest state.
 */
export function useDraftContextSource(enabled: boolean, source: DraftSource): void {
  const instanceId = useMemo(() => crypto.randomUUID(), []);
  const latest = useRef(source);
  useLayoutEffect(() => {
    latest.current = source;
  });
  const hasUserText = useMemo(() => userTextOf(source.bodyHtml) !== '', [source.bodyHtml]);
  const { kind, subject } = source;
  // The published fields are read once here, on purpose: a change of text or
  // subject is published below without registering again.
  // biome-ignore lint/correctness/useExhaustiveDependencies: registration follows the composer, not its text
  useEffect(() => {
    if (!enabled) return;
    useDraftContextStore.getState().register(
      {
        instanceId,
        kind,
        accountId: () => latest.current.accountId,
        snapshot: () => draftContextOf(latest.current),
      },
      { subject, hasUserText },
    );
    return () => useDraftContextStore.getState().unregister(instanceId);
  }, [enabled, instanceId, kind]);
  useEffect(() => {
    if (enabled) useDraftContextStore.getState().publish(instanceId, { subject, hasUserText });
  }, [enabled, instanceId, subject, hasUserText]);
}
