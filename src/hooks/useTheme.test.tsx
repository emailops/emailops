// `useTheme` is the single mechanism every `dark:` class in the app depends on.
// If the class stops landing on <html>, nothing else works and no other test
// notices — every component still renders its light classes happily.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const prefs = new Map<string, string>();

vi.mock('@/lib/api', () => ({
  getPref: vi.fn(async (key: string) => prefs.get(key) ?? null),
  setPref: vi.fn(async (key: string, value: string) => {
    prefs.set(key, value);
  }),
}));

import { useTheme } from './useTheme';

let container: HTMLDivElement;
let root: Root;
let listeners: ((event: { matches: boolean }) => void)[];

/** Stub `matchMedia`, which jsdom does not implement. */
function stubSystem(prefersDark: boolean) {
  listeners = [];
  Object.defineProperty(window, 'matchMedia', {
    writable: true,
    configurable: true,
    value: (query: string) => ({
      matches: prefersDark,
      media: query,
      addEventListener: (_: string, fn: (event: { matches: boolean }) => void) => listeners.push(fn),
      removeEventListener: () => {},
    }),
  });
}

function Probe() {
  useTheme();
  return null;
}

async function mount() {
  await act(async () => {
    root.render(<Probe />);
  });
}

beforeEach(() => {
  prefs.clear();
  document.documentElement.className = '';
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.clearAllMocks();
});

describe('useTheme', () => {
  it('puts the dark class on <html> when the system is dark', async () => {
    stubSystem(true);
    await mount();
    expect(document.documentElement.classList.contains('dark')).toBe(true);
  });

  it('leaves the dark class off when the system is light', async () => {
    stubSystem(false);
    await mount();
    expect(document.documentElement.classList.contains('dark')).toBe(false);
    expect(document.documentElement.classList.contains('light')).toBe(true);
  });

  it('honours a stored preference over the system setting', async () => {
    // The override is the whole reason the setting exists.
    prefs.set('appearance_theme', 'dark');
    stubSystem(false);
    await mount();
    expect(document.documentElement.classList.contains('dark')).toBe(true);
  });

  it('marks the document light explicitly, not just by absence', async () => {
    // The root background rule in index.css needs a class to outrank the
    // `prefers-color-scheme` fallback; "no dark class" is not enough.
    prefs.set('appearance_theme', 'light');
    stubSystem(true);
    await mount();
    expect(document.documentElement.classList.contains('light')).toBe(true);
    expect(document.documentElement.classList.contains('dark')).toBe(false);
  });

  it('follows the system when it changes mid-session', async () => {
    stubSystem(false);
    await mount();
    expect(document.documentElement.classList.contains('dark')).toBe(false);

    await act(async () => {
      for (const fn of listeners) fn({ matches: true });
    });
    expect(document.documentElement.classList.contains('dark')).toBe(true);
  });

  it('does not follow the system while an explicit choice is in force', async () => {
    // A Mac that switches at sunset must not flip an app the user pinned.
    prefs.set('appearance_theme', 'light');
    stubSystem(false);
    await mount();

    await act(async () => {
      for (const fn of listeners) fn({ matches: true });
    });
    expect(document.documentElement.classList.contains('dark')).toBe(false);
  });

  it('sets color-scheme so the browser furniture matches', async () => {
    // Without it a dark app keeps white scrollbars and light form controls.
    stubSystem(true);
    await mount();
    expect(document.documentElement.style.colorScheme).toBe('dark');
  });
});
