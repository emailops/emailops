// The Config dialog showed scope and prompt only: an existing Lens's columns
// could not be seen, let alone changed, after creation.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, opts?: { defaultValue?: string }) => opts?.defaultValue ?? key,
    i18n: { language: 'en' },
  }),
}));

vi.mock('@/lib/api', () => ({
  getFolders: vi.fn().mockResolvedValue([]),
  currentPlatform: () => 'macos',
}));

const updateLens = vi.fn().mockResolvedValue({});
vi.mock('@/stores/lensStore', () => {
  const state = { updateLens: (...args: unknown[]) => updateLens(...args) };
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
  scope: {},
  schema: {
    columns: [
      { key: 'vendor', label: 'Vendor', type: 'string', description: 'Who billed', required: true },
      { key: 'amount', label: 'Amount', type: 'currency', description: '', required: false },
    ],
  },
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
const keyInputs = () =>
  [...document.querySelectorAll('input[data-testid="lens-create-column-key"]')] as HTMLInputElement[];

function setValue(input: HTMLInputElement, value: string) {
  const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
  set?.call(input, value);
  input.dispatchEvent(new Event('input', { bubbles: true }));
}

async function openColumns() {
  await act(async () => root.render(<LensConfigModal lens={lens} open onClose={() => {}} />));
  await act(async () => button('lenses:columns.title').click());
}

describe('LensConfigModal columns tab', () => {
  it('shows the Lens columns', async () => {
    await openColumns();
    expect(keyInputs().map((i) => i.value)).toEqual(['vendor', 'amount']);
  });

  it('saves edited columns as the Lens schema', async () => {
    await openColumns();
    await act(async () => setValue(keyInputs()[1], 'total'));
    await act(async () => button('common:actions.save').click());
    expect(updateLens).toHaveBeenCalledWith('lens-1', {
      schema: {
        columns: [
          { key: 'vendor', label: 'Vendor', type: 'string', description: 'Who billed', required: true },
          { key: 'total', label: 'Amount', type: 'currency', description: '', required: false },
        ],
      },
    });
  });

  it('refuses to save a column without a key', async () => {
    await openColumns();
    await act(async () => setValue(keyInputs()[0], ''));
    await act(async () => button('common:actions.save').click());
    expect(updateLens).not.toHaveBeenCalled();
    expect(document.body.textContent).toContain('lenses:create.errors.missingKey');
  });
});
