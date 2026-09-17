import { describe, expect, it, vi } from 'vitest';
import { type ChatToolEffectPayload, handleChatToolEffect, parseNavTarget } from './chatToolEffects';

describe('handleChatToolEffect', () => {
  it('calls openComposeTab with the payload fields for new-mail openComposer (no emailId)', () => {
    const openComposeTab = vi.fn();
    const openThreadReply = vi.fn();
    const navigateToInbox = vi.fn();
    const log = vi.fn();
    const payload: ChatToolEffectPayload = {
      kind: 'openComposer',
      draftId: 'd1',
      accountId: 'acc1',
      toAddresses: ['a@x.com'],
      subject: 'Hi',
      body: 'hello there',
    };
    handleChatToolEffect(payload, { openComposeTab, openThreadReply, navigateToInbox, log });
    expect(openThreadReply).not.toHaveBeenCalled();
    expect(openComposeTab).toHaveBeenCalledTimes(1);
    const [accountId, toAddresses, subject, bodyHtml] = openComposeTab.mock.calls[0];
    expect(accountId).toBe('acc1');
    expect(toAddresses).toEqual(['a@x.com']);
    expect(subject).toBe('Hi');
    // body went through plainTextToHtml — should be HTML wrapping the text.
    expect(bodyHtml).toContain('hello there');
    expect(log).toHaveBeenCalledWith('success', 'ai', expect.stringContaining('d1'));
  });

  it('routes openComposer with emailId to openThreadReply (inline reply path)', () => {
    // When the backend draft carries the inbound emailId, the user wants the
    // draft to land inside the existing thread — same UX as clicking Reply
    // on the thread — not in a standalone Compose tab.
    const openComposeTab = vi.fn();
    const openThreadReply = vi.fn();
    const navigateToInbox = vi.fn();
    const log = vi.fn();
    handleChatToolEffect(
      {
        kind: 'openComposer',
        draftId: 'd1',
        accountId: 'acc1',
        emailId: 'eml-7',
        toAddresses: ['a@x.com'],
        subject: 'Re: Hi',
        body: 'sounds good',
      },
      { openComposeTab, openThreadReply, navigateToInbox, log },
    );
    expect(openComposeTab).not.toHaveBeenCalled();
    expect(openThreadReply).toHaveBeenCalledTimes(1);
    const [accountId, emailId, body] = openThreadReply.mock.calls[0];
    expect(accountId).toBe('acc1');
    expect(emailId).toBe('eml-7');
    // Plain text — the inline ReplyCompose body is a plain-text textarea.
    expect(body).toBe('sounds good');
    expect(navigateToInbox).toHaveBeenCalledTimes(1);
    expect(log).toHaveBeenCalledWith('success', 'ai', expect.stringContaining('d1'));
  });

  it('navigates to inbox BEFORE opening the compose tab so the new tab is visible', () => {
    // Order matters — the tab bar only renders for the inbox-family views.
    // If openComposeTab fires while viewMode is "chat", the tab is appended
    // but stays hidden behind the chat panel.
    const order: string[] = [];
    const navigateToInbox = vi.fn(() => {
      order.push('navigate');
    });
    const openComposeTab = vi.fn(() => {
      order.push('open');
    });
    const openThreadReply = vi.fn();
    handleChatToolEffect(
      {
        kind: 'openComposer',
        draftId: 'd1',
        accountId: 'acc1',
        toAddresses: ['a@x.com'],
        subject: 'Hi',
        body: 'body',
      },
      { openComposeTab, openThreadReply, navigateToInbox },
    );
    expect(order).toEqual(['navigate', 'open']);
  });

  it('on the reply path, sets pending draft BEFORE navigating so EmailView reads it on mount', () => {
    // openThreadReply seeds emailStore.pendingChatDraft + calls
    // navigateToEmail. If navigateToInbox fires first and renders the inbox
    // (cached selectedEmail still in store from before), EmailView could
    // see the right thread + no pending draft and open a stale plain Reply.
    const order: string[] = [];
    const navigateToInbox = vi.fn(() => {
      order.push('navigate');
    });
    const openThreadReply = vi.fn(() => {
      order.push('reply');
    });
    const openComposeTab = vi.fn();
    handleChatToolEffect(
      {
        kind: 'openComposer',
        draftId: 'd1',
        accountId: 'acc1',
        emailId: 'eml-7',
        toAddresses: ['a@x.com'],
        subject: 'Re: Hi',
        body: 'body',
      },
      { openComposeTab, openThreadReply, navigateToInbox },
    );
    expect(order).toEqual(['reply', 'navigate']);
  });

  it('logs an error and skips the open when openComposer is missing required fields', () => {
    const openComposeTab = vi.fn();
    const openThreadReply = vi.fn();
    const navigateToInbox = vi.fn();
    const log = vi.fn();
    // Cast through unknown so TS lets us simulate an under-typed payload —
    // exactly what a backend version skew would deliver.
    handleChatToolEffect({ kind: 'openComposer' } as unknown as ChatToolEffectPayload, {
      openComposeTab,
      openThreadReply,
      navigateToInbox,
      log,
    });
    expect(openComposeTab).not.toHaveBeenCalled();
    expect(openThreadReply).not.toHaveBeenCalled();
    expect(navigateToInbox).not.toHaveBeenCalled();
    expect(log).toHaveBeenCalledWith('error', 'ai', expect.stringContaining('missing fields'));
  });

  it('falls back to debug log for an unknown effect kind without throwing', () => {
    const openComposeTab = vi.fn();
    const openThreadReply = vi.fn();
    const navigateToInbox = vi.fn();
    const log = vi.fn();
    handleChatToolEffect({ kind: 'someFutureEffect' } as ChatToolEffectPayload, {
      openComposeTab,
      openThreadReply,
      navigateToInbox,
      log,
    });
    expect(openComposeTab).not.toHaveBeenCalled();
    expect(openThreadReply).not.toHaveBeenCalled();
    expect(navigateToInbox).not.toHaveBeenCalled();
    expect(log).toHaveBeenCalledWith('debug', 'ai', expect.stringContaining('someFutureEffect'));
  });

  it('tolerates missing toAddresses on openComposer (defaults to empty array)', () => {
    const openComposeTab = vi.fn();
    const openThreadReply = vi.fn();
    const navigateToInbox = vi.fn();
    handleChatToolEffect(
      {
        kind: 'openComposer',
        draftId: 'd1',
        accountId: 'acc1',
        toAddresses: undefined as unknown as string[],
        subject: 'Hi',
        body: 'body',
      },
      { openComposeTab, openThreadReply, navigateToInbox },
    );
    expect(openComposeTab.mock.calls[0][1]).toEqual([]);
  });

  it('navigateTo settings/<tab> opens that settings tab', () => {
    const openSettingsTab = vi.fn();
    const navigateToView = vi.fn();
    const log = vi.fn();
    handleChatToolEffect(
      { kind: 'navigateTo', target: 'settings/ai', title: 'AI features › Choosing a backend' },
      {
        openComposeTab: vi.fn(),
        openThreadReply: vi.fn(),
        navigateToInbox: vi.fn(),
        openSettingsTab,
        navigateToView,
        log,
      },
    );
    expect(openSettingsTab).toHaveBeenCalledWith('ai');
    expect(navigateToView).not.toHaveBeenCalled();
    expect(log).toHaveBeenCalledWith('success', 'ai', expect.stringContaining('Choosing a backend'));
  });

  it('navigateTo view/<mode> switches the main view', () => {
    const openSettingsTab = vi.fn();
    const navigateToView = vi.fn();
    handleChatToolEffect(
      { kind: 'navigateTo', target: 'view/tagboard', title: 'AI features › Tag Board' },
      { openComposeTab: vi.fn(), openThreadReply: vi.fn(), navigateToInbox: vi.fn(), openSettingsTab, navigateToView },
    );
    expect(navigateToView).toHaveBeenCalledWith('tagboard');
    expect(openSettingsTab).not.toHaveBeenCalled();
  });

  it('navigateTo ignores targets outside the allowlists and logs a warning', () => {
    const openSettingsTab = vi.fn();
    const navigateToView = vi.fn();
    const log = vi.fn();
    for (const target of ['settings/nope', 'view/folder:abc', 'dialog/ai', 'settings']) {
      handleChatToolEffect(
        { kind: 'navigateTo', target, title: 'x' },
        {
          openComposeTab: vi.fn(),
          openThreadReply: vi.fn(),
          navigateToInbox: vi.fn(),
          openSettingsTab,
          navigateToView,
          log,
        },
      );
    }
    expect(openSettingsTab).not.toHaveBeenCalled();
    expect(navigateToView).not.toHaveBeenCalled();
    expect(log).toHaveBeenCalledTimes(4);
    expect(log).toHaveBeenCalledWith('error', 'ai', expect.stringContaining('unknown target'));
  });

  it('navigateTo without wired handlers is a logged no-op', () => {
    const log = vi.fn();
    handleChatToolEffect(
      { kind: 'navigateTo', target: 'settings/ai', title: 'x' },
      { openComposeTab: vi.fn(), openThreadReply: vi.fn(), navigateToInbox: vi.fn(), log },
    );
    expect(log).toHaveBeenCalledWith('debug', 'ai', expect.stringContaining('no settings handler'));
  });

  it('parseNavTarget mirrors the backend allowlists', () => {
    expect(parseNavTarget('settings/privacy')).toEqual({ kind: 'settings', tab: 'privacy' });
    expect(parseNavTarget('view/calendar')).toEqual({ kind: 'view', view: 'calendar' });
    expect(parseNavTarget('view/')).toBeNull();
    expect(parseNavTarget('')).toBeNull();
  });
});
