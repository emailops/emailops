// Contacts has no Load more: scrolling near the end asks for the next page, a
// failed page offers Retry under the last row, and the company view, which
// shares the scroll container, never walks the list.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));
vi.mock('@/lib/api', () => ({
  getOrganizationDomain: vi.fn(() => Promise.resolve(null)),
  listContacts: vi.fn(),
  listContactsByCompany: vi.fn(() => Promise.resolve([])),
  getContactDetail: vi.fn(() => Promise.resolve(null)),
  currentPlatform: vi.fn(() => 'macos'),
}));

import * as api from '@/lib/api';
import { ContactsView } from './ContactsView';

function contact(n: number) {
  return { email: `c${n}@empresa.example`, name: `C${n}`, emailCount: 1, lastTimestamp: 1 } as never;
}

function page(from: number, to: number, hasMore: boolean) {
  const items = Array.from({ length: to - from }, (_, i) => contact(from + i));
  return { items, total: 120, hasMore } as never;
}

let container: HTMLDivElement;
let root: Root;

/** jsdom has no layout: give the scroll container a viewport. */
function setLayout(el: HTMLElement, clientHeight: number, scrollHeight: number) {
  Object.defineProperty(el, 'clientHeight', { value: clientHeight, configurable: true });
  Object.defineProperty(el, 'scrollHeight', { value: scrollHeight, configurable: true });
}

const scroller = () => container.querySelector('.overflow-y-auto') as HTMLElement;
const rows = () => container.querySelectorAll('[data-testid="contact-row"]').length;

async function mount() {
  await act(async () => {
    root.render(<ContactsView accountId="acc" onComposeTo={() => {}} onViewEmailsFrom={() => {}} />);
  });
}

async function scroll() {
  await act(async () => {
    scroller().dispatchEvent(new Event('scroll', { bubbles: true }));
  });
}

async function search() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

beforeEach(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  vi.clearAllMocks();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe('ContactsView paging', () => {
  it('has no Load more button', async () => {
    vi.mocked(api.listContacts).mockResolvedValue(page(0, 50, true));
    await mount();
    await search();

    expect(container.textContent).not.toContain('loadMore');
  });

  it('loads the next page near the end of the list', async () => {
    vi.mocked(api.listContacts).mockResolvedValueOnce(page(0, 50, true));
    await mount();
    await search();
    vi.mocked(api.listContacts).mockResolvedValueOnce(page(50, 100, false));
    setLayout(scroller(), 400, 500);

    await scroll();

    expect(rows()).toBe(100);
    expect(api.listContacts).toHaveBeenLastCalledWith('acc', expect.objectContaining({ offset: 50 }));
  });

  it('keeps a contact once when a page repeats it', async () => {
    vi.mocked(api.listContacts).mockResolvedValueOnce(page(0, 50, true));
    await mount();
    await search();
    vi.mocked(api.listContacts).mockResolvedValueOnce(page(49, 60, false));
    setLayout(scroller(), 400, 500);

    await scroll();

    expect(rows()).toBe(60);
  });

  it('does not load while the list is far from its end', async () => {
    vi.mocked(api.listContacts).mockResolvedValueOnce(page(0, 50, true));
    await mount();
    await search();
    setLayout(scroller(), 400, 5000);

    await scroll();

    expect(api.listContacts).toHaveBeenCalledTimes(1);
  });

  it('does not walk a hidden list', async () => {
    vi.mocked(api.listContacts).mockResolvedValueOnce(page(0, 50, true));
    await mount();
    await search();
    setLayout(scroller(), 0, 0);

    await scroll();

    expect(api.listContacts).toHaveBeenCalledTimes(1);
  });

  it('stops at a failed page and offers Retry under the last row', async () => {
    vi.mocked(api.listContacts).mockResolvedValueOnce(page(0, 50, true));
    await mount();
    await search();
    vi.mocked(api.listContacts).mockRejectedValueOnce(new Error('offline'));
    setLayout(scroller(), 400, 500);

    await scroll();
    await scroll();

    expect(api.listContacts).toHaveBeenCalledTimes(2);
    expect(rows()).toBe(50);
    expect(container.textContent).toContain('contacts:view.pageFailed');

    vi.mocked(api.listContacts).mockResolvedValueOnce(page(50, 60, false));
    await act(async () => container.querySelector<HTMLButtonElement>('[data-testid="contacts-retry"]')?.click());

    expect(rows()).toBe(60);
    expect(container.textContent).not.toContain('contacts:view.pageFailed');
  });

  it('does not walk the list from the company view', async () => {
    vi.mocked(api.listContacts).mockResolvedValue(page(0, 50, true));
    await mount();
    await search();
    const company = [...container.querySelectorAll('button')].find(
      (b) => b.textContent === 'contacts:view.modeCompany',
    );
    await act(async () => company?.click());
    await search();
    vi.mocked(api.listContacts).mockClear();
    setLayout(scroller(), 400, 100);

    await scroll();

    expect(api.listContacts).not.toHaveBeenCalled();
  });
});
