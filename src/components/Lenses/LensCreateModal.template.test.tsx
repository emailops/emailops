// Picking a template used to create the Lens on the spot, with no chance to
// choose the account, folders or columns. It now opens the form prefilled.

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
  listLensTemplates: vi.fn(),
  createLensFromTemplate: vi.fn(),
  getFolders: vi.fn().mockResolvedValue([]),
  currentPlatform: () => 'macos',
}));

const createLens = vi.fn();
vi.mock('@/stores/lensStore', () => {
  const state = { createLens: (...args: unknown[]) => createLens(...args) };
  return { useLensStore: (selector: (s: typeof state) => unknown) => selector(state) };
});

vi.mock('@/stores/accountStore', () => {
  const state = { accounts: [{ id: 'acc-1', email: 'owner@example.test' }] };
  return { useAccountStore: (selector: (s: typeof state) => unknown) => selector(state) };
});

import * as api from '@/lib/api';
import { LensCreateModal } from './LensCreateModal';

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  vi.mocked(api.listLensTemplates).mockResolvedValue([
    {
      key: 'contact_form_leads',
      name: 'Contact form leads',
      icon: '📨',
      description: 'People who wrote in.',
      defaultScope: { direction: 'inbound', query: 'contact form', querySearchBody: true },
      schema: {
        columns: [{ key: 'contact_email', label: 'Email', type: 'email', description: '', required: true }],
      },
      prompt: 'Extract the submitter.',
    },
  ]);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.clearAllMocks();
});

describe('LensCreateModal templates', () => {
  it('opens the prefilled form instead of creating the Lens', async () => {
    await act(async () => {
      root.render(<LensCreateModal open onClose={() => {}} onCreated={() => {}} />);
    });
    const card = [...document.querySelectorAll('button')].find((b) => b.textContent?.includes('Contact form leads'));
    expect(card).toBeDefined();
    await act(async () => card?.click());

    expect(api.createLensFromTemplate).not.toHaveBeenCalled();
    expect(createLens).not.toHaveBeenCalled();
    const inputs = [...document.querySelectorAll('input')] as HTMLInputElement[];
    expect(inputs.some((i) => i.value === 'Contact form leads')).toBe(true);
    expect(inputs.some((i) => i.value === 'contact form')).toBe(true);
    const bodySearch = inputs.find((i) => i.type === 'checkbox' && i.checked);
    expect(bodySearch).toBeDefined();
    expect(document.querySelector('textarea')?.value).toBe('Extract the submitter.');
  });

  it('reopens on the Templates tab even after a template switched it to the form', async () => {
    const render = (open: boolean) =>
      root.render(<LensCreateModal open={open} onClose={() => {}} onCreated={() => {}} />);
    await act(async () => render(true));
    const card = [...document.querySelectorAll('button')].find((b) => b.textContent?.includes('Contact form leads'));
    await act(async () => card?.click());
    expect(document.querySelector('textarea')).not.toBeNull(); // on the form now

    await act(async () => render(false));
    await act(async () => render(true));
    expect(document.querySelector('textarea')).toBeNull();
    expect([...document.querySelectorAll('button')].some((b) => b.textContent?.includes('Contact form leads'))).toBe(
      true,
    );
  });
});
