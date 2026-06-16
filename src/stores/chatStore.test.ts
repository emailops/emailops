import { beforeEach, describe, expect, it, vi } from 'vitest';
import * as api from '@/lib/api';
import type { ChatMessage, ChatPhaseEvent, ChatStreamEvent } from '@/types';
import { useChatStore } from './chatStore';

vi.mock('@/lib/api');

function assistantMessage(id: string): ChatMessage {
  return {
    id,
    conversationId: 'conv-1',
    role: 'assistant',
    content: '',
    model: null,
    tokenCount: null,
    latencyMs: null,
    createdAt: 0,
    sources: [],
    referencedEmailIds: [],
    referencedDraftIds: [],
  };
}

function phaseEvent(overrides: Partial<ChatPhaseEvent> = {}): ChatPhaseEvent {
  return { messageId: 'msg-1', conversationId: 'conv-1', phase: 'retrieving', ...overrides };
}

function streamEvent(overrides: Partial<ChatStreamEvent> = {}): ChatStreamEvent {
  return { messageId: 'msg-1', conversationId: 'conv-1', token: '', done: false, ...overrides };
}

describe('chatStore processing phase', () => {
  beforeEach(() => {
    useChatStore.setState({
      activeConversationId: 'conv-1',
      streamingMessageId: 'msg-1',
      streamingPhase: null,
      messages: [assistantMessage('msg-1')],
      error: null,
    });
  });

  it('records the phase for the active conversation’s streaming message', () => {
    useChatStore.getState().handlePhase(phaseEvent({ phase: 'runningTools' }));
    expect(useChatStore.getState().streamingPhase).toBe('runningTools');
  });

  it('ignores a phase event for a different conversation', () => {
    useChatStore.getState().handlePhase(phaseEvent({ conversationId: 'other' }));
    expect(useChatStore.getState().streamingPhase).toBeNull();
  });

  it('ignores a phase event that is not for the streaming message', () => {
    useChatStore.getState().handlePhase(phaseEvent({ messageId: 'stale' }));
    expect(useChatStore.getState().streamingPhase).toBeNull();
  });

  it('records an early phase for the active conversation before the streaming id is set', () => {
    // Race: the backend reaches the first emit_phase (e.g. thread-bound chat
    // jumps almost straight to RunningTools) before sendMessage's command
    // returns and assigns streamingMessageId. The event must still register.
    useChatStore.setState({ streamingMessageId: null });
    useChatStore.getState().handlePhase(phaseEvent({ phase: 'runningTools' }));
    expect(useChatStore.getState().streamingPhase).toBe('runningTools');
  });

  it('ignores an early phase for a different conversation even when no id is set', () => {
    useChatStore.setState({ streamingMessageId: null });
    useChatStore.getState().handlePhase(phaseEvent({ conversationId: 'other' }));
    expect(useChatStore.getState().streamingPhase).toBeNull();
  });

  it('clears the phase once the stream completes', () => {
    useChatStore.getState().handlePhase(phaseEvent({ phase: 'generating' }));
    useChatStore.getState().handleStreamToken(streamEvent({ token: 'hi', done: true, tokenCount: 5, latencyMs: 10 }));
    expect(useChatStore.getState().streamingPhase).toBeNull();
  });

  it('keeps the phase while tokens are still streaming', () => {
    useChatStore.getState().handlePhase(phaseEvent({ phase: 'generating' }));
    useChatStore.getState().handleStreamToken(streamEvent({ token: 'partial', done: false }));
    expect(useChatStore.getState().streamingPhase).toBe('generating');
  });

  it('drops the phase on reset', () => {
    useChatStore.getState().handlePhase(phaseEvent());
    useChatStore.getState().reset();
    expect(useChatStore.getState().streamingPhase).toBeNull();
  });
});

describe('chatStore selectConversation', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // Simulate having navigated away mid-turn: the `done` stream event for the
    // backgrounded conversation is dropped by handleStreamToken's conversation
    // guard, so the streaming flags are still set when the user returns.
    useChatStore.setState({
      activeConversationId: 'conv-2',
      streamingMessageId: 'msg-1',
      streamingPhase: 'runningTools',
      messages: [],
      error: null,
    });
  });

  it('clears stale streaming state when opening a past conversation', async () => {
    const saved = { ...assistantMessage('msg-1'), content: 'the saved reply' };
    vi.mocked(api.getChatMessages).mockResolvedValue([saved]);

    await useChatStore.getState().selectConversation('conv-1');

    expect(useChatStore.getState().messages).toEqual([saved]);
    expect(useChatStore.getState().streamingMessageId).toBeNull();
    expect(useChatStore.getState().streamingPhase).toBeNull();
  });

  it('clears stale streaming state when deselecting the conversation', async () => {
    await useChatStore.getState().selectConversation(null);

    expect(useChatStore.getState().streamingMessageId).toBeNull();
    expect(useChatStore.getState().streamingPhase).toBeNull();
  });
});

describe('chatStore sendMessage preserves an early phase', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    useChatStore.setState({
      activeConversationId: 'conv-1',
      streamingMessageId: null,
      streamingPhase: null,
      messages: [],
      isSending: false,
      error: null,
      selectedCategories: ['primary'],
    });
  });

  it('keeps a phase that arrives during the send round-trip', async () => {
    // Thread-bound turns reach their first emit_phase almost immediately —
    // often before the send_chat_message command returns the assistant id.
    // Simulate that by emitting the phase from inside the mocked command,
    // before it resolves. The resolution must not wipe streamingPhase.
    vi.mocked(api.sendChatMessage).mockImplementation(async () => {
      useChatStore.getState().handlePhase(phaseEvent({ messageId: 'msg-1', phase: 'runningTools' }));
      return {
        userMessage: { ...assistantMessage('user-1'), role: 'user', content: 'resume este email' },
        assistantMessage: assistantMessage('msg-1'),
      };
    });

    await useChatStore.getState().sendMessage('resume este email');

    expect(useChatStore.getState().streamingMessageId).toBe('msg-1');
    expect(useChatStore.getState().streamingPhase).toBe('runningTools');
  });

  it('clears a stale phase from a previous turn when a new send starts', async () => {
    useChatStore.setState({ streamingPhase: 'generating' });
    vi.mocked(api.sendChatMessage).mockResolvedValue({
      userMessage: { ...assistantMessage('user-2'), role: 'user', content: 'hi' },
      assistantMessage: assistantMessage('msg-2'),
    });

    await useChatStore.getState().sendMessage('hi');

    expect(useChatStore.getState().streamingPhase).toBeNull();
  });
});
