// Regression test: the chat input must auto-grow with its content instead of
// staying a fixed 2-row box. jsdom does no layout, so scrollHeight is stubbed
// on the element; the assertion is that typing re-plans the inline height and
// that clearing the value (as submit does) shrinks it back.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
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
