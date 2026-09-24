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

describe('latest matches while a research reads', () => {
  it('lists the latest matches so the user can judge the run early', () => {
    const onOpenEmail = vi.fn();
    useChatStore.setState({
      researchStopping: false,
      researchProgress: {
        messageId: 'm1',
        conversationId: 'c1',
        stage: 'reading',
        batch: 2,
        batches: 10,
        emailsRead: 20,
        emailsTotal: 100,
        matches: 12,
        recent: [
          {
            emailId: 'e1',
            date: '2025-03-12',
            subject: 'Quote for the dashboard',
            finding: 'Sent a quote of 4,000 EUR',
            emails: 3,
          },
          { emailId: 'e2', date: '2025-04-02', subject: 'Re: pricing', finding: 'Follow-up on the quote', emails: 1 },
        ],
      },
    });
    act(() =>
      root.render(
        <MessageBubble message={message} isStreaming phase="researching" accountId="acc1" onOpenEmail={onOpenEmail} />,
      ),
    );
    const list = container.querySelector('[data-testid="research-recent"]');
    expect(list?.textContent).toContain('research.matchesSoFar');
    expect(list?.textContent).toContain('Quote for the dashboard');
    expect(list?.textContent).toContain('Sent a quote of 4,000 EUR');
    // Each match is the chat's email chip, which opens the email.
    const chips = list?.querySelectorAll('button') ?? [];
    expect(chips.length).toBe(2);
    expect(chips[0].textContent).toContain('Quote for the dashboard');
    // One entry per conversation, saying how many of its emails matched; the
    // finding gets its own line instead of being cut to "—…".
    expect(list?.textContent).toContain('research.threadEmails');
    const finding = list?.querySelector('[data-testid="research-recent-finding"]');
    expect(finding?.textContent).toBe('Sent a quote of 4,000 EUR');
    expect(finding?.className).not.toContain('truncate');
  });
});
