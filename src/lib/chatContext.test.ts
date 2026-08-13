import { describe, expect, it } from 'vitest';
import { chatTurnContext, deriveChatContext, isConversationThreadBound } from '@/lib/chatContext';
import type { ChatMessage, Email } from '@/types';

// The planner reads only threadId / accountId / subject off an Email. Building
// a full row here would obscure that, so the fixture states the dependency and
// casts the rest.
function email(overrides: Partial<Email> = {}): Email {
  return {
    id: 'e1',
    accountId: 'acct-1',
    threadId: 'thread-1',
    subject: 'Quarterly planning',
    ...overrides,
  } as Email;
}

describe('deriveChatContext', () => {
  it('returns null when nothing is open', () => {
    expect(deriveChatContext({ viewMode: 'inbox', activeTab: null, selectedEmail: null })).toBeNull();
  });

  it('uses the selected email when no tab is active', () => {
    const selected = { ...email(), threadId: 'thread-7', subject: 'Budget review' };
    expect(deriveChatContext({ viewMode: 'inbox', activeTab: null, selectedEmail: selected })).toEqual({
      threadId: 'thread-7',
      accountId: 'acct-1',
      subject: 'Budget review',
    });
  });

  it('prefers the active thread tab over the selected email', () => {
    // What the user is actually looking at is the foreground tab, not whatever
    // is still selected in the list behind it.
    const selected = { ...email(), threadId: 'thread-7', subject: 'Budget review' };
    const tab = {
      type: 'thread' as const,
      id: 'tab-1',
      threadId: 'thread-9',
      accountId: 'acct-2',
      subject: 'Vendor contract',
      threadEmails: [],
      isLoading: false,
      focusEmailId: null,
    };
    expect(deriveChatContext({ viewMode: 'inbox', activeTab: tab, selectedEmail: selected })).toEqual({
      threadId: 'thread-9',
      accountId: 'acct-2',
      subject: 'Vendor contract',
    });
  });

  it('ignores compose and attachment tabs', () => {
    // Neither is an email thread the model can be grounded in; fall through to
    // the selected email instead.
    const selected = { ...email(), threadId: 'thread-7', subject: 'Budget review' };
    const compose = {
      type: 'compose' as const,
      id: 'tab-2',
      accountId: 'acct-1',
      toAddresses: [],
      subject: '',
      bodyHtml: '',
    };
    expect(deriveChatContext({ viewMode: 'inbox', activeTab: compose as never, selectedEmail: selected })).toEqual({
      threadId: 'thread-7',
      accountId: 'acct-1',
      subject: 'Budget review',
    });
  });

  it('returns null outside mailbox-backed views', () => {
    // Calendar/contacts/tasks/etc. have no thread to ground a turn in, even if
    // an email is still selected underneath.
    const selected = email();
    for (const viewMode of ['calendar', 'contacts', 'tasks', 'memory', 'lenses', 'dashboard'] as const) {
      expect(deriveChatContext({ viewMode, activeTab: null, selectedEmail: selected })).toBeNull();
    }
  });

  it('works across every mailbox-backed view', () => {
    const selected = email();
    for (const viewMode of ['inbox', 'sent', 'spam', 'deleted', 'folder:Archive/2026'] as const) {
      expect(deriveChatContext({ viewMode, activeTab: null, selectedEmail: selected })?.threadId).toBe('thread-1');
    }
  });

  it('falls back to a placeholder subject when the thread has none', () => {
    const selected = { ...email(), subject: '' };
    expect(deriveChatContext({ viewMode: 'inbox', activeTab: null, selectedEmail: selected })?.subject).toBe('');
  });
});

describe('isConversationThreadBound', () => {
  const msg = (role: string): ChatMessage => ({ role }) as ChatMessage;

  it('is false for an ordinary conversation', () => {
    expect(isConversationThreadBound([msg('user'), msg('assistant')])).toBe(false);
  });

  it('is false for an empty conversation', () => {
    expect(isConversationThreadBound([])).toBe(false);
  });

  it('is true when a seeded system message is present', () => {
    // "Chat about this thread" seeds one at creation. The backend gives that
    // binding precedence over ambient view context, so the panel must not
    // claim the open thread is being used.
    expect(isConversationThreadBound([msg('system'), msg('user')])).toBe(true);
  });
});

describe('chatTurnContext', () => {
  const context = { threadId: 'thread-7', accountId: 'acct-owning-thread-7', subject: 'Budget review' };

  it('sends the thread together with the account that owns it', () => {
    // Regression: only the threadId used to be sent. In unified mode the chat
    // runs on the first enabled account, so the backend looked the thread up
    // under an account that does not own it, found nothing, and silently fell
    // back to retrieval — the user asked about the open email and was told the
    // model did not know which email they meant.
    expect(chatTurnContext(context, true)).toEqual({
      threadId: 'thread-7',
      accountId: 'acct-owning-thread-7',
    });
  });

  it('sends nothing when the chip was dismissed', () => {
    // A dismissed chip must behave exactly like having nothing open.
    expect(chatTurnContext(context, false)).toBeNull();
  });

  it('sends nothing when no context is offered', () => {
    expect(chatTurnContext(null, true)).toBeNull();
  });
});
