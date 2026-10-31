// A note under an answer that stopped at the model's token limit, so a reply
// that ends mid-sentence does not read as complete.

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

function message(answerTruncated: boolean | undefined): ChatMessage {
  return {
    id: 'm1',
    conversationId: 'c1',
    role: 'assistant',
    content: 'The kickoff moved to',
    sources: [],
    model: null,
    tokenCount: null,
    latencyMs: null,
    createdAt: 0,
    trace: {
      route: { mode: 'rag_first', reason: '', matchedKeywords: [], classifier: 'planner' },
      toolCalls: [],
      model: 'qwen',
      totalElapsedMs: 1000,
      steps: [{ type: 'route' }],
      answerTruncated,
    },
  };
}

function render(answerTruncated: boolean | undefined, isStreaming = false) {
  act(() => {
    root.render(<MessageBubble message={message(answerTruncated)} isStreaming={isStreaming} accountId="acc1" />);
  });
}

const notice = () => container.querySelector('[data-testid="chat-truncated-notice"]');

describe('MessageBubble — answer cut off', () => {
  it('tells the user when the answer stopped at the token limit', () => {
    render(true);
    expect(notice()).not.toBeNull();
  });

  it('says nothing about a finished answer, or a trace saved before the flag existed', () => {
    render(false);
    expect(notice()).toBeNull();
    render(undefined);
    expect(notice()).toBeNull();
  });

  it('waits until the answer has finished streaming', () => {
    render(true, true);
    expect(notice()).toBeNull();
  });
});
