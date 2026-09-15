import type { DraftAttachmentInput, SaveDraftRequest } from '@/lib/api';

/**
 * Snapshot of the composer used to decide whether to auto-save and to build the
 * save request. Pure — no React, unit-testable in isolation.
 */
export interface ComposeDraftState {
  /** Existing draft id being edited, or undefined for a not-yet-saved draft. */
  draftId?: string;
  accountId: string;
  toAddresses: string[];
  ccAddresses: string[];
  subject: string;
  /** Plain-text body (derived from the rich HTML). */
  plainBody: string;
  /** Rich HTML body preserved on the draft for later editing. */
  bodyHtml: string;
  /**
   * File-path attachments this composer manages. `undefined` means the composer
   * doesn't manage attachments (the save leaves the draft's existing files
   * untouched — the backend treats a missing field as "don't touch"). An empty
   * array explicitly clears them.
   */
  attachments?: DraftAttachmentInput[];
  isSending: boolean;
  sent: boolean;
}

/**
 * Auto-save fires only when the composer holds meaningful content and is not
 * mid-send or already sent. This stops an empty (just-opened) composer from
 * creating a blank draft, and stops a save racing a send.
 */
export function shouldAutosaveDraft(s: ComposeDraftState): boolean {
  if (s.isSending || s.sent) return false;
  return s.toAddresses.length > 0 || s.subject.trim().length > 0 || s.plainBody.trim().length > 0;
}

/** Build the backend save request from the composer snapshot. */
export function buildSaveDraftRequest(s: ComposeDraftState): SaveDraftRequest {
  return {
    id: s.draftId,
    accountId: s.accountId,
    toAddresses: s.toAddresses,
    ccAddresses: s.ccAddresses,
    subject: s.subject,
    body: s.plainBody,
    bodyHtml: s.bodyHtml.trim() ? s.bodyHtml : null,
    // Omit the field entirely when unmanaged so the backend preserves existing
    // files; send the (possibly empty) list when the composer manages them.
    ...(s.attachments !== undefined ? { attachments: s.attachments } : {}),
  };
}

/** Persist a draft and return at least its id (structurally compatible with `api.saveDraft`). */
type SaveDraftFn = (req: SaveDraftRequest) => Promise<{ id: string }>;

export interface DraftAutosaver {
  /**
   * Enqueue a save for the given composer snapshot. Resolves when the save
   * (and any earlier queued save) completes.
   */
  save: (state: ComposeDraftState) => Promise<void>;
  /** The id of the draft row this composer is backing, or undefined if none saved yet. */
  currentId: () => string | undefined;
  /** Await any in-flight/queued save and return the final draft id. */
  flush: () => Promise<string | undefined>;
}

/**
 * Serializes debounced auto-saves so a single draft upserts one row instead of
 * piling up duplicates.
 *
 * The bug this prevents: the debounced effect snapshots `draftId` at schedule
 * time, but `saveDraft` is async (a provider round-trip can exceed the debounce
 * window). If a second save fires before the first returns its new id, it also
 * sends `id=undefined` and the backend inserts a fresh row. Here we (a) chain
 * saves so only one runs at a time and (b) inject the freshest id at execution
 * time, so the second save always upserts the row the first created.
 */
export function createDraftAutosaver(
  saveDraft: SaveDraftFn,
  onError?: (err: unknown) => void,
  initialId?: string,
): DraftAutosaver {
  // Seed with an existing draft id when editing (e.g. opened from the Drafts
  // view) so the first save upserts that row instead of creating a duplicate.
  let currentId: string | undefined = initialId;
  let chain: Promise<void> = Promise.resolve();

  const save = (state: ComposeDraftState): Promise<void> => {
    chain = chain.then(async () => {
      try {
        // Read the id produced by any prior save, not the stale snapshot.
        const saved = await saveDraft(buildSaveDraftRequest({ ...state, draftId: currentId }));
        currentId = saved.id;
      } catch (err) {
        onError?.(err);
      }
    });
    return chain;
  };

  const flush = async (): Promise<string | undefined> => {
    await chain;
    return currentId;
  };

  return { save, currentId: () => currentId, flush };
}

/** Debounced front for a draft saver: one save per quiet period, and the
 * pending edit is never lost when the composer closes. */
export interface DebouncedDraftSaver {
  /** Remember `state` and (re)start the quiet-period timer. */
  schedule: (state: ComposeDraftState) => void;
  /** Save right now whatever is still waiting; no-op when nothing is. */
  flushPending: () => void;
  /** Drop a pending save (after a successful send, or a deliberate discard). */
  cancel: () => void;
}

/**
 * The bug this exists for: the debounced effect cleared its timer on unmount,
 * so the last edits before Escape (typically the subject, typed last) never
 * reached the draft row. `flushPending` on close saves them.
 */
export function createDebouncedDraftSaver(
  save: (state: ComposeDraftState) => Promise<void>,
  delayMs: number,
): DebouncedDraftSaver {
  let pending: ComposeDraftState | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;

  const run = () => {
    timer = null;
    const state = pending;
    pending = null;
    if (state) void save(state);
  };
  const cancel = () => {
    if (timer !== null) clearTimeout(timer);
    timer = null;
    pending = null;
  };
  return {
    schedule: (state) => {
      if (timer !== null) clearTimeout(timer);
      pending = state;
      timer = setTimeout(run, delayMs);
    },
    flushPending: () => {
      if (timer === null) return;
      clearTimeout(timer);
      run();
    },
    cancel,
  };
}
