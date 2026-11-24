import { describe, expect, it } from 'vitest';
import type { DraftFailedEvent, DraftGeneratedEvent } from '@/lib/api';
import { createDraftRequestTracker, type EventOutcome } from './draftRequest';

function generated(requestId: string): EventOutcome {
  return { kind: 'generated', event: { requestId, emailId: '', body: 'hi', sources: [] } as DraftGeneratedEvent };
}
function failed(requestId: string): EventOutcome {
  return { kind: 'failed', event: { requestId, emailId: '', error: 'boom' } as DraftFailedEvent };
}

describe('createDraftRequestTracker', () => {
  it('applies the event of the request once its id is known', () => {
    const tracker = createDraftRequestTracker();
    tracker.begin();
    expect(tracker.resolve('r1')).toBeNull();
    const outcome = generated('r1');
    expect(tracker.accept(outcome)).toBe(outcome);
  });

  it('ignores events of other requests', () => {
    const tracker = createDraftRequestTracker();
    tracker.begin();
    tracker.resolve('r1');
    expect(tracker.accept(generated('other'))).toBeNull();
  });

  it('holds an event that beat the command returning its id, and hands it over on resolve', () => {
    // A fast failure (no model loaded) emits draft-failed before
    // generate_draft's invoke resolves; dropping it left the spinner forever.
    const tracker = createDraftRequestTracker();
    tracker.begin();
    const early = failed('r1');
    expect(tracker.accept(early)).toBeNull();
    expect(tracker.resolve('r1')).toBe(early);
    // Settled: a duplicate does not apply twice.
    expect(tracker.accept(generated('r1'))).toBeNull();
  });

  it('does not hand over an early event that belongs to another request', () => {
    const tracker = createDraftRequestTracker();
    tracker.begin();
    tracker.accept(generated('other'));
    expect(tracker.resolve('r1')).toBeNull();
  });

  it('applies nothing after cancel', () => {
    const tracker = createDraftRequestTracker();
    tracker.begin();
    tracker.resolve('r1');
    tracker.cancel();
    expect(tracker.accept(generated('r1'))).toBeNull();
  });

  it('ignores a resolve that arrives after cancel', () => {
    const tracker = createDraftRequestTracker();
    tracker.begin();
    tracker.accept(generated('r1'));
    tracker.cancel();
    expect(tracker.resolve('r1')).toBeNull();
    expect(tracker.accept(generated('r1'))).toBeNull();
  });

  it('times out a request still waiting at its deadline, and drops what arrives later', () => {
    const tracker = createDraftRequestTracker();
    const token = tracker.begin();
    tracker.resolve('r1');
    expect(tracker.expire(token)).toEqual({ kind: 'timedOut' });
    // The late answer must not overwrite what the user did meanwhile.
    expect(tracker.accept(generated('r1'))).toBeNull();
  });

  it('times out a request whose command never returned its id', () => {
    const tracker = createDraftRequestTracker();
    const token = tracker.begin();
    expect(tracker.expire(token)).toEqual({ kind: 'timedOut' });
    expect(tracker.resolve('r1')).toBeNull();
  });

  it('does not time out a request that already settled', () => {
    const tracker = createDraftRequestTracker();
    const token = tracker.begin();
    tracker.resolve('r1');
    tracker.accept(generated('r1'));
    expect(tracker.expire(token)).toBeNull();
  });

  it("an earlier request's deadline does not time out the next one", () => {
    const tracker = createDraftRequestTracker();
    const first = tracker.begin();
    tracker.resolve('r1');
    tracker.accept(generated('r1'));
    tracker.begin();
    tracker.resolve('r2');
    expect(tracker.expire(first)).toBeNull();
    const outcome = generated('r2');
    expect(tracker.accept(outcome)).toBe(outcome);
  });
});
