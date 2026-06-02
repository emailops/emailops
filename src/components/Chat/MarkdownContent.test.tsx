// Regression tests for the URI-scheme handlers in `MarkdownContent`.
//
// react-markdown 10 ships a default `urlTransform` whose safe-protocol
// allowlist is `https?|ircs?|mailto|xmpp`. That silently rewrites every
// `email://`, `draft://`, `citation://`, and `attachment://` href to "" —
// so our custom `a` component receives an empty string, the
// `href.startsWith('email://')` branch is never taken, and the link
// renders as plain bold text instead of an `EmailRefPill`.
//
// These tests render the component to static markup and assert the chip
// is actually present in the output, which is what the user reported as
// missing in the chat panel.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { renderToStaticMarkup } from 'react-dom/server';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { MarkdownContent } from './MarkdownContent';

// `addLog` is the side effect we're guarding: a hallucinated `email://` /
// `draft://` ref must be logged exactly once per mounted bubble, not on every
// re-render (the v0.5.x duplicate-warning bug fired it ~23× during streaming).
const { addLogMock } = vi.hoisted(() => ({ addLogMock: vi.fn() }));
vi.mock('@/stores/logStore', () => ({
  useLogStore: (selector: (s: { addLog: typeof addLogMock }) => unknown) => selector({ addLog: addLogMock }),
}));

describe('MarkdownContent — URI scheme rendering', () => {
  it('renders `[label](email://ID)` as an EmailRefPill when ID is in the allowlist', () => {
    const html = renderToStaticMarkup(
      <MarkdownContent
        content="[Seguimiento](email://19e2598128ca4655)"
        sources={[]}
        accountId="acc-1"
        emailRefAllowlist={['19e2598128ca4655']}
      />,
    );
    // EmailRefPill's title attribute is the most stable anchor for the
    // assertion — it includes the label and is unique to the pill.
    expect(html).toContain('title="Open email: Seguimiento"');
  });

  it('still renders the pill when the link sits inside bold (`**[label](email://X)**`)', () => {
    // This is exactly the shape the LLM emits in the user's failing case
    // ("**[Seguimiento Comité de Buenas Prácticas](email://...)** — de
    // Chema López…"). The bold wrapper must not interfere.
    const html = renderToStaticMarkup(
      <MarkdownContent
        content="**[Seguimiento Comité de Buenas Prácticas](email://19e2598128ca4655)**"
        sources={[]}
        accountId="acc-1"
        emailRefAllowlist={['19e2598128ca4655']}
      />,
    );
    expect(html).toContain('title="Open email: Seguimiento Comité de Buenas Prácticas"');
  });

  it('drops `email://` links whose id is not in the allowlist (hallucinations)', () => {
    const html = renderToStaticMarkup(
      <MarkdownContent
        content="[Hallucinated](email://nope-not-real)"
        sources={[]}
        accountId="acc-1"
        emailRefAllowlist={['only-this-one']}
      />,
    );
    expect(html).not.toContain('email://nope-not-real');
    expect(html).not.toContain('title="Open email');
    // Label survives as plain text so the user still sees what the LLM wrote.
    expect(html).toContain('Hallucinated');
  });

  it('renders `[label](draft://DRAFT_ID)` as a DraftRefPill when in the allowlist', () => {
    const html = renderToStaticMarkup(
      <MarkdownContent
        content="Draft saved: [Re: Q3 plan](draft://draft-abc)"
        sources={[]}
        accountId="acc-1"
        draftRefAllowlist={['draft-abc']}
      />,
    );
    expect(html).toContain('title="Re-open draft: Re: Q3 plan"');
  });

  it('keeps the pill inline with the list marker for loose numbered lists', () => {
    // Real failing case from the chat panel: the LLM emits a numbered
    // list with blank-line separators, which remark renders as "loose"
    // — wrapping every item's content in `<p>`. With a block-level `<p>`,
    // the marker ("1.") lands on one line and the pill on the next.
    // The fix should keep the `<p>` rendered inline so the chip flows
    // next to the marker the way prose does.
    const content =
      '1. **[Seguimiento](email://eml-1)** — de Chema López — 2026-05-14\n   Snippet: "foo"\n\n2. **[Otro](email://eml-2)** — de Chema López';
    const html = renderToStaticMarkup(
      <MarkdownContent content={content} sources={[]} accountId="acc-1" emailRefAllowlist={['eml-1', 'eml-2']} />,
    );
    // Both pills must render (regression-safety against the urlTransform
    // strip getting reintroduced).
    expect(html).toContain('title="Open email: Seguimiento"');
    expect(html).toContain('title="Open email: Otro"');
    // Structural assertion: the loose-list `<p>` wrapper must be unwrapped
    // so the marker and the chip share one line. A `<p>` directly inside
    // `<li>` would re-introduce the broken layout from the bug report.
    const liBlocks = Array.from(html.matchAll(/<li[^>]*>([\s\S]*?)<\/li>/g)).map((m) => m[1]);
    expect(liBlocks.length).toBeGreaterThan(0);
    for (const block of liBlocks) {
      expect(block).not.toMatch(/<p[\s>]/);
    }
  });
});

describe('MarkdownContent — hallucination warning de-dup', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    addLogMock.mockClear();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => {
      root.unmount();
    });
    container.remove();
  });

  it('logs a dropped hallucinated draft ref only once across re-renders', () => {
    // The model emitted `draft://d-1`, an id never produced by a tool call,
    // so the chip is dropped. Streaming re-renders the bubble many times; the
    // warning must still fire exactly once, not once per render.
    // Fresh element each render (new props object) so React actually
    // re-reconciles — mirroring how streaming pushes new content each tick.
    const render = () =>
      act(() => {
        root.render(
          <MarkdownContent
            content="Here is your [draft](draft://d-1)"
            sources={[]}
            accountId="acc-1"
            draftRefAllowlist={[]}
          />,
        );
      });
    render();
    render();
    render();

    expect(addLogMock).toHaveBeenCalledTimes(1);
    expect(addLogMock).toHaveBeenCalledWith('warn', 'chat', expect.stringContaining('Dropping draft://d-1'));
  });

  it('logs distinct hallucinated refs once each, even when one id repeats', () => {
    const el = (
      <MarkdownContent
        content="[a](email://x-1) then [a-again](email://x-1) and [b](draft://y-2)"
        sources={[]}
        accountId="acc-1"
        emailRefAllowlist={[]}
        draftRefAllowlist={[]}
      />
    );
    act(() => {
      root.render(el);
    });
    act(() => {
      root.render(el);
    });

    // Two distinct messages (one email id appearing twice collapses to one).
    expect(addLogMock).toHaveBeenCalledTimes(2);
    expect(addLogMock).toHaveBeenCalledWith('warn', 'chat', expect.stringContaining('Dropping email://x-1'));
    expect(addLogMock).toHaveBeenCalledWith('warn', 'chat', expect.stringContaining('Dropping draft://y-2'));
  });
});
