// The root key handler: real keydown events in jsdom drive the stores and the
// app callbacks the shortcut table promises.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({
  getPref: vi.fn(async () => null),
  setPref: vi.fn(async () => {}),
  currentPlatform: () => 'macos',
}));

import { useEmailStore } from '@/stores/emailStore';
import { useSelectionStore } from '@/stores/selectionStore';
import { useShortcutStore } from '@/stores/shortcutStore';
import type { Email } from '@/types';
import { type GlobalShortcutHost, useGlobalShortcuts } from './useGlobalShortcuts';

const row = (id: string, over: Partial<Email> = {}): Email =>
  ({
    id,
    accountId: 'a1',
    threadId: `t-${id}`,
    mailbox: 'inbox',
    isRead: false,
    isStarred: false,
    subject: `Subject ${id}`,
    ...over,
  }) as Email;

const LIST = [row('e1'), row('e2', { isStarred: true }), row('e3')];

let container: HTMLDivElement;
let root: Root;
let host: GlobalShortcutHost;
const actions = {
  archiveThreads: vi.fn(async () => {}),
  deleteThreads: vi.fn(async () => {}),
  setThreadsRead: vi.fn(async () => {}),
  setThreadsStarred: vi.fn(async () => {}),
};

function Harness({ h }: { h: GlobalShortcutHost }) {
  useGlobalShortcuts(h);
  return (
    <div>
      <input data-testid="field" />
      <div data-testid="editor" contentEditable suppressContentEditableWarning />
      <button type="button" data-testid="button">
        ok
      </button>
    </div>
  );
}

function makeHost(over: Partial<GlobalShortcutHost> = {}): GlobalShortcutHost {
  return {
    listView: true,
    layout: 'full-width',
    openConversation: false,
    openEmailId: null,
    openEmail: vi.fn(),
    closeConversation: vi.fn(),
    compose: vi.fn(),
    focusSearch: vi.fn(),
    openSearchPalette: vi.fn(),
    goTo: vi.fn(),
    ...over,
  };
}

async function render(over: Partial<GlobalShortcutHost> = {}) {
  host = makeHost(over);
  await act(async () => {
    root.render(<Harness h={host} />);
  });
}

function press(key: string, init: KeyboardEventInit = {}, target: EventTarget = document.body) {
  const event = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true, ...init });
  act(() => {
    target.dispatchEvent(event);
  });
  return event;
}

const el = (id: string) => container.querySelector(`[data-testid="${id}"]`) as HTMLElement;

beforeEach(() => {
  for (const fn of Object.values(actions)) fn.mockClear();
  useEmailStore.setState({ ...actions });
  useSelectionStore.getState().clear();
  useShortcutStore.setState({
    enabled: true,
    helpOpen: false,
    paneCommand: null,
    bulkSnoozeRequested: false,
    listEmails: LIST,
    cursorId: null,
  });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  document.querySelectorAll('[data-test-modal]').forEach((n) => n.remove());
});

describe('useGlobalShortcuts — list navigation', () => {
  it('j / k move the cursor in the full-width list; Enter opens it', async () => {
    await render();
    press('j');
    press('j');
    expect(useShortcutStore.getState().cursorId).toBe('e2');
    press('k');
    expect(useShortcutStore.getState().cursorId).toBe('e1');
    const enter = press('Enter');
    expect(host.openEmail).toHaveBeenCalledWith(LIST[0]);
    expect(enter.defaultPrevented).toBe(true);
  });

  it('in the split layout j opens the next conversation', async () => {
    await render({ layout: 'split', openConversation: true, openEmailId: 'e1' });
    press('j');
    expect(host.openEmail).toHaveBeenCalledWith(LIST[1]);
    expect(useShortcutStore.getState().cursorId).toBe('e2');
  });

  it('x selects the cursor row and Escape clears the selection', async () => {
    await render();
    press('j');
    press('x');
    expect([...useSelectionStore.getState().ids]).toEqual(['e1']);
    press('Escape');
    expect(useSelectionStore.getState().ids.size).toBe(0);
  });

  it('* a selects every row', async () => {
    await render();
    press('*', { shiftKey: true });
    press('a');
    expect(useSelectionStore.getState().ids.size).toBe(3);
  });
});

describe('useGlobalShortcuts — conversation actions', () => {
  it('e archives the selected conversations and clears the selection', async () => {
    await render();
    useSelectionStore.getState().selectAll(['e1', 'e3']);
    press('e');
    expect(actions.archiveThreads).toHaveBeenCalledWith([
      { accountId: 'a1', threadId: 't-e1' },
      { accountId: 'a1', threadId: 't-e3' },
    ]);
    expect(useSelectionStore.getState().ids.size).toBe(0);
  });

  it('s on a selection stars all when any is unstarred', async () => {
    await render();
    useSelectionStore.getState().selectAll(['e1', 'e2']);
    press('s');
    expect(actions.setThreadsStarred).toHaveBeenCalledWith(expect.any(Array), true);
  });

  it('# deletes the cursor row in the full-width list', async () => {
    await render();
    press('j');
    press('#', { shiftKey: true });
    expect(actions.deleteThreads).toHaveBeenCalledWith([{ accountId: 'a1', threadId: 't-e1' }]);
  });

  it('Shift+U on the cursor row marks it unread; s unstars a starred row', async () => {
    await render();
    press('j');
    press('j');
    press('U', { shiftKey: true });
    expect(actions.setThreadsRead).toHaveBeenCalledWith([{ accountId: 'a1', threadId: 't-e2' }], false);
    press('s');
    expect(actions.setThreadsStarred).toHaveBeenCalledWith([{ accountId: 'a1', threadId: 't-e2' }], false);
  });

  it('with a conversation open, actions go to the reading pane', async () => {
    await render({ layout: 'split', openConversation: true, openEmailId: 'e1' });
    press('r');
    expect(useShortcutStore.getState().paneCommand?.command).toBe('reply');
    press('e');
    expect(useShortcutStore.getState().paneCommand?.command).toBe('archive');
    expect(actions.archiveThreads).not.toHaveBeenCalled();
  });

  it('b on a selection asks the bulk toolbar for its snooze picker', async () => {
    await render();
    useSelectionStore.getState().selectAll(['e1']);
    press('b');
    expect(useShortcutStore.getState().bulkSnoozeRequested).toBe(true);
  });
});

describe('useGlobalShortcuts — app shortcuts', () => {
  it('c composes, / searches, ⌘K opens the search palette, ? opens help', async () => {
    await render();
    press('c');
    expect(host.compose).toHaveBeenCalledTimes(1);
    press('/');
    expect(host.focusSearch).toHaveBeenCalledTimes(1);
    press('k', { metaKey: true });
    expect(host.openSearchPalette).toHaveBeenCalledTimes(1);
    press('?', { shiftKey: true });
    expect(useShortcutStore.getState().helpOpen).toBe(true);
  });

  it('g then i goes to the inbox', async () => {
    await render();
    press('g');
    press('i');
    expect(host.goTo).toHaveBeenCalledWith('inbox');
  });
});

// An Escape the page leaves unhandled goes back to AppKit, and a full-screen
// window takes it as "leave full screen".
describe('useGlobalShortcuts — Escape never leaves full screen', () => {
  it('is consumed even when it has nothing to do', async () => {
    await render({ layout: 'split' });
    expect(press('Escape').defaultPrevented).toBe(true);
  });

  it('is consumed from a text field, with a modal open and with shortcuts off', async () => {
    await render();
    expect(press('Escape', {}, el('field')).defaultPrevented).toBe(true);
    const modal = document.createElement('div');
    modal.setAttribute('aria-modal', 'true');
    modal.setAttribute('data-test-modal', '');
    document.body.appendChild(modal);
    expect(press('Escape').defaultPrevented).toBe(true);
    modal.remove();
    useShortcutStore.setState({ enabled: false });
    expect(press('Escape').defaultPrevented).toBe(true);
  });

  it('still lets a search box with text clear itself', async () => {
    await render();
    const search = document.createElement('input');
    search.type = 'search';
    search.value = 'invoice';
    container.appendChild(search);
    expect(press('Escape', {}, search).defaultPrevented).toBe(false);
  });
});

describe('useGlobalShortcuts — when not to fire', () => {
  it('ignores single keys typed into an input or a rich-text editor', async () => {
    await render();
    press('c', {}, el('field'));
    press('c', {}, el('editor'));
    expect(host.compose).not.toHaveBeenCalled();
  });

  it('⌘K still works from an input', async () => {
    await render();
    press('k', { metaKey: true }, el('field'));
    expect(host.openSearchPalette).toHaveBeenCalledTimes(1);
  });

  it('Enter on a focused button clicks the button, it does not open a row', async () => {
    await render();
    press('j');
    press('Enter', {}, el('button'));
    expect(host.openEmail).not.toHaveBeenCalled();
  });

  it('nothing fires while a modal dialog is open', async () => {
    await render();
    const modal = document.createElement('div');
    modal.setAttribute('aria-modal', 'true');
    modal.setAttribute('data-test-modal', '');
    document.body.appendChild(modal);
    press('c');
    press('k', { metaKey: true });
    expect(host.compose).not.toHaveBeenCalled();
    expect(host.openSearchPalette).not.toHaveBeenCalled();
  });

  it('a key another handler already consumed is left alone', async () => {
    await render();
    const event = new KeyboardEvent('keydown', { key: 'c', bubbles: true, cancelable: true });
    event.preventDefault();
    act(() => {
      document.body.dispatchEvent(event);
    });
    expect(host.compose).not.toHaveBeenCalled();
  });

  it('the setting turns every shortcut off, ⌘K included', async () => {
    useShortcutStore.setState({ enabled: false });
    await render();
    press('c');
    press('k', { metaKey: true });
    expect(host.compose).not.toHaveBeenCalled();
    expect(host.openSearchPalette).not.toHaveBeenCalled();
  });
});
