// The `?` help modal is rendered from the shortcut registry, so every
// shortcut the app handles is listed — with its label and its keys.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({
  getPref: vi.fn(async () => null),
  setPref: vi.fn(async () => {}),
  currentPlatform: () => 'macos',
}));

import { i18n, initI18n } from '@/i18n';
import { SHORTCUT_GROUPS, SHORTCUTS } from '@/lib/shortcuts';
import { useShortcutStore } from '@/stores/shortcutStore';
import { ShortcutHelpModal } from './ShortcutHelpModal';

let container: HTMLDivElement;
let root: Root;

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  useShortcutStore.setState({ helpOpen: true });
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
    root.render(<ShortcutHelpModal />);
  });
}

describe('ShortcutHelpModal', () => {
  it('renders nothing while closed', async () => {
    useShortcutStore.setState({ helpOpen: false });
    await render();
    expect(document.body.querySelector('[data-testid="shortcut-help"]')).toBeNull();
  });

  it('lists every registry entry with its translated label', async () => {
    await render();
    const items = document.body.querySelectorAll('[data-shortcut-id]');
    expect([...items].map((n) => n.getAttribute('data-shortcut-id'))).toEqual(
      SHORTCUT_GROUPS.flatMap((g) => SHORTCUTS.filter((s) => s.group === g).map((s) => s.id)),
    );
    for (const s of SHORTCUTS) {
      const label = i18n.t(s.labelKey as 'shortcuts:items.next');
      expect(label).not.toBe(s.labelKey);
      const item = document.body.querySelector(`[data-shortcut-id="${s.id}"]`);
      expect(item?.textContent).toContain(label);
    }
  });

  it('shows one heading per group', async () => {
    await render();
    const headings = [...document.body.querySelectorAll('[data-testid="shortcut-group"] h3')].map((h) => h.textContent);
    expect(headings).toEqual(['Navigation', 'Conversation actions', 'Compose', 'Go to', 'Application']);
  });

  it('formats keys for the platform', async () => {
    await render();
    const palette = document.body.querySelector('[data-shortcut-id="app.searchPalette"]');
    expect(palette?.textContent).toContain('⌘K');
    const keys = [...document.body.querySelectorAll('[data-shortcut-id="go.inbox"] kbd')].map((k) => k.textContent);
    expect(keys).toEqual(['g', 'i']);
  });

  it('closing it clears the store flag', async () => {
    await render();
    const close = document.body.querySelector('[data-testid="modal-overlay"] button') as HTMLButtonElement;
    act(() => close.click());
    expect(useShortcutStore.getState().helpOpen).toBe(false);
  });
});
