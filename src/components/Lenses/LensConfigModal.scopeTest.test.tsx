// The Scope tab's Test button: runs the scope being edited (unsaved) against
// the mailbox and shows how many emails match plus the most recent ones.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, unknown>) => (opts ? `${key} ${JSON.stringify(opts)}` : key),
    i18n: { language: 'en' },
  }),
}));

const sampleLensScope = vi.fn();
vi.mock('@/lib/api', () => ({
  getFolders: vi.fn().mockResolvedValue([]),
  currentPlatform: () => 'macos',
  sampleLensScope: (...args: unknown[]) => sampleLensScope(...args),
}));

vi.mock('@/stores/lensStore', () => {
  const state = { updateLens: vi.fn() };
  return { useLensStore: (selector: (s: typeof state) => unknown) => selector(state) };
});

vi.mock('@/stores/accountStore', () => {
  const state = { accounts: [] };
  return { useAccountStore: (selector: (s: typeof state) => unknown) => selector(state) };
});

import type { Lens } from '@/types';
import { LensConfigModal } from './LensConfigModal';

const lens = {
  id: 'lens-1',
  name: 'Invoices',
  icon: null,
  templateKey: null,
  accountId: null,
  scope: { mailboxes: ['inbox'], query: 'invoice' },
  schema: { columns: [] },
  promptText: 'Extract.',
  promptVersion: 1,
  modelProvider: null,
  modelName: null,
  isEnabled: true,
  sortOrder: 0,
  createdAt: 0,
  updatedAt: 0,
} as unknown as Lens;

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.clearAllMocks();
});

const button = (label: string) =>
  [...document.querySelectorAll('button')].find((b) => b.textContent === label) as HTMLButtonElement;

function setValue(input: HTMLInputElement, value: string) {
  const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
  set?.call(input, value);
  input.dispatchEvent(new Event('input', { bubbles: true }));
}

const queryInput = () =>
  document.querySelector('input[placeholder="lenses:scope.keywordPlaceholder"]') as HTMLInputElement;

describe('LensConfigModal scope Test button', () => {
  it('tests the unsaved scope and shows the count and the most recent matches', async () => {
    sampleLensScope.mockResolvedValue({
      total: 42,
      capped: false,
      recent: [
        { emailId: 'e3', subject: 'Invoice March', sender: 'Vendor A', timestamp: 1_700_000_300 },
        { emailId: 'e2', subject: 'Invoice February', sender: 'Vendor B', timestamp: 1_700_000_200 },
        { emailId: 'e1', subject: 'Invoice January', sender: 'Vendor C', timestamp: 1_700_000_100 },
      ],
    });
    await act(async () => root.render(<LensConfigModal lens={lens} open onClose={() => {}} />));
    await act(async () => setValue(queryInput(), 'invoice OR receipt'));
    await act(async () => button('lenses:scope.test').click());

    expect(sampleLensScope).toHaveBeenCalledWith(
      expect.objectContaining({ mailboxes: ['inbox'], query: 'invoice OR receipt' }),
    );
    const text = document.body.textContent ?? '';
    expect(text).toContain('lenses:scope.testResult {"count":42}');
    for (const subject of ['Invoice March', 'Invoice February', 'Invoice January']) {
      expect(text).toContain(subject);
    }
  });

  it('says the count is a lower bound when the scope hit the cap', async () => {
    sampleLensScope.mockResolvedValue({ total: 5000, capped: true, recent: [] });
    await act(async () => root.render(<LensConfigModal lens={lens} open onClose={() => {}} />));
    await act(async () => button('lenses:scope.test').click());
    expect(document.body.textContent).toContain('lenses:scope.testResultCapped {"count":5000}');
  });

  it('shows the error when the test fails', async () => {
    sampleLensScope.mockRejectedValue(new Error('fts5: syntax error'));
    await act(async () => root.render(<LensConfigModal lens={lens} open onClose={() => {}} />));
    await act(async () => button('lenses:scope.test').click());
    expect(document.body.textContent).toContain('fts5: syntax error');
  });
});
