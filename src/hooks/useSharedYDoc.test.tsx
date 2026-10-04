// The editor's Y.Doc and the backend's copy: loaded once, local edits saved in
// batches, other people's changes pulled when the store reports them, and
// pending changes mailed when a shared document is closed.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import * as Y from 'yjs';

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
}));
vi.mock('@/lib/api', () => ({
  getSharedDocState: vi.fn(),
  getSharedDocDiff: vi.fn(),
  applySharedDocUpdate: vi.fn(() => Promise.resolve()),
  flushSharedDoc: vi.fn(() => Promise.resolve(true)),
}));

import * as api from '@/lib/api';
import { bytesFromBase64, bytesToBase64 } from '@/lib/yjsBytes';
import { useSharedDocsStore } from '@/stores/sharedDocsStore';
import { useSharedYDoc } from './useSharedYDoc';

const mocked = vi.mocked(api);

/** A peer's document holding `text` in the `body` text. */
function peerWith(text: string) {
  const peer = new Y.Doc();
  peer.getText('body').insert(0, text);
  return peer;
}

let latest: Y.Doc | null = null;

function Harness({ mailOnClose }: { mailOnClose: boolean }) {
  const { doc } = useSharedYDoc({ accountId: 'acc-1', docId: 'd1', mailOnClose });
  latest = doc;
  return null;
}

describe('useSharedYDoc', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    vi.useFakeTimers();
    vi.clearAllMocks();
    latest = null;
    useSharedDocsStore.setState({ changes: {} });
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    vi.useRealTimers();
    container.remove();
  });

  async function mount(stored: Y.Doc, mailOnClose = false) {
    mocked.getSharedDocState.mockResolvedValue(bytesToBase64(Y.encodeStateAsUpdate(stored)));
    await act(async () => {
      root.render(<Harness mailOnClose={mailOnClose} />);
    });
  }

  it('loads the stored document and saves local edits as one batched update', async () => {
    const stored = peerWith('Hello');
    await mount(stored);
    expect(latest?.getText('body').toString()).toBe('Hello');

    act(() => {
      latest?.getText('body').insert(5, ' there');
      latest?.getText('body').insert(11, '!');
    });
    expect(mocked.applySharedDocUpdate).not.toHaveBeenCalled();
    await act(async () => {
      vi.advanceTimersByTime(500);
    });

    expect(mocked.applySharedDocUpdate).toHaveBeenCalledTimes(1);
    const [, , update] = mocked.applySharedDocUpdate.mock.calls[0];
    Y.applyUpdate(stored, bytesFromBase64(update));
    expect(stored.getText('body').toString()).toBe('Hello there!');
    act(() => root.unmount());
  });

  it('pulls what arrived from other people without saving it back', async () => {
    const stored = peerWith('Hello');
    await mount(stored);
    const other = new Y.Doc();
    Y.applyUpdate(other, Y.encodeStateAsUpdate(stored));
    other.getText('body').insert(5, ' world');
    mocked.getSharedDocDiff.mockImplementation(async (_a, _d, sv) =>
      bytesToBase64(Y.encodeStateAsUpdate(other, bytesFromBase64(sv))),
    );

    await act(async () => {
      useSharedDocsStore.setState({ changes: { d1: 1 } });
    });
    await act(async () => {
      vi.advanceTimersByTime(500);
    });

    expect(latest?.getText('body').toString()).toBe('Hello world');
    expect(mocked.applySharedDocUpdate).not.toHaveBeenCalled();
    act(() => root.unmount());
  });

  it('saves pending edits and mails them when a shared document closes', async () => {
    await mount(peerWith('Hi'), true);
    act(() => {
      latest?.getText('body').insert(2, '!');
    });
    await act(async () => {
      root.unmount();
    });

    expect(mocked.applySharedDocUpdate).toHaveBeenCalledTimes(1);
    expect(mocked.flushSharedDoc).toHaveBeenCalledWith('acc-1', 'd1');
  });

  it('does not mail anything when an unshared document closes', async () => {
    await mount(peerWith('Hi'), false);
    await act(async () => {
      root.unmount();
    });
    expect(mocked.flushSharedDoc).not.toHaveBeenCalled();
  });
});
