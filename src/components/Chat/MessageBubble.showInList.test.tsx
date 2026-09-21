// The "show in list" button turns the emails an answer cites into an `id:`
// search, so the user can see them together in the filtered email list.

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
    content: 'Two invoices arrived [1] [2].',
    sources: [
      {
        citationNumber: 1,
        emailId: 'e1',
        relevanceScore: 1,
        subject: 'a',
        sender: 'x',
        senderEmail: 'x@ex.com',
        timestamp: 0,
      },
      {
        citationNumber: 2,
        emailId: 'e2',
        relevanceScore: 1,
        subject: 'b',
        sender: 'x',
        senderEmail: 'x@ex.com',
        timestamp: 0,
      },
    ],
    model: null,
    tokenCount: null,
    latencyMs: null,
    createdAt: 0,
    ...overrides,
  };
}

function showInListButton() {
  return container.querySelector<HTMLButtonElement>('[data-testid="chat-show-in-list"]');
}

function render(message: ChatMessage, isStreaming: boolean, onShowEmailsInList?: (q: string) => void) {
  act(() => {
    root.render(
      <MessageBubble
        message={message}
        isStreaming={isStreaming}
        accountId="acc-1"
        onShowEmailsInList={onShowEmailsInList}
      />,
    );
  });
}

describe('MessageBubble — show referenced emails in list', () => {
  it('searches the list for the cited emails when clicked', () => {
    const onShow = vi.fn();
    render(assistantMessage(), false, onShow);
    act(() => showInListButton()?.click());
    expect(onShow).toHaveBeenCalledWith('id:e1 id:e2');
  });

  it('is hidden when the answer references no emails', () => {
    render(assistantMessage({ sources: [] }), false, vi.fn());
    expect(showInListButton()).toBeNull();
  });

  it('is hidden while the answer is still streaming', () => {
    render(assistantMessage(), true, vi.fn());
    expect(showInListButton()).toBeNull();
  });
});
