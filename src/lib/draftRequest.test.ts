import { describe, expect, it } from 'vitest';
import type { DraftFailedEvent, DraftGeneratedEvent } from '@/lib/api';
import { createDraftRequestTracker, type DraftOutcome } from './draftRequest';

function generated(requestId: string): DraftOutcome {
  return { kind: 'generated', event: { requestId, emailId: '', body: 'hi', sources: [] } as DraftGeneratedEvent };
}
function failed(requestId: string): DraftOutcome {
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
});
