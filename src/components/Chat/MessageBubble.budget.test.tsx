// A note under the answer when the context budget cut something the answer
// rests on — and only then: trimming old history is routine and stays silent.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { BudgetTrace, ChatMessage } from '@/types';
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

function message(budget: BudgetTrace | undefined): ChatMessage {
  return {
    id: 'm1',
    conversationId: 'c1',
    role: 'assistant',
    content: 'The newsletter covers three topics.',
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
      budget,
    },
  };
}

function render(budget: BudgetTrace | undefined, props: { isStreaming?: boolean; onOpenAiSettings?: () => void } = {}) {
  act(() => {
    root.render(
      <MessageBubble
        message={message(budget)}
        isStreaming={props.isStreaming ?? false}
        accountId="acc1"
        onOpenAiSettings={props.onOpenAiSettings}
      />,
    );
  });
}

const notice = () => container.querySelector('[data-testid="chat-budget-notice"]');
const settingsButton = () => container.querySelector<HTMLButtonElement>('[data-testid="chat-budget-settings"]');

const cut: BudgetTrace = {
  nCtx: 8192,
  replyReserve: 1024,
  estimatedPromptTokens: 6000,
  cuts: [{ kind: 'sourceExcerpts', charsPerEmail: 1200 }],
  fits: true,
};

describe('MessageBubble — context budget notice', () => {
  it('says nothing when the turn was not cut', () => {
    render(undefined);
    expect(notice()).toBeNull();
  });

  it('says nothing when only older history was cut', () => {
    render({ ...cut, cuts: [{ kind: 'historyTurns', messages: 2 }] });
    expect(notice()).toBeNull();
  });

  it('tells the user when this turn’s emails were shortened', () => {
    render(cut);
    expect(notice()?.getAttribute('data-variant')).toBe('cut');
  });

  it('uses the overflow wording when the prompt did not fit at all', () => {
    render({ ...cut, cuts: [], fits: false });
    expect(notice()?.getAttribute('data-variant')).toBe('overflow');
  });

  it('waits until the answer has finished streaming', () => {
    render(cut, { isStreaming: true });
    expect(notice()).toBeNull();
  });

  it('opens the AI settings from the note when a handler is wired', () => {
    const onOpenAiSettings = vi.fn();
    render(cut, { onOpenAiSettings });
    act(() => settingsButton()?.click());
    expect(onOpenAiSettings).toHaveBeenCalledTimes(1);
  });

  it('offers no settings button without a handler', () => {
    render(cut);
    expect(settingsButton()).toBeNull();
  });
});
