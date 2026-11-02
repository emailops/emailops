import { create } from 'zustand';
import * as api from '@/lib/api';
import { useLogStore } from '@/stores/logStore';
import { useTagStore } from '@/stores/tagStore';
import type { EmailTag, JunkFlaggedAction, JunkVerdict } from '@/types';

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
  /** Verdicts a list must hold before it paints: a failure is the list's to show. */
  fetchVerdicts: (emailIds: string[]) => Promise<void>;
  /** Best effort, for what is already on screen: a failure costs a badge and is logged. */
  loadVerdicts: (emailIds: string[]) => Promise<void>;
  getVerdict: (emailId: string) => JunkVerdict | undefined;
  /** Drop the "already fetched" mark so the next load re-asks the backend. */
  invalidate: (emailId: string) => void;
  setFeedback: (accountId: string, emailId: string, isJunk: boolean) => Promise<void>;
  /** Mirror a "not junk" the backend already recorded (the not-spam thread
   *  action): clear the verdict and the junk chip without another call. */
  markNotJunkLocally: (emailIds: string[]) => void;
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

/** The kind a flagged message shows: what the detector settled on, or plain spam
 *  for one the user confirmed against a `legit` call. */
function junkKind(verdict: JunkVerdict | undefined): string {
  return verdict?.primaryKind && verdict.primaryKind !== 'legit' ? verdict.primaryKind : 'spam';
}

/**
 * The junk chip a row shows. It follows the verdict, so flagged mail keeps its
 * warning with no `junk` tag written or the tag load failed; the user's "not
 * junk" outranks a tag that has not caught up.
 */
export function junkChip(
  emailId: string,
  tags: readonly EmailTag[],
  verdict: JunkVerdict | undefined,
): EmailTag | undefined {
  if (verdict?.userOverride === 'not_junk') return undefined;
  const tag = tags.find((t) => t.tagType === 'junk');
  if (tag) return tag;
  if (!isFlagged(verdict)) return undefined;
  return { emailId, tagType: 'junk', tagValue: junkKind(verdict), confidence: null, createdAt: verdict?.scoredAt ?? 0 };
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

  fetchVerdicts: async (emailIds: string[]) => {
    if (emailIds.length === 0) return;
    const { loaded } = get();
    const missing = emailIds.filter((id) => !(id in loaded));
    if (missing.length === 0) return;

    const verdicts = await api.getJunkVerdicts(missing);
    // Mark every requested id as loaded, including the ones with no verdict —
    // otherwise unscored mail is re-fetched on every list render.
    const loadedNext: Record<string, true> = {};
    for (const id of missing) loadedNext[id] = true;
    set((state) => ({
      verdictsByEmail: { ...state.verdictsByEmail, ...verdicts },
      loaded: { ...state.loaded, ...loadedNext },
    }));
  },

  loadVerdicts: async (emailIds: string[]) => {
    try {
      await get().fetchVerdicts(emailIds);
    } catch (error) {
      useLogStore.getState().addLog('error', 'system', `Could not load junk verdicts: ${String(error)}`);
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

  markNotJunkLocally: (emailIds) => {
    set((state) => {
      const verdictsByEmail = { ...state.verdictsByEmail };
      for (const id of emailIds) {
        const existing = verdictsByEmail[id];
        if (existing) verdictsByEmail[id] = { ...existing, userOverride: 'not_junk' };
      }
      return { verdictsByEmail };
    });
    const tags = useTagStore.getState();
    for (const id of emailIds) {
      tags.setEmailTags(
        id,
        (tags.tagsByEmail[id] ?? []).filter((t) => t.tagType !== 'junk'),
      );
    }
  },

  setFeedback: async (accountId: string, emailId: string, isJunk: boolean) => {
    // Captured before anything is overwritten, so a failed write can put the
    // UI back exactly as the user found it.
    const previousVerdict = get().verdictsByEmail[emailId];
    const previousTags = useTagStore.getState().tagsByEmail[emailId];

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
    const withoutJunk = (previousTags ?? []).filter((t) => t.tagType !== 'junk');
    if (isJunk) {
      useTagStore.getState().setEmailTags(emailId, [
        ...withoutJunk,
        {
          emailId,
          tagType: 'junk',
          // The kind the detector settled on, so confirming a phishing warning
          // does not silently relabel the message as ordinary spam.
          tagValue: junkKind(previousVerdict),
          confidence: null,
          createdAt: Math.floor(Date.now() / 1000),
        },
      ]);
    } else {
      useTagStore.getState().setEmailTags(emailId, withoutJunk);
    }

    try {
      await api.setJunkFeedback(accountId, emailId, isJunk);
    } catch (err) {
      // Undo the optimistic write. The user disagreed with a verdict and the
      // app failed to record it; leaving the message looking corrected is a
      // claim that survives until the next reload and then silently reverses.
      set((state) => {
        if (!previousVerdict) return state;
        return { verdictsByEmail: { ...state.verdictsByEmail, [emailId]: previousVerdict } };
      });
      if (previousTags) useTagStore.getState().setEmailTags(emailId, previousTags);
      // Re-thrown so the caller can tell the user. Swallowing it here is what
      // made the failure invisible in the first place.
      throw err;
    }
  },
}));
