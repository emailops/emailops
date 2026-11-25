import type { DraftFailedEvent, DraftGeneratedEvent } from '@/lib/api';
import { withoutSignature } from '@/lib/signature';

/** A `draft-generated` or `draft-failed` event, tagged by kind, or the end
 *  of the wait for one (`timedOut`). */
export type DraftOutcome =
  | { kind: 'generated'; event: DraftGeneratedEvent }
  | { kind: 'failed'; event: DraftFailedEvent }
  | { kind: 'timedOut' };

/**
 * How long a composer waits for an AI draft or polish before it gives up,
 * frees its controls and keeps the user's text. Above the backend's own
 * per-call limits (Ollama gives a generation 60 s) and a draft's two or three
 * model calls, with room for a request queued behind another AI task.
 */
export const AI_DRAFT_TIMEOUT_MS = 120_000;

/**
 * Whether the text the user wrote (the composer's HTML, signature left out)
 * holds a link or an image. Polish sends the model plain text and puts its
 * answer in place of that text, so it would drop them: it is not offered then.
 */
export function draftHasLinksOrImages(bodyHtml: string): boolean {
  const typed = new DOMParser().parseFromString(`<body>${withoutSignature(bodyHtml)}</body>`, 'text/html').body;
  return typed.querySelector('a[href], img') !== null;
}

/** An outcome the backend sends: a `draft-generated` or `draft-failed` event. */
export type EventOutcome = Extract<DraftOutcome, { event: unknown }>;

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
  /** A request was sent; its id is not known yet. Returns the request's
   *  token, for `expire`. */
  begin: () => number;
  /** The command returned `requestId`. Returns an outcome that already
   *  arrived for it — apply it now — or null to keep waiting. */
  resolve: (requestId: string) => DraftOutcome | null;
  /** An event arrived. Returns it when it settles the current request (apply
   *  it), null otherwise. */
  accept: (outcome: EventOutcome) => DraftOutcome | null;
  /** Stop waiting: nothing arriving later is applied. */
  cancel: () => void;
  /** The deadline of the request `token` passed. Returns a `timedOut`
   *  outcome to apply when that request is still waiting (it stops waiting),
   *  null when it already settled or another request replaced it. */
  expire: (token: number) => DraftOutcome | null;
}

export function createDraftRequestTracker(): DraftRequestTracker {
  let pending = false;
  let token = 0;
  let currentId: string | null = null;
  let early = new Map<string, EventOutcome>();

  const cancel = () => {
    pending = false;
    currentId = null;
    early = new Map();
  };

  return {
    begin: () => {
      cancel();
      pending = true;
      token += 1;
      return token;
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
    expire: (expired) => {
      if (!pending || expired !== token) return null;
      cancel();
      return { kind: 'timedOut' };
    },
  };
}
