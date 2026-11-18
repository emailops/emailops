import { beforeEach, describe, expect, it } from 'vitest';
import { type DraftHandle, useDraftContextStore } from './draftContextStore';

function handle(instanceId: string): DraftHandle {
  return { instanceId, kind: 'reply', accountId: () => 'acc-1', snapshot: () => null };
}

const store = () => useDraftContextStore.getState();

describe('draftContextStore', () => {
  beforeEach(() => {
    useDraftContextStore.setState({ active: null, subject: '', hasUserText: false });
  });

  it('holds the composer that registered last', () => {
    store().register(handle('a'), { subject: 'First', hasUserText: true });
    store().register(handle('b'), { subject: 'Second', hasUserText: false });
    expect(store().active?.instanceId).toBe('b');
    expect(store().subject).toBe('Second');
    expect(store().hasUserText).toBe(false);
  });

  it('keeps the entry when another composer unregisters', () => {
    store().register(handle('b'), { subject: 'Second', hasUserText: true });
    store().unregister('a');
    expect(store().active?.instanceId).toBe('b');
  });

  it('clears the entry when its own composer unregisters', () => {
    store().register(handle('a'), { subject: 'First', hasUserText: true });
    store().unregister('a');
    expect(store().active).toBeNull();
    expect(store().hasUserText).toBe(false);
  });

  it('takes what the registered composer publishes as the user types', () => {
    store().register(handle('a'), { subject: '', hasUserText: false });
    store().publish('a', { subject: 'Re: Q3 report', hasUserText: true });
    expect(store().subject).toBe('Re: Q3 report');
    expect(store().hasUserText).toBe(true);
  });

  it('ignores what another composer publishes', () => {
    store().register(handle('b'), { subject: 'Second', hasUserText: false });
    store().publish('a', { subject: 'First', hasUserText: true });
    expect(store().subject).toBe('Second');
    expect(store().hasUserText).toBe(false);
  });
});
