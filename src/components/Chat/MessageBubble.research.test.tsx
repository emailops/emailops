// A research turn can run for minutes or hours: the bubble says where it is,
// how long is left, and lets the user stop it and get the report so far.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useChatStore } from '@/stores/chatStore';
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

const message: ChatMessage = {
  id: 'm1',
  conversationId: 'c1',
  role: 'assistant',
  content: '',
  sources: [],
  model: null,
  tokenCount: null,
  latencyMs: null,
  createdAt: 0,
};

function render() {
  act(() => root.render(<MessageBubble message={message} isStreaming phase="researching" accountId="acc1" />));
}

describe('research status in the bubble', () => {
  it('shows the batch progress, the time left and a stop button', () => {
    const stopResearch = vi.fn(async () => {});
    useChatStore.setState({
      researchProgress: {
        messageId: 'm1',
        conversationId: 'c1',
        stage: 'reading',
        batch: 3,
        batches: 10,
        emailsRead: 30,
        emailsTotal: 100,
      },
      researchStartedAt: Date.now() - 60_000,
      researchStopping: false,
      stopResearch,
    });
    render();
    expect(container.textContent).toContain('processing.research.reading');
    expect(container.textContent).toContain('processing.research.remaining');
    const stop = container.querySelector<HTMLButtonElement>('[data-testid="research-stop"]');
    expect(stop?.textContent).toContain('research.stop');
    act(() => stop?.click());
    expect(stopResearch).toHaveBeenCalled();
  });

  it('says it is stopping once asked', () => {
    useChatStore.setState({ researchStopping: true });
    render();
    const stop = container.querySelector<HTMLButtonElement>('[data-testid="research-stop"]');
    expect(stop?.disabled).toBe(true);
    expect(stop?.textContent).toContain('research.stopping');
  });
});
