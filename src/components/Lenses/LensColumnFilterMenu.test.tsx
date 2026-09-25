import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));

vi.mock('@/lib/api', () => ({
  getLensColumnValues: vi.fn(),
}));

import * as api from '@/lib/api';
import type { LensColumn } from '@/types';
import { LensColumnFilterMenu } from './LensColumnFilterMenu';

const column: LensColumn = { key: 'vendor', label: 'Vendor', type: 'string', description: '', required: false };

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  vi.mocked(api.getLensColumnValues).mockResolvedValue([
    { value: null, count: 1 },
    { value: 'Acme', count: 2 },
    { value: 'Globex', count: 1 },
  ]);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.clearAllMocks();
});

async function openMenu(onApply = vi.fn(), active?: { key: string; values: string[]; includeEmpty: boolean }) {
  await act(async () => {
    root.render(<LensColumnFilterMenu lensId="lens-1" column={column} active={active} onApply={onApply} />);
  });
  const toggle = container.querySelector('button[aria-label="lenses:table.filter.button"]') as HTMLButtonElement;
  await act(async () => toggle.click());
  return onApply;
}

const checkbox = (label: string) =>
  [...container.querySelectorAll('label')].find((l) => l.textContent?.startsWith(label))?.querySelector('input') as
    | HTMLInputElement
    | undefined;

const button = (label: string) =>
  [...container.querySelectorAll('button')].find((b) => b.textContent === label) as HTMLButtonElement;

describe('LensColumnFilterMenu', () => {
  it('lists every value of the column with its count, all ticked by default', async () => {
    await openMenu();
    expect(api.getLensColumnValues).toHaveBeenCalledWith('lens-1', 'vendor');
    expect(container.textContent).toContain('Acme');
    expect(container.textContent).toContain('(2)');
    expect(checkbox('Globex')?.checked).toBe(true);
    expect(checkbox('lenses:table.filter.empty')?.checked).toBe(true);
  });

  it('applies the ticked values', async () => {
    const onApply = await openMenu();
    act(() => checkbox('Globex')?.click());
    await act(async () => button('lenses:table.filter.apply').click());
    expect(onApply).toHaveBeenCalledWith({ key: 'vendor', values: ['Acme'], includeEmpty: true });
  });

  it('reopens with the active filter ticked, and clears it', async () => {
    const onApply = await openMenu(vi.fn(), { key: 'vendor', values: ['Acme'], includeEmpty: false });
    expect(checkbox('Acme')?.checked).toBe(true);
    expect(checkbox('Globex')?.checked).toBe(false);
    await act(async () => button('lenses:table.filter.clear').click());
    expect(onApply).toHaveBeenCalledWith(null);
  });
});
