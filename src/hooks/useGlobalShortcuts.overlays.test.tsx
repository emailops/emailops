// No conversation shortcut may act while an overlay is on screen.
//
// A contributor's Delete key trashed the conversation behind a dialog because
// the "is a dialog open?" check only knew some overlays. These tests render the
// real overlays next to the real app-wide key handler, press every
// conversation key, and require that nothing happened: no store action, no
// pane command, no cursor move, no row opened.

import type { ReactElement } from 'react';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

// Every api call resolves to nothing: the overlays only have to render.
vi.mock('@/lib/api', () => {
  const fns = new Map<PropertyKey, unknown>([['currentPlatform', () => 'macos']]);
  return new Proxy(
    {},
    {
      has: () => true,
      get: (_t, key) => {
        if (key === 'then' || key === '__esModule') return undefined;
        if (!fns.has(key))
          fns.set(
            key,
            vi.fn(async () => null),
          );
        return fns.get(key);
      },
    },
  );
});
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}), emit: vi.fn(async () => {}) }));
vi.mock('@/components/shared/EmailPreviewById', () => ({ EmailPreviewById: () => null }));
// Settings' tab bodies are not what is tested here; the dialog shell is.
for (const m of [
  '@/components/Settings/AiDraftsSettings',
  '@/components/Settings/AiSearchSettings',
  '@/components/Settings/AiSettings',
  '@/components/Settings/AiTranslationSettings',
  '@/components/Settings/CalendarSettings',
  '@/components/Settings/ClassificationSettings',
  '@/components/Settings/JunkSettings',
  '@/components/Settings/LensesSettings',
  '@/components/Settings/MemorySettings',
  '@/components/Settings/PrivacySettings',
  '@/components/Settings/TasksSettings',
  '@/components/Settings/SignaturesSettings',
]) {
  vi.doMock(m, () => new Proxy({}, { get: () => () => null }));
}

import { AddImapAccountModal } from '@/components/AddImapAccountModal';
import { ChatInput } from '@/components/Chat/ChatInput';
import { ComposeModal } from '@/components/ComposeModal';
import { ShortcutHelpModal } from '@/components/common/ShortcutHelpModal';
import { AttachmentLightbox } from '@/components/EmailView/AttachmentLightbox';
import { EmailActionsMenu } from '@/components/Inbox/EmailActionsMenu';
import { SnoozeMenuButton } from '@/components/Inbox/SnoozePicker';
import { LensRowDrawer } from '@/components/Lenses/LensRowDrawer';
import { OnboardingWizard } from '@/components/Onboarding/OnboardingWizard';
import { SettingsDialog } from '@/components/Settings/SettingsDialog';
import { AccountSettingsDialog } from '@/components/Sidebar/AccountSettingsDialog';
import { AddAccountModal } from '@/components/Sidebar/AddAccountModal';
import { SenderDialogs } from '@/components/shared/SenderDialogs';
import { initI18n } from '@/i18n';
import { useEmailStore } from '@/stores/emailStore';
import { useOverlayStore } from '@/stores/overlayStore';
import { useSelectionStore } from '@/stores/selectionStore';
import { statusKey, useSenderStore } from '@/stores/senderStore';
import { useShortcutStore } from '@/stores/shortcutStore';
import type { Account, Email } from '@/types';
import { type GlobalShortcutHost, useGlobalShortcuts } from './useGlobalShortcuts';

const row = (id: string): Email =>
  ({
    id,
    accountId: 'a1',
    threadId: `t-${id}`,
    mailbox: 'inbox',
    isRead: true,
    isStarred: false,
    subject: `Subject ${id}`,
    sender: 'Ana',
    senderEmail: 'ana@example.com',
    timestamp: 1_700_000_000,
  }) as Email;

const LIST = [row('e1'), row('e2'), row('e3')];
const ACCOUNT = {
  id: 'a1',
  email: 'me@example.com',
  name: 'Me',
  displayName: 'Me',
  provider: 'gmail',
  enabled: true,
} as unknown as Account;

const actions = {
  archiveThreads: vi.fn(async () => {}),
  deleteThreads: vi.fn(async () => {}),
  setThreadsRead: vi.fn(async () => {}),
  setThreadsStarred: vi.fn(async () => {}),
  snoozeThreads: vi.fn(async () => {}),
};

let container: HTMLDivElement;
let root: Root;
let host: GlobalShortcutHost;

function Harness({ h, overlay }: { h: GlobalShortcutHost; overlay: ReactElement }) {
  useGlobalShortcuts(h);
  return (
    <div>
      <button type="button" data-testid="other-row">
        row
      </button>
      {overlay}
    </div>
  );
}

function makeHost(over: Partial<GlobalShortcutHost>): GlobalShortcutHost {
  return {
    listView: true,
    layout: 'split',
    openConversation: true,
    openEmailId: 'e1',
    openEmail: vi.fn(),
    closeConversation: vi.fn(),
    compose: vi.fn(),
    focusSearch: vi.fn(),
    openSearchPalette: vi.fn(),
    goTo: vi.fn(),
    ...over,
  };
}

const noop = () => {};

interface OverlayCase {
  name: string;
  render: () => ReactElement;
  /** Puts the overlay on screen after the first render (a click, a store flag). */
  open?: () => void;
  /** Where the keys land (default: whatever has focus, else the body). */
  target?: () => EventTarget;
}

const el = (selector: string) => document.querySelector(selector) as HTMLElement;
const clickEl = (selector: string) =>
  act(() => {
    el(selector).click();
  });

const OVERLAYS: OverlayCase[] = [
  {
    name: 'Settings dialog',
    render: () => (
      <SettingsDialog
        activeAccountId={null}
        currentLayout="split"
        onChangeLayout={noop}
        tasksEnabled={false}
        onChangeTasksEnabled={noop}
        memoriesEnabled={false}
        onChangeMemoriesEnabled={noop}
        lensesEnabled={false}
        onChangeLensesEnabled={noop}
        onClose={noop}
      />
    ),
  },
  {
    name: 'attachment lightbox',
    render: () => (
      <AttachmentLightbox
        meta={{
          id: 'm1',
          emailId: 'e1',
          accountId: 'a1',
          providerAttachmentId: 'p1',
          filename: 'photo.png',
          mimeType: 'image/png',
          fileSize: 10,
          filePath: null,
        }}
        onClose={noop}
      />
    ),
  },
  {
    name: 'compose modal',
    render: () => <ComposeModal accounts={[ACCOUNT]} defaultAccountId="a1" onClose={noop} />,
    target: () => document.body,
  },
  {
    name: 'row ⋮ menu',
    render: () => <EmailActionsMenu email={LIST[1]} onStatus={noop} />,
    open: () => clickEl('[aria-label="More actions"]'),
  },
  {
    name: 'snooze picker',
    render: () => <SnoozeMenuButton testId="snooze" onPick={noop} />,
    open: () => clickEl('[data-testid="snooze"]'),
  },
  {
    name: 'add-account modal',
    render: () => <AddAccountModal isSubmitting={false} onClose={noop} onConfirm={noop} />,
  },
  {
    name: 'add-IMAP-account modal',
    render: () => <AddImapAccountModal onSuccess={noop} onCancel={noop} />,
    target: () => document.body,
  },
  {
    name: 'account settings dialog',
    render: () => (
      <AccountSettingsDialog
        account={ACCOUNT}
        onClose={noop}
        onSaved={noop}
        onToggleEnabled={async () => {}}
        onDelete={async () => {}}
      />
    ),
    target: () => document.body,
  },
  {
    name: 'onboarding',
    render: () => <OnboardingWizard currentLayout="split" onChangeLayout={noop} onComplete={noop} />,
  },
  {
    name: 'lens row drawer',
    render: () => (
      <LensRowDrawer
        row={{
          lensId: 'l1',
          emailId: 'e1',
          accountId: 'a1',
          emailSubject: 'Invoice',
          emailSender: 'ana@example.com',
          emailTimestamp: 1_700_000_000,
          data: {},
          hasOverrides: false,
          promptVersion: 1,
          status: 'done',
          errorMessage: null,
          extractedAt: 1_700_000_000,
        }}
        onClose={noop}
      />
    ),
  },
  {
    name: 'block-sender dialog',
    render: () => <SenderDialogs />,
    open: () =>
      act(() => {
        useSenderStore.getState().openDialog({ type: 'block', accountId: 'a1', address: 'ana@example.com' });
      }),
  },
  {
    name: 'unsubscribe dialog',
    render: () => <SenderDialogs />,
    open: () =>
      act(() => {
        useSenderStore
          .getState()
          .openDialog({ type: 'unsubscribe', accountId: 'a1', emailId: 'e1', senderName: 'Ana' });
      }),
  },
  {
    name: 'shortcut help',
    render: () => <ShortcutHelpModal />,
    open: () =>
      act(() => {
        useShortcutStore.getState().setHelpOpen(true);
      }),
  },
];

const KEYS: { key: string; shiftKey?: boolean }[] = [
  { key: '#', shiftKey: true },
  { key: 'Delete' },
  { key: 'e' },
  { key: 's' },
  { key: 'b' },
  { key: 'j' },
];

function press(key: string, init: KeyboardEventInit, target: EventTarget) {
  const event = new KeyboardEvent('keydown', { key, bubbles: true, cancelable: true, ...init });
  act(() => {
    target.dispatchEvent(event);
  });
}

function expectNothingHappened() {
  for (const fn of Object.values(actions)) expect(fn).not.toHaveBeenCalled();
  expect(useShortcutStore.getState().paneCommand).toBeNull();
  expect(useShortcutStore.getState().bulkSnoozeRequested).toBe(false);
  expect(useShortcutStore.getState().cursorId).toBe('e1');
  expect(useSelectionStore.getState().ids.size).toBe(0);
  expect(host.openEmail).not.toHaveBeenCalled();
}

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  for (const fn of Object.values(actions)) fn.mockClear();
  useEmailStore.setState({ ...actions, emails: LIST, snoozes: new Map() } as never);
  useSelectionStore.getState().clear();
  useShortcutStore.setState({
    enabled: true,
    helpOpen: false,
    paneCommand: null,
    bulkSnoozeRequested: false,
    listEmails: LIST,
    cursorId: 'e1',
  });
  useSenderStore.setState({
    dialog: null,
    statusByEmail: {
      [statusKey('a1', 'e1')]: {
        address: 'ana@example.com',
        blocked: false,
        unsubscribedAt: null,
        unsubscribe: { kind: 'oneClick', target: 'example.com', url: null },
      },
    },
  });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  useOverlayStore.setState({ count: 0 });
});

async function mount(overlay: OverlayCase, over: Partial<GlobalShortcutHost>) {
  host = makeHost(over);
  await act(async () => {
    root.render(<Harness h={host} overlay={overlay.render()} />);
  });
  overlay.open?.();
  // Let effects that load data settle.
  await act(async () => {});
}

describe.each(OVERLAYS)('with the $name open', (overlay) => {
  it.each(KEYS)('$key does not touch the open conversation', async ({ key, shiftKey }) => {
    await mount(overlay, {});
    const target = overlay.target?.() ?? document.activeElement ?? document.body;
    press(key, { shiftKey }, target);
    expectNothingHappened();
  });

  it.each(KEYS)('$key does not touch the cursor row of the full-width list', async ({ key, shiftKey }) => {
    await mount(overlay, { layout: 'full-width', openConversation: false, openEmailId: null });
    const target = overlay.target?.() ?? document.activeElement ?? document.body;
    press(key, { shiftKey }, target);
    expectNothingHappened();
  });
});

describe('the chat panel input', () => {
  it.each(KEYS)('$key typed into the chat is text, not a shortcut', async ({ key, shiftKey }) => {
    await mount({ name: 'chat', render: () => <ChatInput onSend={noop} onClear={noop} disabled={false} /> }, {});
    const textarea = container.querySelector('textarea') as HTMLTextAreaElement;
    act(() => textarea.focus());
    press(key, { shiftKey }, textarea);
    expectNothingHappened();
  });
});

describe('a focused row with its menu closed', () => {
  it('e acts on the open conversation, not on the focused row', async () => {
    await mount({ name: 'row menu', render: () => <EmailActionsMenu email={LIST[1]} onStatus={noop} /> }, {});
    const other = el('[data-testid="other-row"]');
    act(() => other.focus());
    press('e', {}, other);
    expect(useShortcutStore.getState().paneCommand?.command).toBe('archive');
    expect(actions.archiveThreads).not.toHaveBeenCalled();
  });
});

describe('Backspace', () => {
  it.each(['macos', 'windows', 'linux'])('never deletes on %s', async (platform) => {
    const api = await import('@/lib/api');
    vi.spyOn(api, 'currentPlatform').mockReturnValue(platform);
    await mount({ name: 'none', render: () => <span /> }, {});
    press('Backspace', {}, document.body);
    press('Backspace', { metaKey: true }, document.body);
    press('Backspace', { ctrlKey: true }, document.body);
    await mount(
      { name: 'none', render: () => <span /> },
      { layout: 'full-width', openConversation: false, openEmailId: null },
    );
    press('Backspace', {}, document.body);
    expectNothingHappened();
  });
});

describe('Escape closes the menus', () => {
  it('closes the row ⋮ menu and gives the keys back', async () => {
    await mount(OVERLAYS.find((o) => o.name === 'row ⋮ menu') as OverlayCase, {});
    expect(useOverlayStore.getState().count).toBe(1);
    press('Escape', {}, document.body);
    expect(useOverlayStore.getState().count).toBe(0);
    press('e', {}, document.body);
    expect(useShortcutStore.getState().paneCommand?.command).toBe('archive');
  });

  it('closes the snooze picker', async () => {
    await mount(OVERLAYS.find((o) => o.name === 'snooze picker') as OverlayCase, {});
    expect(useOverlayStore.getState().count).toBe(1);
    press('Escape', {}, document.body);
    expect(useOverlayStore.getState().count).toBe(0);
  });
});
