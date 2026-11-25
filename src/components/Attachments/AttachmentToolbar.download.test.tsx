// A bulk download reports what it really copied: the toast counts copied of
// requested, and one Logs entry groups the skipped attachments by reason —
// ids only, never paths or file names.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { BulkDownloadSummary } from '@/types';

const { bulkDownloadAttachments, addLog, addToast } = vi.hoisted(() => ({
  bulkDownloadAttachments: vi.fn(),
  addLog: vi.fn(),
  addToast: vi.fn(),
}));

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, unknown>) => (opts ? `${key}${JSON.stringify(opts)}` : key),
  }),
}));
vi.mock('@/lib/api', () => ({ bulkDownloadAttachments, revealInFinder: vi.fn() }));
vi.mock('@/stores/logStore', () => ({
  useLogStore: (selector: (s: { addLog: typeof addLog }) => unknown) => selector({ addLog }),
}));
vi.mock('@/stores/toastStore', () => ({
  useToastStore: (selector: (s: { addToast: typeof addToast }) => unknown) => selector({ addToast }),
}));

import { AttachmentToolbar } from './AttachmentToolbar';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;
const onClearChecked = vi.fn();

function render(checked: string[], isSelectingAll = false, queryAccountId: string | null = 'acc-1') {
  act(() => {
    root.render(
      <AttachmentToolbar
        queryAccountId={queryAccountId}
        totalCount={Math.max(checked.length, 1)}
        selectedTag={null}
        availableTags={[]}
        checkedCount={checked.length}
        allChecked={false}
        isSelectingAll={isSelectingAll}
        onSetSelectedTag={vi.fn()}
        onToggleCheckAll={vi.fn()}
        onClearChecked={onClearChecked}
        checkedIds={new Set(checked)}
        onOpenRules={vi.fn()}
        suggestionCount={0}
      />,
    );
  });
}

async function download(summary: BulkDownloadSummary, checked: string[], queryAccountId: string | null = 'acc-1') {
  bulkDownloadAttachments.mockResolvedValue(summary);
  render(checked, false, queryAccountId);
  const button = Array.from(container.querySelectorAll('button')).find((b) =>
    b.textContent?.includes('attachments:toolbar.download'),
  );
  await act(async () => {
    button?.click();
  });
}

const ids = (n: number) => Array.from({ length: n }, (_, i) => `att-${i}`);
const DEST = '/Users/someone/Downloads';

beforeEach(() => {
  container = document.createElement('div');
  root = createRoot(container);
  bulkDownloadAttachments.mockReset();
  addLog.mockReset();
  addToast.mockReset();
  onClearChecked.mockReset();
});

afterEach(() => {
  act(() => root.unmount());
});

describe('AttachmentToolbar bulk download', () => {
  it('counts copied of requested and the skipped ones in the toast', async () => {
    await download({ dest: DEST, copied: 9, skipped: [{ attachmentId: 'att-3', reason: 'file_missing' }] }, ids(10));

    expect(addToast).toHaveBeenCalledTimes(1);
    expect(addToast.mock.calls[0][0].message).toBe(
      'attachments:toolbar.downloadedWithSkippedToast{"count":1,"copied":9,"total":10}',
    );
  });

  it('logs one error entry with the count per reason and the skipped ids', async () => {
    await download({ dest: DEST, copied: 9, skipped: [{ attachmentId: 'att-3', reason: 'file_missing' }] }, ids(10));

    expect(addLog).toHaveBeenCalledTimes(1);
    const [level, source, message] = addLog.mock.calls[0];
    expect(level).toBe('error');
    expect(source).toBe('attachments');
    expect(message).toContain('attachments:skipReason.file_missing: 1');
    expect(message).toContain('att-3');
  });

  it('names at most 20 skipped ids in the log', async () => {
    const skipped = ids(25).map((attachmentId) => ({ attachmentId, reason: 'not_found' as const }));
    await download({ dest: DEST, copied: 0, skipped }, ids(25));

    const message: string = addLog.mock.calls[0][2];
    expect(message).toContain('att-19');
    expect(message).not.toContain('att-20');
  });

  it('logs success and leaves skipped ones out of the toast when everything was copied', async () => {
    await download({ dest: DEST, copied: 2, skipped: [] }, ids(2));

    expect(addToast.mock.calls[0][0].message).toBe('attachments:toolbar.downloadedSummaryToast{"count":2,"copied":2}');
    expect(addLog.mock.calls[0][0]).toBe('success');
  });

  it('never puts the destination path in the log', async () => {
    await download({ dest: DEST, copied: 2, skipped: [] }, ids(2));

    expect(addLog.mock.calls[0][2]).not.toContain(DEST);
  });

  it('offers "Show in Finder" only when something was copied', async () => {
    await download({ dest: DEST, copied: 1, skipped: [] }, ids(1));
    expect(addToast.mock.calls[0][0].onAction).toBeTypeOf('function');

    addToast.mockReset();
    await download({ dest: DEST, copied: 0, skipped: [{ attachmentId: 'att-0', reason: 'not_found' }] }, ids(1));
    expect(addToast.mock.calls[0][0].onAction).toBeUndefined();
  });

  it('clears the selection after the download', async () => {
    await download({ dest: DEST, copied: 0, skipped: [{ attachmentId: 'att-0', reason: 'copy_failed' }] }, ids(1));

    expect(onClearChecked).toHaveBeenCalledTimes(1);
  });

  it('disables "select all" while every id of the filter loads', () => {
    render([], true);

    expect(container.querySelector<HTMLInputElement>('input[type="checkbox"]')?.disabled).toBe(true);
  });

  it('in All accounts sends the ids of every account with no account', async () => {
    await download({ dest: DEST, copied: 2, skipped: [] }, ['a1', 'b1'], null);

    expect(bulkDownloadAttachments).toHaveBeenCalledWith(null, ['a1', 'b1']);
  });
});
