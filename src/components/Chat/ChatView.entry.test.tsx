// Opening Chat from the sidebar on a phone should land on a blank new chat.
// The active conversation lives in the store, so it survives ChatView's
// unmount — which meant tapping Chat silently resumed whatever conversation
// was last open, with the history that would explain it on a different screen.
// Desktop keeps resuming: there the conversation list is always visible.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));
vi.mock('@/hooks/useResponsiveLayout', () => ({
  useResponsiveLayout: vi.fn(() => ({ isStacked: false, isMobile: false })),
}));
vi.mock('@/lib/api', () => ({ prewarmChat: vi.fn(() => Promise.resolve()) }));
vi.mock('@/components/shared/AccountScopeChip', () => ({ AccountScopeChip: () => null }));
vi.mock('./ChatInput', () => ({ ChatInput: () => null }));
vi.mock('./ConversationList', () => ({ ConversationList: () => null }));
vi.mock('./MessageList', () => ({ MessageList: () => null }));
vi.mock('@/stores/accountStore', () => ({
  isUnifiedMode: () => false,
  useAccountStore: (selector: (s: unknown) => unknown) => selector({ activeAccountId: 'a1', accounts: [] }),
}));
vi.mock('@/stores/logStore', () => ({
  useLogStore: (selector: (s: unknown) => unknown) => selector({ addLog: vi.fn() }),
}));

const selectConversation = vi.fn(() => Promise.resolve());
const fetchConversations = vi.fn(() => Promise.resolve());

vi.mock('@/stores/chatStore', () => ({
  useChatStore: () => ({
    conversations: [],
    activeConversationId: 'c1',
    messages: [],
    streamingMessageId: null,
    streamingPhase: null,
    isSending: false,
    isLoadingConversations: false,
    isLoadingMessages: false,
    error: null,
    fetchConversations,
    createConversation: vi.fn(),
    selectConversation,
    renameConversation: vi.fn(),
    deleteConversation: vi.fn(),
    sendMessage: vi.fn(),
    loadCategoriesPref: vi.fn(),
    categoriesLoaded: true,
    selectedCategories: new Set<string>(),
  }),
}));

import { useResponsiveLayout } from '@/hooks/useResponsiveLayout';
import { ChatView } from './ChatView';

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.clearAllMocks();
});

async function mount(isStacked: boolean, accountId: string | null = 'a1') {
  vi.mocked(useResponsiveLayout).mockReturnValue({ isStacked, isMobile: isStacked });
  await act(async () => {
    root.render(<ChatView accountId={accountId} onNavigateToInbox={() => {}} />);
  });
}

describe('ChatView entry', () => {
  it('starts a blank chat when opened on a phone', async () => {
    await mount(true);

    expect(selectConversation).toHaveBeenCalledWith(null);
  });

  it('resumes the last conversation on a desktop', async () => {
    // Deliberate: the conversation list is on screen there, so resuming is
    // both visible and one click away from any other conversation.
    await mount(false);

    expect(selectConversation).not.toHaveBeenCalled();
  });

  it('still loads the conversation list on a phone', async () => {
    // Clearing the selection must not also skip the fetch — the history screen
    // would come up empty.
    await mount(true);

    expect(fetchConversations).toHaveBeenCalledWith('a1');
  });

  it('does nothing without an account', async () => {
    await mount(true, null);

    expect(selectConversation).not.toHaveBeenCalled();
  });
});
