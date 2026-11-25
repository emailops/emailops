// Settings → Appearance: how an open conversation is laid out, stored in the
// SQLite preferences.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({
  getPref: vi.fn(async () => null),
  setPref: vi.fn(async () => {}),
  currentPlatform: () => 'macos',
}));

import { initI18n } from '@/i18n';
import * as api from '@/lib/api';
import { CONVERSATION_VIEW_PREF } from '@/lib/conversationView';
import { useConversationViewStore } from '@/stores/conversationViewStore';
import { useLogStore } from '@/stores/logStore';
import { ConversationViewSetting } from './ConversationViewSetting';

let container: HTMLDivElement;
let root: Root;

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  useConversationViewStore.setState({ view: 'per-message' });
  vi.mocked(api.setPref).mockReset();
  vi.mocked(api.setPref).mockResolvedValue(undefined);
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function render() {
  await act(async () => {
    root.render(<ConversationViewSetting />);
  });
}

const select = () => container.querySelector('select') as HTMLSelectElement;

async function choose(value: string) {
  await act(async () => {
    select().value = value;
    select().dispatchEvent(new Event('change', { bubbles: true }));
  });
}

describe('ConversationViewSetting', () => {
  it('offers the per-message view, selected by default, and the own-message-first view', async () => {
    await render();
    expect(container.textContent).toContain('Conversation view');
    expect(Array.from(select().options).map((o) => o.textContent)).toEqual([
      'Open the latest message, expand the others one by one',
      'Your latest message first, every message open',
    ]);
    expect(select().value).toBe('per-message');
  });

  it('stores the choice', async () => {
    await render();
    await choose('own-first');
    expect(api.setPref).toHaveBeenCalledWith(CONVERSATION_VIEW_PREF, 'own-first');
    expect(useConversationViewStore.getState().view).toBe('own-first');
  });

  it('shows and logs a failed save, keeping the old choice', async () => {
    vi.mocked(api.setPref).mockRejectedValue(new Error('disk full'));
    await render();
    await choose('own-first');
    expect(container.textContent).toContain('disk full');
    expect(useConversationViewStore.getState().view).toBe('per-message');
    expect(useLogStore.getState().entries.some((l) => l.level === 'error' && l.message.includes('disk full'))).toBe(
      true,
    );
  });
});
