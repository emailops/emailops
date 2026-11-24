// The conversation view preference, stored in SQLite like the other
// Settings → Appearance choices.

import { beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({
  getPref: vi.fn(async () => null),
  setPref: vi.fn(async () => {}),
}));

import * as api from '@/lib/api';
import { CONVERSATION_VIEW_PREF } from '@/lib/conversationView';
import { useConversationViewStore } from './conversationViewStore';
import { useLogStore } from './logStore';

const view = () => useConversationViewStore.getState().view;

beforeEach(() => {
  useConversationViewStore.setState({ view: 'per-message' });
  vi.clearAllMocks();
});

describe('the conversation view preference', () => {
  it('loads from SQLite, defaulting to the per-message view', async () => {
    vi.mocked(api.getPref).mockResolvedValueOnce('own-first');
    await useConversationViewStore.getState().load();
    expect(api.getPref).toHaveBeenCalledWith(CONVERSATION_VIEW_PREF);
    expect(view()).toBe('own-first');
    vi.mocked(api.getPref).mockResolvedValueOnce(null);
    await useConversationViewStore.getState().load();
    expect(view()).toBe('per-message');
  });

  it('keeps the default and logs when the preference cannot be read', async () => {
    vi.mocked(api.getPref).mockRejectedValueOnce(new Error('db locked'));
    await useConversationViewStore.getState().load();
    expect(view()).toBe('per-message');
    expect(useLogStore.getState().entries.some((l) => l.level === 'error' && l.message.includes('db locked'))).toBe(
      true,
    );
  });

  it('saves, and keeps the old value when saving fails', async () => {
    await useConversationViewStore.getState().setView('own-first');
    expect(api.setPref).toHaveBeenCalledWith(CONVERSATION_VIEW_PREF, 'own-first');
    expect(view()).toBe('own-first');
    vi.mocked(api.setPref).mockRejectedValueOnce(new Error('disk full'));
    await expect(useConversationViewStore.getState().setView('per-message')).rejects.toThrow('disk full');
    expect(view()).toBe('own-first');
  });
});
