// "This answer isn't right" → say why → a corrective turn runs.
//
// Two steps on purpose: a bare thumbs-down gives the retry nothing to work
// with, so the control only reports a rejection once the user has said what
// was wrong.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { ChatMessage } from '@/types';
import { MessageBubble } from './MessageBubble';

vi.mock('@/stores/logStore', () => ({
  useLogStore: (selector: (s: { addLog: () => void }) => unknown) => selector({ addLog: () => {} }),
}));

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

function assistantMessage(overrides: Partial<ChatMessage> = {}): ChatMessage {
  return {
    id: 'm1',
    conversationId: 'c1',
    role: 'assistant',
    content: 'You have 4 invoices from August.',
    sources: [],
    model: null,
    tokenCount: null,
    latencyMs: null,
    createdAt: 0,
    ...overrides,
  };
}

function render(props: {
  message?: ChatMessage;
  isStreaming?: boolean;
  onReject?: (reason: string) => void;
  isRejected?: boolean;
  isSending?: boolean;
}) {
  act(() => {
    root.render(
      <MessageBubble
        message={props.message ?? assistantMessage()}
        isStreaming={props.isStreaming ?? false}
        accountId="acc1"
        onReject={props.onReject}
        isRejected={props.isRejected}
        isSending={props.isSending}
      />,
    );
  });
}

const markWrong = () => container.querySelector<HTMLButtonElement>('[data-testid="chat-mark-wrong"]');
const reasonBox = () => container.querySelector<HTMLTextAreaElement>('[data-testid="chat-wrong-reason"]');
const submit = () => container.querySelector<HTMLButtonElement>('[data-testid="chat-wrong-submit"]');
const rejectedNote = () => container.querySelector('[data-testid="chat-rejected-note"]');

/** Type into the controlled textarea.
 *
 * Assigning `.value` directly is invisible to React — it tracks the last value
 * it wrote and skips the change as a no-op — so go through the native setter
 * the way React's own test utils do. */
function type(text: string) {
  const box = reasonBox();
  if (!box) throw new Error('reason box not rendered');
  const setter = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')?.set;
  act(() => {
    setter?.call(box, text);
    box.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

describe('MessageBubble — marking an answer wrong', () => {
  it('offers the control on a finished assistant answer', () => {
    render({ onReject: vi.fn() });
    expect(markWrong()).not.toBeNull();
  });

  it('does not offer it while the answer is still streaming', () => {
    render({ onReject: vi.fn(), isStreaming: true });
    expect(markWrong()).toBeNull();
  });

  it('does not offer it when no retry handler is wired', () => {
    render({});
    expect(markWrong()).toBeNull();
  });

  it('asks what was wrong instead of rejecting on the first click', () => {
    const onReject = vi.fn();
    render({ onReject });
    act(() => markWrong()?.click());
    expect(onReject).not.toHaveBeenCalled();
    expect(reasonBox()).not.toBeNull();
  });

  it('reports the reason verbatim when submitted', () => {
    const onReject = vi.fn();
    render({ onReject });
    act(() => markWrong()?.click());
    type('esos correos son de septiembre, no de agosto');
    act(() => submit()?.click());
    expect(onReject).toHaveBeenCalledWith('esos correos son de septiembre, no de agosto');
  });

  it('refuses to submit an empty reason', () => {
    const onReject = vi.fn();
    render({ onReject });
    act(() => markWrong()?.click());
    expect(submit()?.disabled).toBe(true);
    act(() => submit()?.click());
    expect(onReject).not.toHaveBeenCalled();
  });

  it('trims the reason before reporting it', () => {
    const onReject = vi.fn();
    render({ onReject });
    act(() => markWrong()?.click());
    type('   las fechas están mal   ');
    act(() => submit()?.click());
    expect(onReject).toHaveBeenCalledWith('las fechas están mal');
  });

  it('will not queue a second turn while one is already in flight', () => {
    const onReject = vi.fn();
    render({ onReject, isSending: true });
    act(() => markWrong()?.click());
    type('wrong');
    act(() => submit()?.click());
    expect(onReject).not.toHaveBeenCalled();
  });

  it('replaces the control with a note once the answer is marked wrong', () => {
    render({ onReject: vi.fn(), isRejected: true });
    expect(markWrong()).toBeNull();
    expect(rejectedNote()).not.toBeNull();
  });

  it('never offers the control on the user own message', () => {
    render({ message: assistantMessage({ role: 'user' }), onReject: vi.fn() });
    expect(markWrong()).toBeNull();
  });
});
