import type { DraftFailedEvent, DraftGeneratedEvent } from '@/lib/api';

/** A `draft-generated` or `draft-failed` event, tagged by kind. */
export type DraftOutcome =
  | { kind: 'generated'; event: DraftGeneratedEvent }
  | { kind: 'failed'; event: DraftFailedEvent };

/**
 * Matches `draft-generated` / `draft-failed` events to the AI-draft request a
 * composer started.
 *
 * The backend returns the request id from the command and emits the outcome
 * as an event, so a fast outcome (e.g. a failure because no model is loaded)
 * can arrive before the command resolves. Matching only on an id stored after
 * the await dropped that event and left the spinner running forever. While a
 * request is pending without an id, events are held; `resolve` hands over the
 * one carrying the new id.
 */
export interface DraftRequestTracker {
  /** A request was sent; its id is not known yet. */
  begin: () => void;
  /** The command returned `requestId`. Returns an outcome that already
   *  arrived for it — apply it now — or null to keep waiting. */
  resolve: (requestId: string) => DraftOutcome | null;
  /** An event arrived. Returns it when it settles the current request (apply
   *  it), null otherwise. */
  accept: (outcome: DraftOutcome) => DraftOutcome | null;
  /** Stop waiting: nothing arriving later is applied. */
  cancel: () => void;
}

export function createDraftRequestTracker(): DraftRequestTracker {
  let pending = false;
  let currentId: string | null = null;
  let early = new Map<string, DraftOutcome>();

  const cancel = () => {
    pending = false;
    currentId = null;
    early = new Map();
  };

  return {
    begin: () => {
      cancel();
      pending = true;
    },
    resolve: (requestId) => {
      if (!pending || currentId !== null) return null;
      const arrived = early.get(requestId) ?? null;
      early = new Map();
      if (arrived) {
        cancel();
        return arrived;
      }
      currentId = requestId;
      return null;
    },
    accept: (outcome) => {
      if (!pending) return null;
      if (currentId === null) {
        early.set(outcome.event.requestId, outcome);
        return null;
      }
      if (outcome.event.requestId !== currentId) return null;
      cancel();
      return outcome;
    },
    cancel,
  };
}
