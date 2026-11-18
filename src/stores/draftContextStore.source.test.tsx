import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { type DraftSource, useDraftContextSource, useDraftContextStore } from './draftContextStore';

function Composer({ enabled, source }: { enabled: boolean; source: DraftSource }) {
  useDraftContextSource(enabled, source);
  return null;
}

const source: DraftSource = {
  kind: 'reply',
  accountId: 'acc-1',
  subject: 'Re: Q3 report',
  to: ['ana@example.com'],
  cc: [],
  bodyHtml: '<p></p>',
  replyToEmailId: 'email-1',
};

let container: HTMLDivElement;
let root: Root;
const store = () => useDraftContextStore.getState();
const render = (enabled: boolean, s: DraftSource) => act(() => root.render(<Composer enabled={enabled} source={s} />));

beforeEach(() => {
  useDraftContextStore.setState({ active: null, subject: '', hasUserText: false });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe('useDraftContextSource', () => {
  it('registers the composer while it is enabled', () => {
    render(true, source);
    expect(store().active?.kind).toBe('reply');
    expect(store().subject).toBe('Re: Q3 report');
    expect(store().hasUserText).toBe(false);
  });

  it('does not register a composer that is not on screen', () => {
    render(false, source);
    expect(store().active).toBeNull();
  });

  it('registers when the composer comes on screen and leaves when it goes', () => {
    render(false, source);
    render(true, source);
    expect(store().active).not.toBeNull();
    render(false, source);
    expect(store().active).toBeNull();
  });

  it('publishes that there is text as the user types, and the snapshot has the latest text', () => {
    render(true, source);
    render(true, { ...source, bodyHtml: '<p>The report is attached.</p>' });
    expect(store().hasUserText).toBe(true);
    expect(store().active?.snapshot()?.body).toBe('The report is attached.');
  });

  it('reads the account when asked, so a change of sender counts', () => {
    render(true, source);
    render(true, { ...source, accountId: 'acc-2' });
    expect(store().active?.accountId()).toBe('acc-2');
  });

  it('clears the entry when the composer unmounts', () => {
    render(true, source);
    act(() => root.render(null));
    expect(store().active).toBeNull();
  });
});
