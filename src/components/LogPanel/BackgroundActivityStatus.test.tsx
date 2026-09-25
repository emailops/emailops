// The status bar says which expensive AI work is running and how far along
// it is: a research run first, then lens / memory / task backfills,
// classification, embeddings, junk scoring and model downloads.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useActivityStore } from '@/stores/activityStore';
import { useChatStore } from '@/stores/chatStore';
import { BackgroundActivityStatus } from './BackgroundActivityStatus';

vi.mock('@/lib/api');
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  useChatStore.setState({ runningResearch: null });
  useActivityStore.setState({ activities: [], polling: false });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function render() {
  act(() => root.render(<BackgroundActivityStatus />));
  return container.querySelector('[data-testid="background-activity"]');
}

describe('BackgroundActivityStatus', () => {
  it('is empty when nothing expensive runs', () => {
    expect(render()).toBeNull();
  });

  it('puts a running research first, with its step', () => {
    useChatStore.setState({
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
    useActivityStore.setState({
      activities: [{ key: 'embeddings', kind: 'embeddings', target: 'all', progress: { current: 3, total: 9 } }],
    });
    const el = render();
    expect(el?.textContent).toContain('research.statusBar');
    expect(el?.textContent).toContain('+1');
    expect(el?.getAttribute('title')).toContain('activity.embeddings');
  });

  it('shows a backfill with its progress', () => {
    useActivityStore.setState({
      activities: [
        { key: 'lensBackfill:l1', kind: 'lensBackfill', target: 'l1', progress: { current: 120, total: 400 } },
      ],
    });
    const el = render();
    expect(el?.textContent).toContain('activity.lensBackfill');
    expect(el?.textContent).toContain('120/400');
  });
});
