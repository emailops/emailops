// The app-wide listener for background rule scans: progress and the final
// outcome land in the attachment store whether or not the rules modal is open.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const events = vi.hoisted(() => ({ handlers: {} as Record<string, (e: { payload: unknown }) => void> }));
vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(async (name: string, handler: (e: { payload: unknown }) => void) => {
    events.handlers[name] = handler;
    return () => {
      delete events.handlers[name];
    };
  }),
}));

const logs = vi.hoisted(() => ({ addLog: vi.fn() }));
vi.mock('@/stores/logStore', () => ({
  useLogStore: (selector: (s: { addLog: typeof logs.addLog }) => unknown) => selector({ addLog: logs.addLog }),
}));

vi.mock('@/lib/api', () => ({
  countAttachmentsForRule: vi.fn(async () => 7),
}));

import * as api from '@/lib/api';
import { useAttachmentStore } from '@/stores/attachmentStore';
import { useRuleApplyEvents } from './useRuleApplyEvents';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const onApplied = vi.fn();

function Probe() {
  useRuleApplyEvents(onApplied);
  return null;
}

let container: HTMLDivElement;
let root: Root;

async function emit(name: string, payload: unknown) {
  await act(async () => {
    events.handlers[name]?.({ payload });
  });
}

beforeEach(async () => {
  vi.clearAllMocks();
  useAttachmentStore.setState({ ruleApplies: {} });
  container = document.createElement('div');
  root = createRoot(container);
  await act(async () => root.render(<Probe />));
});

afterEach(() => {
  act(() => root.unmount());
});

describe('useRuleApplyEvents', () => {
  it('feeds progress of the running scan into the store', async () => {
    const runId = useAttachmentStore.getState().beginRuleApply('r1');

    await emit('attachment-rule-apply-progress', { ruleId: 'r1', runId, processed: 1, total: 4, saved: 1 });

    expect(useAttachmentStore.getState().ruleApplies.r1).toMatchObject({ processed: 1, total: 4 });
  });

  it('a finished scan records every attachment of the rule and refreshes the list', async () => {
    const runId = useAttachmentStore.getState().beginRuleApply('r1');

    await emit('attachment-rule-apply-finished', {
      ruleId: 'r1',
      accountId: 'acct-1',
      runId,
      status: 'done',
      saved: 2,
      error: null,
    });

    expect(api.countAttachmentsForRule).toHaveBeenCalledWith('acct-1', 'r1');
    expect(useAttachmentStore.getState().ruleApplies.r1).toMatchObject({ status: 'done', saved: 2, collected: 7 });
    expect(onApplied).toHaveBeenCalled();
  });

  it('a failed scan is shown as failed and logged with its error', async () => {
    const runId = useAttachmentStore.getState().beginRuleApply('r1');

    await emit('attachment-rule-apply-finished', {
      ruleId: 'r1',
      accountId: 'acct-1',
      runId,
      status: 'failed',
      saved: 0,
      error: { code: 'sync', params: {}, message: 'provider offline' },
    });

    expect(useAttachmentStore.getState().ruleApplies.r1.status).toBe('failed');
    expect(logs.addLog).toHaveBeenCalledWith('error', 'attachments', expect.stringContaining('provider offline'));
  });

  it('a scan cancelled by a newer one is not reported as failed', async () => {
    const runId = useAttachmentStore.getState().beginRuleApply('r1');

    await emit('attachment-rule-apply-finished', {
      ruleId: 'r1',
      accountId: 'acct-1',
      runId,
      status: 'cancelled',
      saved: 0,
      error: null,
    });

    expect(useAttachmentStore.getState().ruleApplies.r1).toBeUndefined();
    expect(logs.addLog).not.toHaveBeenCalledWith('error', expect.anything(), expect.anything());
  });

  it('ignores malformed payloads', async () => {
    useAttachmentStore.getState().beginRuleApply('r1');

    await emit('attachment-rule-apply-finished', { ruleId: 'r1', status: 'exploded' });

    expect(useAttachmentStore.getState().ruleApplies.r1.status).toBe('running');
  });

  it('stops listening when unmounted', async () => {
    act(() => root.unmount());
    root = createRoot(container);
    await act(async () => {});

    expect(events.handlers['attachment-rule-apply-finished']).toBeUndefined();
    await act(async () => root.render(<Probe />));
  });

  it('a finished scan whose count cannot be read still finishes, and logs why', async () => {
    vi.mocked(api.countAttachmentsForRule).mockRejectedValueOnce(new Error('db locked'));
    const runId = useAttachmentStore.getState().beginRuleApply('r1');

    await emit('attachment-rule-apply-finished', {
      ruleId: 'r1',
      accountId: 'acct-1',
      runId,
      status: 'done',
      saved: 2,
      error: null,
    });

    expect(useAttachmentStore.getState().ruleApplies.r1).toMatchObject({ status: 'done', saved: 2 });
    expect(logs.addLog).toHaveBeenCalledWith('error', 'attachments', expect.stringContaining('db locked'));
  });

  it('ignores malformed progress', async () => {
    useAttachmentStore.getState().beginRuleApply('r1');

    for (const payload of [null, 'r1', { ruleId: 'r1', processed: '1' }]) {
      await emit('attachment-rule-apply-progress', payload);
    }

    expect(useAttachmentStore.getState().ruleApplies.r1.processed).toBe(0);
  });

  it('a failed scan without an error payload is still reported', async () => {
    const runId = useAttachmentStore.getState().beginRuleApply('r1');

    await emit('attachment-rule-apply-finished', {
      ruleId: 'r1',
      accountId: 'acct-1',
      runId,
      status: 'failed',
      saved: 0,
      error: null,
    });

    expect(useAttachmentStore.getState().ruleApplies.r1.status).toBe('failed');
    expect(logs.addLog).toHaveBeenCalledWith('error', 'attachments', expect.stringContaining('unknown error'));
  });
});
