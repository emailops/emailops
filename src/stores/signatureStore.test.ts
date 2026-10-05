// Account signatures: loaded once per account and cached for the composers,
// replaced in the cache when the settings editor saves.

import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({
  getAccountSignature: vi.fn(),
  saveAccountSignature: vi.fn(),
}));

import * as api from '@/lib/api';
import type { AccountSignature } from '@/types';
import { useLogStore } from './logStore';
import { useSignatureStore } from './signatureStore';

const sig = (accountId: string, html: string): AccountSignature => ({
  accountId,
  html,
  useForNew: true,
  useForReplies: true,
  updatedAt: 1,
});

beforeEach(() => {
  vi.mocked(api.getAccountSignature).mockReset();
  vi.mocked(api.saveAccountSignature).mockReset();
  useSignatureStore.setState({ byAccount: {} });
});

describe('signatureStore', () => {
  it('loads an account signature once and serves it from the cache', async () => {
    vi.mocked(api.getAccountSignature).mockResolvedValue(sig('acc1', '<p>Ana</p>'));
    expect(await useSignatureStore.getState().load('acc1')).toEqual(sig('acc1', '<p>Ana</p>'));
    expect(await useSignatureStore.getState().load('acc1')).toEqual(sig('acc1', '<p>Ana</p>'));
    expect(api.getAccountSignature).toHaveBeenCalledTimes(1);
  });

  it('reports a failed load and yields no signature', async () => {
    vi.mocked(api.getAccountSignature).mockRejectedValue(new Error('db locked'));
    const before = useLogStore.getState().entries.length;
    expect(await useSignatureStore.getState().load('acc1')).toBeNull();
    const logs = useLogStore.getState().entries.slice(before);
    expect(logs.some((l) => l.level === 'error' && l.message.includes('db locked'))).toBe(true);
  });

  it('keeps what the backend stored after a save', async () => {
    vi.mocked(api.saveAccountSignature).mockResolvedValue(sig('acc1', '<p>Clean</p>'));
    const saved = await useSignatureStore
      .getState()
      .save('acc1', { html: '<p onclick="x">Clean</p>', useForNew: true, useForReplies: true });
    expect(saved.html).toBe('<p>Clean</p>');
    expect(useSignatureStore.getState().byAccount.acc1?.html).toBe('<p>Clean</p>');
  });
});
