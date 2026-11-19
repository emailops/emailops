// Settings → AI shows how far the email index is from done and how fast it
// is being filled, from `get_indexing_status`, refreshed while it is open.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, vars?: Record<string, unknown>) => (vars ? `${key} ${JSON.stringify(vars)}` : key),
    i18n: { language: 'en' },
  }),
}));

const api = vi.hoisted(() => ({
  getIndexingStatus: vi.fn(),
}));
vi.mock('@/lib/api', () => api);

import { IndexingStatus } from './IndexingStatus';

describe('IndexingStatus', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    vi.clearAllMocks();
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
    vi.useRealTimers();
  });

  async function mount(status: { backlog: number; messagesPerMinute: number | null }) {
    api.getIndexingStatus.mockResolvedValue(status);
    await act(async () => {
      root.render(<IndexingStatus />);
    });
  }

  const text = () => container.textContent ?? '';

  it('shows the backlog and the emails indexed per minute', async () => {
    await mount({ backlog: 1234, messagesPerMinute: 42.4 });

    expect(text()).toContain('settings:ai.indexing.backlog {"count":1234}');
    expect(text()).toContain('settings:ai.indexing.rate {"rate":"42"}');
  });

  it('keeps one decimal for a slow rate, so it never reads as zero', async () => {
    await mount({ backlog: 10, messagesPerMinute: 0.5 });

    expect(text()).toContain('settings:ai.indexing.rate {"rate":"0.5"}');
  });

  it('says the index is up to date when nothing is waiting', async () => {
    await mount({ backlog: 0, messagesPerMinute: null });

    expect(text()).toContain('settings:ai.indexing.upToDate');
    expect(text()).not.toContain('settings:ai.indexing.backlog');
  });

  // No pace is reported in a run's first seconds either, so its absence must
  // not be shown as "not indexing".
  it('shows only the backlog while no pace is measured', async () => {
    await mount({ backlog: 5, messagesPerMinute: null });

    expect(text()).toContain('settings:ai.indexing.backlog {"count":5}');
    expect(text()).not.toContain('settings:ai.indexing.rate');
    expect(text()).not.toContain('settings:ai.indexing.idle');
  });

  it('refreshes while it is open', async () => {
    vi.useFakeTimers();
    await mount({ backlog: 5, messagesPerMinute: null });
    expect(api.getIndexingStatus).toHaveBeenCalledTimes(1);

    api.getIndexingStatus.mockResolvedValue({ backlog: 3, messagesPerMinute: 12 });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5000);
    });

    expect(api.getIndexingStatus).toHaveBeenCalledTimes(2);
    expect(text()).toContain('settings:ai.indexing.backlog {"count":3}');
    expect(text()).toContain('settings:ai.indexing.rate {"rate":"12"}');
  });

  it('shows why the status could not be read', async () => {
    api.getIndexingStatus.mockRejectedValue('database is locked');
    await act(async () => {
      root.render(<IndexingStatus />);
    });

    expect(text()).toContain('settings:ai.indexing.loadFailed {"error":"database is locked"}');
  });
});
