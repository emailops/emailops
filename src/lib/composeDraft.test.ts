import { describe, expect, it, vi } from 'vitest';
import {
  buildSaveDraftRequest,
  type ComposeDraftState,
  createDraftAutosaver,
  shouldAutosaveDraft,
} from './composeDraft';

const base: ComposeDraftState = {
  accountId: 'acct-1',
  toAddresses: [],
  ccAddresses: [],
  subject: '',
  plainBody: '',
  bodyHtml: '',
  isSending: false,
  sent: false,
};

describe('shouldAutosaveDraft', () => {
  it('is false for a just-opened empty composer', () => {
    expect(shouldAutosaveDraft(base)).toBe(false);
  });

  it('is false when only whitespace is present', () => {
    expect(shouldAutosaveDraft({ ...base, subject: '   ', plainBody: '\n\t' })).toBe(false);
  });

  it('is true once there is a recipient', () => {
    expect(shouldAutosaveDraft({ ...base, toAddresses: ['a@b.com'] })).toBe(true);
  });

  it('is true once there is a subject or body', () => {
    expect(shouldAutosaveDraft({ ...base, subject: 'Hi' })).toBe(true);
    expect(shouldAutosaveDraft({ ...base, plainBody: 'body' })).toBe(true);
  });

  it('is false while sending or after sent so a save cannot race the send', () => {
    expect(shouldAutosaveDraft({ ...base, subject: 'Hi', isSending: true })).toBe(false);
    expect(shouldAutosaveDraft({ ...base, subject: 'Hi', sent: true })).toBe(false);
  });
});

describe('buildSaveDraftRequest', () => {
  it('carries the draft id, recipients, and nulls empty html', () => {
    const req = buildSaveDraftRequest({
      ...base,
      draftId: 'd1',
      toAddresses: ['a@b.com'],
      ccAddresses: ['c@b.com'],
      subject: 'Hi',
      plainBody: 'hello',
      bodyHtml: '   ',
    });
    expect(req.id).toBe('d1');
    expect(req.toAddresses).toEqual(['a@b.com']);
    expect(req.ccAddresses).toEqual(['c@b.com']);
    expect(req.body).toBe('hello');
    expect(req.bodyHtml).toBeNull();
  });

  it('keeps non-empty html', () => {
    const req = buildSaveDraftRequest({ ...base, subject: 'Hi', bodyHtml: '<p>hi</p>' });
    expect(req.bodyHtml).toBe('<p>hi</p>');
  });
});

describe('createDraftAutosaver', () => {
  const withBody = (plainBody: string): ComposeDraftState => ({ ...base, subject: 'Hi', plainBody });

  it('reuses the id from the first save on the next save (no duplicate rows)', async () => {
    const seen: (string | undefined)[] = [];
    let n = 0;
    const saveDraft = vi.fn(async (req) => {
      seen.push(req.id);
      const id = req.id ?? `d${++n}`;
      return { id };
    });
    const autosaver = createDraftAutosaver(saveDraft);

    await autosaver.save(withBody('Buen'));
    await autosaver.save(withBody('Buenas tardes'));

    expect(saveDraft).toHaveBeenCalledTimes(2);
    expect(seen[0]).toBeUndefined(); // first save creates the row
    expect(seen[1]).toBe('d1'); // second save upserts the same row
  });

  it('serializes overlapping saves so the second still sees the first id', async () => {
    // First save is slow; a second save is enqueued before it resolves. Without
    // serialization both would send id=undefined and create two rows.
    const seen: (string | undefined)[] = [];
    let releaseFirst: (v: { id: string }) => void = () => {};
    let call = 0;
    const saveDraft = vi.fn((req) => {
      seen.push(req.id);
      call += 1;
      if (call === 1) {
        return new Promise<{ id: string }>((resolve) => {
          releaseFirst = resolve;
        });
      }
      return Promise.resolve({ id: req.id ?? 'unexpected-new-id' });
    });
    const autosaver = createDraftAutosaver(saveDraft);

    const p1 = autosaver.save(withBody('Buen'));
    const p2 = autosaver.save(withBody('Buenas tardes')); // enqueued while #1 in flight
    await vi.waitFor(() => expect(saveDraft).toHaveBeenCalledTimes(1)); // first save is in flight
    releaseFirst({ id: 'd1' });
    await Promise.all([p1, p2]);

    expect(seen[0]).toBeUndefined();
    expect(seen[1]).toBe('d1'); // reused, not a fresh undefined → no second row
  });

  it('flush awaits an in-flight save and returns its id (for delete-after-send)', async () => {
    let releaseFirst: (v: { id: string }) => void = () => {};
    const saveDraft = vi.fn(
      () =>
        new Promise<{ id: string }>((resolve) => {
          releaseFirst = resolve;
        }),
    );
    const autosaver = createDraftAutosaver(saveDraft);

    autosaver.save(withBody('Buen'));
    await vi.waitFor(() => expect(saveDraft).toHaveBeenCalledTimes(1));
    expect(autosaver.currentId()).toBeUndefined(); // not resolved yet

    const flushed = autosaver.flush();
    releaseFirst({ id: 'd1' });
    expect(await flushed).toBe('d1');
    expect(autosaver.currentId()).toBe('d1');
  });

  it('reports errors without breaking the id chain', async () => {
    const onError = vi.fn();
    let call = 0;
    const saveDraft = vi.fn(async (req) => {
      call += 1;
      if (call === 1) throw new Error('network');
      return { id: req.id ?? 'd1' };
    });
    const autosaver = createDraftAutosaver(saveDraft, onError);

    await autosaver.save(withBody('Buen')); // fails
    await autosaver.save(withBody('Buenas')); // recovers, still no id yet → creates row

    expect(onError).toHaveBeenCalledTimes(1);
    expect(saveDraft).toHaveBeenCalledTimes(2);
  });
});
