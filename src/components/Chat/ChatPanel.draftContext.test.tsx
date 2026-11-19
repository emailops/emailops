// The chat panel offers the draft on screen as context. A reply draft carries
// its own thread, so the thread chip gives way while the draft chip is on and
// comes back when the user switches the draft off.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));
vi.mock('@/lib/api', () => ({ prewarmChat: vi.fn(async () => {}) }));
vi.mock('@/components/shared/Select', () => ({ Select: () => null }));
vi.mock('@/stores/viewContextStore', () => ({ useChatViewContext: () => null }));
vi.mock('./ChatAccountPicker', () => ({ ChatAccountPicker: () => null }));
vi.mock('./ChatStarters', () => ({ ChatStarters: () => null }));
vi.mock('./OtherAccountContextNotice', () => ({ OtherAccountContextNotice: () => null }));
vi.mock('./MessageList', () => ({
  MessageList: ({ onRejectMessage }: { onRejectMessage: (id: string, reason: string) => void }) => (
    <button type="button" data-testid="reject" onClick={() => onRejectMessage('m1', 'too long')} />
  ),
}));
vi.mock('./ChatInput', () => ({
  ChatInput: ({ contextSlot, onSend }: { contextSlot?: React.ReactNode; onSend: (text: string) => void }) => (
    <div>
      <div data-testid="slot">{contextSlot}</div>
      <button type="button" data-testid="send" onClick={() => onSend('is it complete?')} />
    </div>
  ),
}));

import { useChatStore } from '@/stores/chatStore';
import { type DraftHandle, useDraftContextStore } from '@/stores/draftContextStore';
import type { ChatDraftContext, ChatMessage } from '@/types';
import { ChatPanel } from './ChatPanel';

const draft: ChatDraftContext = {
  kind: 'reply',
  subject: 'Re: Q3 report',
  to: ['ana@example.com'],
  cc: [],
  body: 'The report is attached.',
  replyToEmailId: 'e1',
};
const replyHandle: DraftHandle = { instanceId: 'c1', kind: 'reply', accountId: () => 'a1', snapshot: () => draft };
const openThread = { threadId: 't1', accountId: 'a1', subject: 'Q3 report' };

let container: HTMLDivElement;
let root: Root;
const sendMessage = vi.fn(async () => {});
const retryWithCorrection = vi.fn(async () => {});

beforeEach(() => {
  vi.clearAllMocks();
  useChatStore.setState({
    sendMessage,
    retryWithCorrection,
    currentAccountId: 'a1',
    activeConversationId: 'conv-1',
    categoriesLoaded: true,
    isLoadingMessages: false,
    messages: [{ id: 'm1', role: 'assistant', content: 'a' } as ChatMessage],
    researchMode: false,
  });
  useDraftContextStore.setState({ active: null, subject: '', hasUserText: false });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function render() {
  act(() =>
    root.render(
      <ChatPanel
        accountId="a1"
        onAccountChange={() => {}}
        context={openThread}
        onClose={() => {}}
        onExpand={() => {}}
      />,
    ),
  );
}

const chips = () => Array.from(container.querySelectorAll('[data-testid="slot"] button[aria-pressed]'));
const click = (el: Element) => act(() => (el as HTMLButtonElement).click());
const send = () => click(container.querySelector('[data-testid="send"]') as Element);
const lastSend = () => sendMessage.mock.calls[sendMessage.mock.calls.length - 1] as unknown[];

function openReplyDraft() {
  useDraftContextStore.setState({ active: replyHandle, subject: 'Re: Q3 report', hasUserText: true });
}

describe('ChatPanel with a draft on screen', () => {
  it('shows the draft chip in place of the thread chip', () => {
    openReplyDraft();
    render();
    expect(chips()).toHaveLength(1);
    expect(container.textContent).toContain('panel.draft.threadAndReply');
  });

  it('sends the draft, not the open thread, which travels with it', () => {
    openReplyDraft();
    render();
    send();
    const call = lastSend();
    expect(call[1]).toBeNull();
    expect(call[6]).toEqual(draft);
    expect(call[7]).toBe(true);
  });

  it('gives the thread chip back when the draft is switched off, still refusing new drafts', () => {
    openReplyDraft();
    render();
    click(chips()[0]);
    expect(chips()).toHaveLength(2);
    send();
    const call = lastSend();
    expect(call[1]).toBe('t1');
    expect(call[6]).toBeNull();
    expect(call[7]).toBe(true);
  });

  it('sends the draft again once it is switched back on', () => {
    openReplyDraft();
    render();
    click(chips()[0]);
    click(chips()[0]);
    send();
    expect(lastSend()[6]).toEqual(draft);
  });

  it('a retry sends the draft as it is when pressed', () => {
    openReplyDraft();
    render();
    click(container.querySelector('[data-testid="reject"]') as Element);
    const call = retryWithCorrection.mock.calls[retryWithCorrection.mock.calls.length - 1] as unknown[];
    expect(call[2]).toBeNull();
    expect(call[6]).toEqual(draft);
    expect(call[7]).toBe(true);
  });

  it('without a composer it behaves as before: the thread, no draft, none open', () => {
    render();
    expect(chips()).toHaveLength(1);
    send();
    const call = lastSend();
    expect(call[1]).toBe('t1');
    expect(call[6]).toBeNull();
    expect(call[7]).toBe(false);
  });
});
