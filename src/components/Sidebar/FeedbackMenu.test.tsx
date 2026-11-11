// The sidebar "Give feedback" row: the feedback popover plus the star button
// next to it, which asks before opening the GitHub repository in the system
// browser.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const openExternal = vi.fn((_url: string) => Promise.resolve());
vi.mock('@tauri-apps/plugin-shell', () => ({ open: (url: string) => openExternal(url) }));
vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, vars?: Record<string, unknown>) => (vars ? `${key} ${JSON.stringify(vars)}` : key),
  }),
}));

import { useLogStore } from '@/stores/logStore';
import { FeedbackMenu, GITHUB_REPO_URL } from './FeedbackMenu';

let container: HTMLDivElement;
let root: Root;
const onSelect = vi.fn();

beforeEach(() => {
  vi.clearAllMocks();
  openExternal.mockImplementation(() => Promise.resolve());
  useLogStore.setState({ entries: [] });
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

async function renderMenu() {
  await act(async () => {
    root.render(<FeedbackMenu onSelect={onSelect} />);
  });
}

function button(label: string): HTMLButtonElement {
  const found = [...container.querySelectorAll('button')].find(
    (b) => b.textContent === label || b.getAttribute('aria-label') === label,
  );
  if (!found) throw new Error(`button ${label} not rendered`);
  return found;
}

async function click(label: string) {
  await act(async () => {
    button(label).click();
  });
}

const dialog = () => container.querySelector('[role="dialog"]');

describe('FeedbackMenu star button', () => {
  it('renders an icon-only star button on the same row as Give feedback', async () => {
    await renderMenu();
    const star = button('sidebar:feedback.star');
    const feedback = button('sidebar:feedback.button');
    expect(star.textContent).toBe('');
    expect(star.title).toBe('sidebar:feedback.star');
    expect(star.parentElement).toBe(feedback.parentElement);
    expect(feedback.compareDocumentPosition(star) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it('opens an explanatory dialog instead of the browser', async () => {
    await renderMenu();
    await click('sidebar:feedback.star');
    expect(dialog()?.textContent).toContain('sidebar:feedback.starDialog.title');
    expect(dialog()?.textContent).toContain(`sidebar:feedback.starDialog.body {"url":"${GITHUB_REPO_URL}"}`);
    expect(openExternal).not.toHaveBeenCalled();
  });

  it('opens the repository in the system browser on Visit and closes the dialog', async () => {
    await renderMenu();
    await click('sidebar:feedback.star');
    await click('sidebar:feedback.starDialog.visit');
    expect(openExternal).toHaveBeenCalledWith(GITHUB_REPO_URL);
    expect(dialog()).toBeNull();
  });

  it('opens nothing on Cancel', async () => {
    await renderMenu();
    await click('sidebar:feedback.star');
    await click('common:actions.cancel');
    expect(openExternal).not.toHaveBeenCalled();
    expect(dialog()).toBeNull();
  });

  it('logs a system error when the browser cannot be opened', async () => {
    openExternal.mockImplementation(() => Promise.reject(new Error('no handler')));
    await renderMenu();
    await click('sidebar:feedback.star');
    await click('sidebar:feedback.starDialog.visit');
    const entries = useLogStore.getState().entries;
    expect(entries).toHaveLength(1);
    expect(entries[0].level).toBe('error');
    expect(entries[0].source).toBe('system');
    expect(entries[0].message).toContain('no handler');
  });
});

describe('FeedbackMenu popover', () => {
  it('still lists the three feedback kinds', async () => {
    await renderMenu();
    await click('sidebar:feedback.button');
    const items = [...container.querySelectorAll('[role="menuitem"]')].map((el) => el.textContent);
    expect(items).toEqual(['sidebar:feedback.general', 'sidebar:feedback.bug', 'sidebar:feedback.idea']);
  });

  it('delegates the picked kind to onSelect', async () => {
    await renderMenu();
    await click('sidebar:feedback.button');
    await click('sidebar:feedback.bug');
    expect(onSelect).toHaveBeenCalledWith('bug');
  });
});
