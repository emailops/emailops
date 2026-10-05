// The history panel lists who changed the document and when, newest first,
// and hands the chosen version up so the pane can show it read-only.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, vars?: Record<string, unknown>) => (vars ? `${key} ${JSON.stringify(vars)}` : key),
    i18n: { language: 'en' },
  }),
}));
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(() => Promise.resolve(() => {})) }));
vi.mock('@/lib/api', () => ({
  listSharedDocVersions: vi.fn(() =>
    Promise.resolve([
      { id: 3, author: 'ana@example.com', origin: 'remote', createdAt: 1_800_000_300 },
      { id: 1, author: 'me@example.com', origin: 'local', createdAt: 1_800_000_000 },
    ]),
  ),
}));

import { HistoryPanel } from './HistoryPanel';

describe('HistoryPanel', () => {
  let container: HTMLDivElement;
  let root: Root;
  const onSelect = vi.fn();

  beforeEach(() => {
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    vi.clearAllMocks();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
  });

  async function mount(selectedId: number | null = null) {
    await act(async () => {
      root.render(
        <HistoryPanel
          accountId="acc-1"
          docId="d1"
          me="me@example.com"
          selectedId={selectedId}
          onSelect={onSelect}
          onClose={() => {}}
        />,
      );
    });
  }

  it('lists each version with its author, "you" for this account', async () => {
    await mount();
    const rows = [...container.querySelectorAll('[data-testid^="shared-doc-version-"]')].map((e) => e.textContent);
    expect(rows).toHaveLength(2);
    expect(rows[0]).toContain('ana@example.com');
    expect(rows[0]).toContain('documents:history.remote');
    expect(rows[1]).toContain('documents:history.you');
  });

  it('selects a version, and selecting it again goes back to the current one', async () => {
    await mount();
    await act(async () => (container.querySelector('[data-testid="shared-doc-version-3"]') as HTMLElement).click());
    expect(onSelect).toHaveBeenLastCalledWith(expect.objectContaining({ id: 3 }));
    await mount(3);
    await act(async () => (container.querySelector('[data-testid="shared-doc-version-3"]') as HTMLElement).click());
    expect(onSelect).toHaveBeenLastCalledWith(null);
  });

  it('lists the cells two people changed at once, and whether that was settled', async () => {
    await act(async () => {
      root.render(
        <HistoryPanel
          accountId="acc-1"
          docId="d1"
          me="me@example.com"
          selectedId={null}
          onSelect={onSelect}
          onClose={() => {}}
          conflicts={[
            { id: '1:4', rowId: 'r', colId: 'c', row: 1, col: 1, lost: '120', kept: '130', resolved: false },
            { id: '2:9', rowId: 'r', colId: 'd', row: 1, col: 2, lost: 'x', kept: 'y', resolved: true },
          ]}
        />,
      );
    });
    const rows = [...container.querySelectorAll('[data-testid="shared-doc-conflict"]')].map((e) => e.textContent);
    expect(rows).toHaveLength(2);
    expect(rows[0]).toContain('B2');
    expect(rows[0]).toContain('documents:conflicts.pending');
    expect(rows[1]).toContain('documents:conflicts.resolved');
  });
});
