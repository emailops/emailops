// Quitting while research runs asks first: the run would be lost.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import * as api from '@/lib/api';
import { useChatStore } from '@/stores/chatStore';
import { ResearchExitDialog } from './ResearchExitDialog';

vi.mock('@/lib/api');

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  vi.clearAllMocks();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  useChatStore.setState({
    researchExitRequested: true,
    runningResearch: {
      messageId: 'm1',
      conversationId: 'c1',
      stage: 'reading',
      batch: 3,
      batches: 10,
      emailsRead: 30,
      emailsTotal: 100,
    },
  });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function button(testId: string): HTMLButtonElement {
  const el = document.querySelector<HTMLButtonElement>(`[data-testid="${testId}"]`);
  if (!el) throw new Error(`${testId} not rendered`);
  return el;
}

describe('ResearchExitDialog', () => {
  it('names the research in progress and its step', () => {
    act(() => root.render(<ResearchExitDialog />));
    expect(document.body.textContent).toContain('research.exit.body');
    expect(document.body.textContent).toContain('processing.research.reading');
  });

  it('keeps the app open on "keep researching"', () => {
    act(() => root.render(<ResearchExitDialog />));
    act(() => button('research-exit-keep').click());
    expect(useChatStore.getState().researchExitRequested).toBe(false);
    expect(api.confirmExit).not.toHaveBeenCalled();
  });

  it('quits on "quit anyway"', () => {
    vi.mocked(api.confirmExit).mockResolvedValue(undefined);
    act(() => root.render(<ResearchExitDialog />));
    act(() => button('research-exit-quit').click());
    expect(api.confirmExit).toHaveBeenCalled();
  });

  it('renders nothing when no quit is pending', () => {
    useChatStore.setState({ researchExitRequested: false });
    act(() => root.render(<ResearchExitDialog />));
    expect(document.querySelector('[data-testid="research-exit-quit"]')).toBeNull();
  });
});
