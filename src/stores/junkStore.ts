import { create } from 'zustand';
import * as api from '@/lib/api';
import { useTagStore } from '@/stores/tagStore';
import type { JunkFlaggedAction, JunkVerdict } from '@/types';

/**
 * Junk verdicts, loaded in batches alongside the email list.
 *
 * A missing entry means "not scored yet" and must render as nothing — the
 * detector distinguishes "no evidence" from "clean", and so does the UI.
 */
interface JunkStore {
  verdictsByEmail: Record<string, JunkVerdict>;
  /** Mirrors the `junk_flagged_action` preference. */
  flaggedAction: JunkFlaggedAction;
  loadConfig: () => Promise<void>;
  /** Same preference the Settings tab writes — one source of truth, so the two
   *  controls can never disagree about what the inbox is doing. */
  setFlaggedAction: (action: JunkFlaggedAction) => Promise<void>;
  /** Ids already fetched, so an unscored email isn't re-requested every render. */
  loaded: Record<string, true>;
  loadVerdicts: (emailIds: string[]) => Promise<void>;
  getVerdict: (emailId: string) => JunkVerdict | undefined;
  /** Drop the "already fetched" mark so the next load re-asks the backend. */
  invalidate: (emailId: string) => void;
  setFeedback: (accountId: string, emailId: string, isJunk: boolean) => Promise<void>;
}

/** Should this verdict show a badge? A `not_junk` override always wins. */
export function isFlagged(verdict: JunkVerdict | undefined): boolean {
  if (!verdict) return false;
  if (verdict.userOverride === 'not_junk') return false;
  return verdict.userOverride === 'junk' || verdict.band === 'junk';
}

/** Should the message be visually deprioritized in the list? */
export function isDeprioritized(verdict: JunkVerdict | undefined): boolean {
  return isFlagged(verdict);
}

/**
 * Should the row be dropped from the inbox list entirely?
 *
 * Only when the user asked for it. "Hide" removes the row from this view; it
 * never moves or deletes anything on the server, and the message stays
 * reachable through search and the provider's own folders.
 */
export function isHiddenFromInbox(verdict: JunkVerdict | undefined, action: JunkFlaggedAction): boolean {
  return action === 'hide' && isFlagged(verdict);
}

export const useJunkStore = create<JunkStore>((set, get) => ({
  verdictsByEmail: {},
  loaded: {},
  flaggedAction: 'dim',

  loadConfig: async () => {
    try {
      const config = await api.getJunkConfig();
      set({ flaggedAction: config.flaggedAction });
    } catch {
      // Fall back to the least destructive behaviour: fading a row is always
      // recoverable, hiding one is not obvious to the user.
    }
  },

  loadVerdicts: async (emailIds: string[]) => {
    if (emailIds.length === 0) return;
    const { loaded } = get();
    const missing = emailIds.filter((id) => !(id in loaded));
    if (missing.length === 0) return;

    try {
      const verdicts = await api.getJunkVerdicts(missing);
      // Mark every requested id as loaded, including the ones with no verdict —
      // otherwise unscored mail is re-fetched on every list render.
      const loadedNext: Record<string, true> = {};
      for (const id of missing) loadedNext[id] = true;
      set((state) => ({
        verdictsByEmail: { ...state.verdictsByEmail, ...verdicts },
        loaded: { ...state.loaded, ...loadedNext },
      }));
    } catch {
      // Non-critical: badges just won't show. The mail itself is unaffected.
    }
  },

  setFlaggedAction: async (action: JunkFlaggedAction) => {
    const previous = get().flaggedAction;
    set({ flaggedAction: action });
    try {
      const config = await api.getJunkConfig();
      await api.setJunkConfig({ ...config, flaggedAction: action });
    } catch {
      // Roll back: leaving the checkbox ticked while the inbox still shows
      // everything would be worse than the toggle appearing not to work.
      set({ flaggedAction: previous });
    }
  },

  getVerdict: (emailId: string) => get().verdictsByEmail[emailId],

  /**
   * Forget the cached "already asked about this one" mark.
   *
   * `loadVerdicts` records every requested id as loaded, including ids that came
   * back with no verdict — otherwise unscored mail is re-fetched on every list
   * render. The cost is that a message viewed BEFORE scoring reached it stays
   * banner-less for the rest of the session: the store never asks again. Scoring
   * now runs inside the sync, but a backfill or a re-score can still land after
   * a message has been opened, so the scored event has to clear the mark.
   */
  invalidate: (emailId: string) => {
    set((state) => {
      const { [emailId]: _dropped, ...loaded } = state.loaded;
      return { loaded };
    });
  },

  setFeedback: async (accountId: string, emailId: string, isJunk: boolean) => {
    // Optimistic: the user's correction should feel instant, and the backend
    // write is what makes it durable.
    set((state) => {
      const existing = state.verdictsByEmail[emailId];
      if (!existing) return state;
      return {
        verdictsByEmail: {
          ...state.verdictsByEmail,
          [emailId]: { ...existing, userOverride: isJunk ? 'junk' : 'not_junk' },
        },
      };
    });

    // The inbox row reads its badge and its dimming from the tag store, not
    // from here. Without this the message stays faded with its chip after the
    // user has just said it is fine — the correction would look ignored until
    // the next reload.
    const tags = useTagStore.getState().tagsByEmail[emailId] ?? [];
    const withoutJunk = tags.filter((t) => t.tagType !== 'junk');
    if (isJunk) {
      useTagStore.getState().setEmailTags(emailId, [
        ...withoutJunk,
        {
          emailId,
          tagType: 'junk',
          tagValue: 'spam',
          confidence: null,
          createdAt: Math.floor(Date.now() / 1000),
        },
      ]);
    } else {
      useTagStore.getState().setEmailTags(emailId, withoutJunk);
    }

    await api.setJunkFeedback(accountId, emailId, isJunk);
  },
}));
