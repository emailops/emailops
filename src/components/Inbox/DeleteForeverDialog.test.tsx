// The "Delete forever" / "Empty Trash" dialog: count, confirmation, progress,
// Cancel and the final summary, driven by the backend's events.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

const { handlers } = vi.hoisted(() => ({
  handlers: new Map<string, (event: { payload: unknown }) => void>(),
}));

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, handler: (event: { payload: unknown }) => void) => {
    handlers.set(name, handler);
    return () => handlers.delete(name);
  }),
}));

vi.mock('@/lib/api', () => ({
  getEmailCount: vi.fn(async () => 3),
  deleteForever: vi.fn(async () => undefined),
  cancelDeleteForever: vi.fn(async () => undefined),
  currentPlatform: () => 'macos',
}));

import { initI18n } from '@/i18n';
import * as api from '@/lib/api';
import type { DeleteForeverFinished } from '@/lib/deleteForever';
import { DeleteForeverDialog } from './DeleteForeverDialog';

let container: HTMLDivElement;
let root: Root;
const onClose = vi.fn();

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  vi.clearAllMocks();
  handlers.clear();
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

const q = (id: string) => document.querySelector<HTMLElement>(`[data-testid="${id}"]`);

async function render(emailIds: string[] | null, folder: 'trash' | 'spam' = 'trash') {
  await act(async () => {
    root.render(<DeleteForeverDialog request={{ accountId: 'imap-1', folder, emailIds }} onClose={onClose} />);
  });
}

async function click(id: string) {
  const el = q(id);
  if (!el) throw new Error(`${id} not rendered`);
  await act(async () => {
    el.dispatchEvent(new MouseEvent('click', { bubbles: true }));
  });
}

function runId(): string {
  const call = vi.mocked(api.deleteForever).mock.calls[0];
  if (!call) throw new Error('deleteForever was not called');
  return call[3];
}

async function emit(name: string, payload: unknown) {
  const handler = handlers.get(name);
  if (!handler) throw new Error(`no listener for ${name}`);
  await act(async () => handler({ payload }));
}

const finished = (extra: Partial<DeleteForeverFinished>): DeleteForeverFinished => ({
  runId: runId(),
  accountId: 'imap-1',
  folder: 'trash',
  total: 3,
  removed: 3,
  failed: [],
  notAttempted: 0,
  cancelled: false,
  error: null,
  ...extra,
});

describe('DeleteForeverDialog', () => {
  it('empties the folder after the count is confirmed, then shows progress and a summary', async () => {
    await render(null);

    expect(api.getEmailCount).toHaveBeenCalledWith('imap-1', 'deleted');
    expect(q('delete-forever-confirm-text')?.textContent).toContain('3 messages in Trash will be deleted forever');
    expect(api.deleteForever).not.toHaveBeenCalled();

    await click('delete-forever-confirm');
    expect(api.deleteForever).toHaveBeenCalledWith('imap-1', 'trash', null, runId());

    await emit('delete-forever-progress', {
      runId: runId(),
      accountId: 'imap-1',
      folder: 'trash',
      processed: 2,
      total: 3,
      removed: 2,
      failed: 0,
    });
    expect(q('delete-forever-progress')?.getAttribute('aria-valuenow')).toBe('2');

    await emit(
      'delete-forever-finished',
      finished({
        removed: 2,
        failed: [{ emailId: 'm-3', error: { code: 'sync', params: { detail: 'kept' }, message: 'kept' } }],
      }),
    );
    const summary = q('delete-forever-summary')?.textContent ?? '';
    expect(summary).toContain('2 messages deleted forever.');
    expect(summary).toContain('1 message could not be deleted and is still there:');
    expect(summary).toContain('kept');
  });

  it('deletes the selected messages without counting the folder', async () => {
    await render(['m-1', 'm-2'], 'spam');

    expect(api.getEmailCount).not.toHaveBeenCalled();
    expect(q('delete-forever-confirm-text')?.textContent).toContain('2 messages in Spam will be deleted forever');

    await click('delete-forever-confirm');
    expect(api.deleteForever).toHaveBeenCalledWith('imap-1', 'spam', ['m-1', 'm-2'], runId());
  });

  it('cancel asks the run to stop and the summary says what was left as it was', async () => {
    await render(null);
    await click('delete-forever-confirm');

    await click('delete-forever-cancel-run');
    expect(api.cancelDeleteForever).toHaveBeenCalledWith(runId());

    await emit('delete-forever-finished', finished({ removed: 1, notAttempted: 2, cancelled: true }));
    const summary = q('delete-forever-summary')?.textContent ?? '';
    expect(summary).toContain('1 message deleted forever.');
    expect(summary).toContain('Cancelled: 2 messages were left as they were.');
  });

  it('cannot be closed while the run is going', async () => {
    await render(null);
    await click('delete-forever-confirm');

    await act(async () => {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });

    expect(onClose).not.toHaveBeenCalled();
    expect(q('delete-forever-cancel-run')).not.toBeNull();
  });

  it('shows why a run stopped above the summary', async () => {
    await render(null);
    await click('delete-forever-confirm');

    await emit(
      'delete-forever-finished',
      finished({
        removed: 0,
        notAttempted: 3,
        error: { code: 'expunge_not_safe', params: {}, message: 'unsafe' },
      }),
    );

    const error = q('delete-forever-error');
    expect(error).not.toBeNull();
    const summary = q('delete-forever-summary');
    expect(summary).not.toBeNull();
    // Pinned above what may scroll.
    expect(error?.compareDocumentPosition(summary as Node) ?? 0).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
  });

  it('reports a run the backend refused to start', async () => {
    vi.mocked(api.deleteForever).mockRejectedValueOnce({ code: 'invalid_input', params: {}, message: 'nope' });
    await render(['m-1']);

    await click('delete-forever-confirm');

    expect(q('delete-forever-error')).not.toBeNull();
  });
});
