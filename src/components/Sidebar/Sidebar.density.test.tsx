// Sidebar density: Comfortable keeps today's classes; Compact tightens the
// vertical spacing of every row in the scrolling list and nothing else.
//
// The fixture turns on every flag the list knows about (AI, tasks, memory,
// skills, lenses, shared docs, calendar), opens "Other Views", and seeds a
// folder and a lens, so all 21 view-row call sites render.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

// Stores the sidebar imports subscribe to backend events at module load.
vi.mock('@tauri-apps/api/event', () => ({ listen: vi.fn(async () => () => {}) }));
// Every backend call resolves to null (VersionLabel's build info, among others).
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

import { initI18n } from '@/i18n';
import type { SidebarDensity } from '@/lib/sidebarDensity';
import { useAiStore } from '@/stores/aiStore';
import { useFolderStore } from '@/stores/folderStore';
import { useLensStore } from '@/stores/lensStore';
import { useMemoryStore } from '@/stores/memoryStore';
import type { Account, SmartFilter } from '@/types';
import { Sidebar } from './Sidebar';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const work: Account = {
  id: 'acct-work',
  provider: 'gmail',
  email: 'work@example.test',
  name: '',
  createdAt: 0,
  sortOrder: 0,
  enabled: true,
  syncFromTimestamp: null,
  color: 'blue',
};
const home: Account = { ...work, id: 'acct-home', provider: 'imap', email: 'home@example.test', sortOrder: 1 };

const smartFilters: SmartFilter[] = [
  { type: 'sender', value: 'ana@example.test', count: 3 },
  { type: 'priority', value: 'urgent', count: 2 },
];

const noop = () => {};

let container: HTMLDivElement;
let root: Root;

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  useAiStore.setState({ enabled: true });
  useFolderStore.setState({
    folders: [
      { id: 'f1', accountId: 'acct-work', serverPath: 'Projects', displayName: 'Projects', role: '', delimiter: '/' },
    ],
    fetchFolders: vi.fn(async () => {}),
  });
  useLensStore.setState({
    lenses: [
      {
        id: 'lens-1',
        name: 'Renewals',
        icon: null,
        templateKey: null,
        accountId: null,
        isEnabled: true,
        sortOrder: 0,
        rowCount: 0,
        staleCount: 0,
      },
    ],
    initialize: vi.fn(async () => {}),
  });
  useMemoryStore.setState({ counts: { totalOpen: 2, overdue: 0, dueToday: 0, awaitingThem: 1 } });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

function renderSidebar(density: SidebarDensity) {
  act(() => {
    root.render(
      <Sidebar
        sidebarDensity={density}
        accounts={[work, home]}
        activeAccount={work}
        isUnifiedActive={false}
        onSelectAccount={noop}
        onAddAccount={noop}
        onMoveAccountUp={noop}
        onMoveAccountDown={noop}
        onToggleAccountEnabled={noop}
        onOpenAccountSettings={noop}
        onSync={noop}
        onOpenSearch={noop}
        onCompose={noop}
        onGiveFeedback={noop}
        onOpenAppSettings={noop}
        isSyncing={false}
        viewMode="inbox"
        onSetViewMode={noop}
        onOpenChatView={noop}
        smartFilters={smartFilters}
        activeFilter={null}
        isLoadingFilters={false}
        onToggleFilter={noop}
        onAppendFilter={noop}
        onClearFilter={noop}
        onPinFilter={noop}
        onUnpinFilter={noop}
        onRemoveFilter={noop}
        onRefreshFilters={noop}
        isFilterPinned={() => false}
        tasksEnabled
        memoriesEnabled
        lensesEnabled
        skillsEnabled
        sharedDocsEnabled
        calendarEntry
        calendarActive={false}
        onOpenCalendarPanel={noop}
        onSelectLens={noop}
      />,
    );
  });
  // "Other Views" starts folded; open it so its rows render too. A re-render on
  // the same root keeps it open, and a second click would fold it again.
  const isOpen = Array.from(container.querySelectorAll('button')).some((b) => b.textContent?.trim() === 'Spam');
  if (!isOpen) {
    const otherViews = buttonWithText(container, 'Other Views');
    act(() => otherViews.click());
  }
}

function nav(): HTMLElement {
  const el = container.querySelector<HTMLElement>('[data-testid="sidebar-nav"]');
  if (!el) throw new Error('sidebar nav not rendered');
  return el;
}

function buttonWithText(scope: ParentNode, text: string): HTMLButtonElement {
  const button = Array.from(scope.querySelectorAll('button')).find((b) => b.textContent?.trim() === text);
  if (!button) throw new Error(`button "${text}" not rendered`);
  return button;
}

/** Elements inside the nav that carry `token` as a whole class (`py-1` must not match `py-1.5`). */
function withClass(token: string): HTMLElement[] {
  return Array.from(nav().querySelectorAll<HTMLElement>('*')).filter((el) => el.classList.contains(token));
}

function focusableLabels(): string[] {
  return Array.from(nav().querySelectorAll<HTMLElement>('button, input, a[href]')).map(
    (el) => `${el.tagName}:${el.getAttribute('title') ?? el.textContent?.trim()}`,
  );
}

describe('Sidebar density', () => {
  it('marks the nav with the density it renders', () => {
    renderSidebar('compact');

    expect(nav().dataset.density).toBe('compact');
  });

  it('keeps comfortable identical to the current markup', () => {
    renderSidebar('comfortable');

    expect(nav().dataset.density).toBe('comfortable');
    expect(nav().className).toBe('flex-1 p-4 space-y-6 overflow-y-auto');
    expect(buttonWithText(nav(), 'Drafts').className).toBe(
      'w-full text-left px-3 py-2 rounded-lg text-sm transition-colors flex items-center gap-2 text-gray-700 hover:bg-gray-100',
    );
    expect(buttonWithText(nav(), 'work@example.test').className).toBe(
      'flex-1 text-left px-3 py-1.5 min-w-0 flex items-center gap-1.5',
    );
    expect(buttonWithText(nav(), 'urgent2').className).toBe(
      'w-full text-left px-3 py-1.5 rounded-lg text-sm transition-colors flex items-center gap-2 text-gray-700 hover:bg-gray-100',
    );
  });

  it('tightens the nav padding and the gap between sections in compact', () => {
    renderSidebar('compact');

    expect(nav().className).toBe('flex-1 px-4 py-2 space-y-3 overflow-y-auto');
  });

  it('gives every row its compact padding in compact', () => {
    renderSidebar('compact');

    expect(withClass('py-1.5')).toEqual([]);
    // The lens chevron keeps py-2 on purpose: the lens row is 28 px either way.
    expect(withClass('py-2').map((el) => el.getAttribute('title'))).toEqual(['Collapse lenses']);
    expect(buttonWithText(nav(), 'Drafts').classList.contains('py-1')).toBe(true);
    expect(buttonWithText(nav(), 'Projects').classList.contains('py-1')).toBe(true);
    expect(buttonWithText(nav(), 'Spam').classList.contains('py-1')).toBe(true);
    expect(buttonWithText(nav(), 'work@example.test').classList.contains('py-1')).toBe(true);
    expect(buttonWithText(nav(), 'All accounts').classList.contains('py-1')).toBe(true);
    expect(buttonWithText(nav(), 'urgent2').classList.contains('py-0.5')).toBe(true);
  });

  it('counts 21 compact view-row paddings, one per call site', () => {
    renderSidebar('compact');

    // 20 view, other-view, folder and AI rows plus the lens row's inner button,
    // the three account rows (All accounts and two accounts), and the one lens
    // in the lens sub-list, which is py-1 in both densities.
    expect(withClass('py-1').filter((el) => el.tagName === 'BUTTON')).toHaveLength(21 + 3 + 1);
  });

  it('leaves the fixed header and the Tasks badge unchanged', () => {
    renderSidebar('comfortable');
    const composeComfortable = buttonWithText(container, 'Compose').className;
    const badgeComfortable = buttonWithText(nav(), 'Tasks3').querySelector('span:last-child')?.className;

    renderSidebar('compact');

    expect(buttonWithText(container, 'Compose').className).toBe(composeComfortable);
    expect(buttonWithText(nav(), 'Tasks3').querySelector('span:last-child')?.className).toBe(badgeComfortable);
  });

  it('keeps the keyboard focus order of the list', () => {
    renderSidebar('comfortable');
    const comfortable = focusableLabels();

    renderSidebar('compact');

    expect(focusableLabels()).toEqual(comfortable);
  });
});
