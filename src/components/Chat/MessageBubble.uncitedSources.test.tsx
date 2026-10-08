// An answer that cites no email inline (qwen3.5-4b-q8 summarised a thread
// without a single link) still shows the emails it rests on as chips under
// the text, instead of only behind the collapsed "N sources used" toggle.

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

function source(n: number, emailId: string, subject: string) {
  return {
    citationNumber: n,
    emailId,
    relevanceScore: 1,
    subject,
    sender: 'Vendor',
    senderEmail: 'vendor@example.com',
    timestamp: 0,
  };
}

function assistantMessage(content: string): ChatMessage {
  return {
    id: 'm1',
    conversationId: 'c1',
    role: 'assistant',
    content,
    sources: [source(1, 'acc::1', 'Pilot proposal'), source(2, 'acc::2', 'Re: Pilot proposal')],
    referencedEmailIds: ['acc::1', 'acc::2'],
    model: null,
    tokenCount: null,
    latencyMs: null,
    createdAt: 0,
  };
}

function render(message: ChatMessage, isStreaming = false) {
  act(() => {
    root.render(<MessageBubble message={message} isStreaming={isStreaming} accountId="acc-1" />);
  });
}

const sourceChips = () => container.querySelector('[data-testid="chat-uncited-sources"]');

describe('MessageBubble — sources of an answer that cites nothing', () => {
  it('shows the sources as chips when the answer has no inline link', () => {
    render(assistantMessage('The vendor proposed a pilot and later asked to talk today.'));
    const row = sourceChips();
    expect(row).not.toBeNull();
    expect(row?.querySelector('[title="Open email: Pilot proposal"]')).not.toBeNull();
    expect(row?.querySelector('[title="Open email: Re: Pilot proposal"]')).not.toBeNull();
  });

  it('adds no row when the answer already links an email', () => {
    render(assistantMessage('The vendor proposed a pilot [proposal](email://acc::1).'));
    expect(sourceChips()).toBeNull();
  });

  it('waits for the answer to finish streaming', () => {
    render(assistantMessage('The vendor proposed'), true);
    expect(sourceChips()).toBeNull();
  });
});
