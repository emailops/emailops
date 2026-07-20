// Occurrence-level search support in the email body iframe. The parent posts
// a highlight command with the query and (optionally) which occurrence inside
// this body is the globally active one; the bridge wraps matches in <mark>s,
// styles the active one distinctly, and reports back how many occurrences it
// found plus the active mark's vertical position — the parent needs both to
// build the "N of M" counter and to scroll the thread container (a sandboxed
// null-origin iframe cannot scroll its parent itself).

import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { BRIDGE_SCRIPT } from './EmailHtmlFrame';

function runBridge() {
  new Function(BRIDGE_SCRIPT)();
}

function postHighlight(query: string, activeIndex: number | null) {
  window.dispatchEvent(
    new MessageEvent('message', {
      data: { __emailFrameCmd: 'highlight', query, activeIndex },
    }),
  );
}

interface MatchesMessage {
  __emailFrame?: boolean;
  type?: string;
  count?: number;
  activeTop?: number | null;
}

function lastMatchesMessage(calls: unknown[][]): MatchesMessage | null {
  const msgs = calls
    .map((call) => call[0] as MatchesMessage)
    .filter((msg) => msg?.__emailFrame && msg.type === 'matches');
  return msgs.length > 0 ? msgs[msgs.length - 1] : null;
}

function marks(): HTMLElement[] {
  return Array.from(document.querySelectorAll('mark[data-email-search-mark]'));
}

describe('EmailHtmlFrame bridge occurrence reporting', () => {
  beforeAll(() => {
    runBridge();
  });

  beforeEach(() => {
    vi.restoreAllMocks();
    document.body.innerHTML = '<p>alpha beta alpha</p><p>gamma alpha</p>';
  });

  it('reports the number of wrapped occurrences after highlighting', () => {
    const spy = vi.spyOn(window, 'postMessage');
    postHighlight('alpha', null);
    expect(marks()).toHaveLength(3);
    const msg = lastMatchesMessage(spy.mock.calls);
    expect(msg?.count).toBe(3);
    expect(msg?.activeTop).toBeNull();
  });

  it('marks the requested occurrence as active and reports its position', () => {
    const spy = vi.spyOn(window, 'postMessage');
    postHighlight('alpha', 1);
    const all = marks();
    expect(all[1].getAttribute('data-email-search-active')).toBe('1');
    expect(all[0].getAttribute('data-email-search-active')).toBeNull();
    const msg = lastMatchesMessage(spy.mock.calls);
    expect(msg?.count).toBe(3);
    expect(typeof msg?.activeTop).toBe('number');
  });

  it('moves the active styling when a new index is posted', () => {
    postHighlight('alpha', 0);
    expect(marks()[0].getAttribute('data-email-search-active')).toBe('1');
    postHighlight('alpha', 2);
    const all = marks();
    expect(all[0].getAttribute('data-email-search-active')).toBeNull();
    expect(all[2].getAttribute('data-email-search-active')).toBe('1');
  });

  it('reports zero occurrences and clears marks for an empty query', () => {
    postHighlight('alpha', 0);
    expect(marks().length).toBeGreaterThan(0);
    const spy = vi.spyOn(window, 'postMessage');
    postHighlight('', null);
    expect(marks()).toHaveLength(0);
    const msg = lastMatchesMessage(spy.mock.calls);
    expect(msg?.count).toBe(0);
    expect(msg?.activeTop).toBeNull();
  });

  it('reports no active position for an out-of-range index', () => {
    const spy = vi.spyOn(window, 'postMessage');
    postHighlight('alpha', 99);
    const msg = lastMatchesMessage(spy.mock.calls);
    expect(msg?.count).toBe(3);
    expect(msg?.activeTop).toBeNull();
  });
});
