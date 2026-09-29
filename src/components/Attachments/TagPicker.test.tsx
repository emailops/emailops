// TagPicker: pick attachment-rule tags from the ones already in use (filtered
// as you type) or create a new one.

import { act, useState } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string, opts?: { tag?: string }) => (opts?.tag ? `${key}:${opts.tag}` : key) }),
}));

import { TagPicker } from './TagPicker';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;
let latest: string[] = [];

function Harness({ initial, existing }: { initial: string[]; existing: string[] }) {
  const [tags, setTags] = useState(initial);
  latest = tags;
  return <TagPicker value={tags} onChange={setTags} existingTags={existing} />;
}

function render(initial: string[], existing: string[]) {
  act(() => {
    root.render(<Harness initial={initial} existing={existing} />);
  });
}

function input(): HTMLInputElement {
  const el = container.querySelector('input');
  if (!el) throw new Error('no input');
  return el;
}

async function type(value: string) {
  await act(async () => {
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
    setter?.call(input(), value);
    input().dispatchEvent(new Event('input', { bubbles: true }));
  });
}

async function focus() {
  await act(async () => {
    input().focus();
    input().dispatchEvent(new FocusEvent('focus', { bubbles: true }));
  });
}

function options(): string[] {
  return Array.from(container.querySelectorAll('[role="option"]')).map((o) => o.textContent ?? '');
}

async function clickOption(text: string) {
  const el = Array.from(container.querySelectorAll<HTMLElement>('[role="option"]')).find((o) =>
    o.textContent?.includes(text),
  );
  if (!el) throw new Error(`option ${text} not found in ${options()}`);
  await act(async () => {
    el.dispatchEvent(new MouseEvent('mousedown', { bubbles: true }));
  });
}

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  latest = [];
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe('TagPicker', () => {
  it('lists the existing tags not yet selected when focused', async () => {
    render(['invoice'], ['invoice', 'receipt', 'payroll']);
    await focus();
    expect(options()).toEqual(['receipt', 'payroll']);
  });

  it('filters the existing tags by what is typed', async () => {
    render([], ['invoice', 'receipt', 'payroll']);
    await focus();
    await type('rec');
    expect(options()[0]).toBe('receipt');
    expect(options()).not.toContain('payroll');
  });

  it('adds an existing tag when picked and clears the filter', async () => {
    render([], ['invoice', 'receipt']);
    await focus();
    await clickOption('receipt');
    expect(latest).toEqual(['receipt']);
    expect(input().value).toBe('');
  });

  it('offers to create a tag that does not exist yet', async () => {
    render([], ['invoice']);
    await focus();
    await type('hacienda');
    await clickOption('attachments:rules.createTag:hacienda');
    expect(latest).toEqual(['hacienda']);
  });

  it('does not offer to create a tag that already exists', async () => {
    render([], ['invoice']);
    await focus();
    await type('Invoice');
    expect(options().some((o) => o.startsWith('attachments:rules.createTag'))).toBe(false);
  });

  it('Enter adds the typed tag', async () => {
    render([], []);
    await focus();
    await type('nomina');
    await act(async () => {
      input().dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    });
    expect(latest).toEqual(['nomina']);
  });

  async function key(k: string) {
    await act(async () => {
      input().dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true }));
    });
  }

  it('Enter does not add a tag containing a comma', async () => {
    render([], []);
    await focus();
    await type('a, b');
    await key('Enter');
    expect(latest).toEqual([]);
  });

  it('arrow keys move through the options and Enter picks the highlighted one', async () => {
    render([], ['receipt', 'payroll']);
    await focus();

    await key('ArrowDown');
    await key('ArrowDown');
    await key('ArrowUp');
    const active = input().getAttribute('aria-activedescendant');
    expect(active && container.querySelector(`#${CSS.escape(active)}`)?.textContent).toBe('receipt');

    await key('Enter');
    expect(latest).toEqual(['receipt']);
  });

  it('Escape closes the list', async () => {
    render([], ['receipt']);
    await focus();
    await key('Escape');
    expect(options()).toEqual([]);
    expect(input().getAttribute('aria-expanded')).toBe('false');
  });

  it('the input is a combobox that controls the list', async () => {
    render([], ['receipt']);
    await focus();
    expect(input().getAttribute('role')).toBe('combobox');
    expect(input().getAttribute('aria-expanded')).toBe('true');
    const listId = input().getAttribute('aria-controls');
    expect(listId && container.querySelector(`#${CSS.escape(listId)}`)?.getAttribute('role')).toBe('listbox');
  });

  it('removes a selected tag', async () => {
    render(['invoice', 'acme'], []);
    const remove = container.querySelector<HTMLButtonElement>('button[aria-label="attachments:rules.removeTag:acme"]');
    await act(async () => {
      remove?.click();
    });
    expect(latest).toEqual(['invoice']);
  });
});
