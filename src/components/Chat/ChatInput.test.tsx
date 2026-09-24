// Regression test: the chat input must auto-grow with its content instead of
// staying a fixed 2-row box. jsdom does no layout, so scrollHeight is stubbed
// on the element; the assertion is that typing re-plans the inline height and
// that clearing the value (as submit does) shrinks it back.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useChatStore } from '@/stores/chatStore';
import { ChatInput } from './ChatInput';

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function renderInput() {
  act(() => {
    root.render(<ChatInput onSend={() => {}} disabled={false} />);
  });
  const textarea = container.querySelector('textarea');
  if (!textarea) throw new Error('textarea not rendered');
  return textarea;
}

function typeInto(textarea: HTMLTextAreaElement, text: string) {
  // Controlled component: go through the native value setter so React's
  // onChange fires (a plain `.value =` is swallowed by React's value tracker).
  const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set;
  act(() => {
    setter?.call(textarea, text);
    textarea.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

function stubScrollHeight(textarea: HTMLTextAreaElement, px: number) {
  Object.defineProperty(textarea, 'scrollHeight', { configurable: true, value: px });
}

describe('ChatInput auto-grow', () => {
  it('re-plans the inline height when the value changes', () => {
    const textarea = renderInput();
    stubScrollHeight(textarea, 120);
    typeInto(textarea, 'a prompt\nspanning\nseveral\nlines');
    expect(textarea.style.height).toBe('120px');
    expect(textarea.style.overflowY).toBe('hidden');
  });

  it('clamps tall content and scrolls internally', () => {
    const textarea = renderInput();
    stubScrollHeight(textarea, 10_000);
    typeInto(textarea, 'x\n'.repeat(200));
    expect(textarea.style.height).toBe('220px');
    expect(textarea.style.overflowY).toBe('auto');
  });

  it('shrinks back when the value is cleared', () => {
    const textarea = renderInput();
    stubScrollHeight(textarea, 10_000);
    typeInto(textarea, 'x\n'.repeat(200));
    expect(textarea.style.height).toBe('220px');

    stubScrollHeight(textarea, 60);
    typeInto(textarea, '');
    expect(textarea.style.height).toBe('60px');
    expect(textarea.style.overflowY).toBe('hidden');
  });
});

describe('ChatInput research toggle', () => {
  it('arms and disarms research mode for the next message', () => {
    useChatStore.setState({ researchMode: false });
    renderInput();
    const toggle = container.querySelector<HTMLButtonElement>('[data-testid="chat-research-toggle"]');
    if (!toggle) throw new Error('research toggle not rendered');
    expect(toggle.getAttribute('aria-pressed')).toBe('false');

    act(() => toggle.click());
    expect(useChatStore.getState().researchMode).toBe(true);
    expect(toggle.getAttribute('aria-pressed')).toBe('true');

    act(() => toggle.click());
    expect(useChatStore.getState().researchMode).toBe(false);
  });
});

describe('ChatInput research confirmation', () => {
  const estimate = { estimateId: 'est-1', emails: 1240, batches: 124, seconds: 2100, filter: null };

  it('shows the estimate with start and cancel, and blocks the input meanwhile', () => {
    const confirmResearch = vi.fn(async () => {});
    const cancelResearch = vi.fn();
    useChatStore.setState({
      pendingResearch: { content: 'q', opts: {}, status: 'ready', estimate, error: null },
      confirmResearch,
      cancelResearch,
    });
    const textarea = renderInput();
    expect(textarea.disabled).toBe(true);
    const card = container.querySelector('[data-testid="research-confirm"]');
    expect(card?.textContent).toContain('research.estimate');
    act(() => container.querySelector<HTMLButtonElement>('[data-testid="research-start"]')?.click());
    expect(confirmResearch).toHaveBeenCalled();
    act(() => container.querySelector<HTMLButtonElement>('[data-testid="research-cancel"]')?.click());
    expect(cancelResearch).toHaveBeenCalled();
  });

  it('offers no start when nothing matches', () => {
    useChatStore.setState({
      pendingResearch: { content: 'q', opts: {}, status: 'ready', estimate: { ...estimate, emails: 0 }, error: null },
    });
    renderInput();
    expect(container.querySelector('[data-testid="research-confirm"]')?.textContent).toContain('research.estimateNone');
    expect(container.querySelector('[data-testid="research-start"]')).toBeNull();
  });

  it('puts a cancelled question back in the textarea', () => {
    useChatStore.setState({ pendingResearch: null, inputPrefill: null });
    const textarea = renderInput();
    act(() => useChatStore.setState({ inputPrefill: { text: 'themes?', nonce: 1 } }));
    expect(textarea.value).toBe('themes?');
  });
});
