// SenderPatternInput: the sender field of an attachment rule suggests the
// senders of the rule's account as you type, for the entry under the cursor.

import { act, useState } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const logs = vi.hoisted(() => ({ addLog: vi.fn() }));
vi.mock('@/stores/logStore', () => ({
  useLogStore: (selector: (s: { addLog: typeof logs.addLog }) => unknown) => selector({ addLog: logs.addLog }),
}));
vi.mock('@/lib/api', () => ({ autocompleteSenders: vi.fn() }));

import * as api from '@/lib/api';
import { SenderPatternInput } from './SenderPatternInput';

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

let container: HTMLDivElement;
let root: Root;
let latest = '';

function Harness({ initial, accountId }: { initial: string; accountId: string }) {
  const [value, setValue] = useState(initial);
  latest = value;
  return <SenderPatternInput value={value} onChange={setValue} accountId={accountId} inputId="sender" />;
}

function render(initial = '', accountId = 'acc-1') {
  act(() => root.render(<Harness initial={initial} accountId={accountId} />));
}

const input = () => container.querySelector('input') as HTMLInputElement;
const options = () => Array.from(container.querySelectorAll('[role="option"]')).map((o) => o.textContent ?? '');

/** Types `value` with the cursor at its end, as the user would, and lets the suggestions load. */
async function type(value: string) {
  await act(async () => {
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set;
    setter?.call(input(), value);
    input().setSelectionRange(value.length, value.length);
    input().dispatchEvent(new Event('input', { bubbles: true }));
  });
}

async function key(k: string) {
  await act(async () => {
    input().dispatchEvent(new KeyboardEvent('keydown', { key: k, bubbles: true }));
  });
}

const VENDOR = { email: 'billing@vendor.example', name: 'Vendor Billing' };

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  vi.mocked(api.autocompleteSenders).mockReset().mockResolvedValue([VENDOR]);
  logs.addLog.mockReset();
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe('SenderPatternInput', () => {
  it("suggests the senders of the rule's account matching what is typed", async () => {
    render('', 'acc-2');
    await type('bill');

    expect(api.autocompleteSenders).toHaveBeenLastCalledWith('acc-2', 'bill', 8);
    expect(options()).toEqual(['billing@vendor.exampleVendor Billing']);
  });

  it('asks for nothing until two characters are typed', async () => {
    render();
    await type('b');

    expect(api.autocompleteSenders).not.toHaveBeenCalled();
    expect(options()).toEqual([]);
  });

  it('picking a suggestion replaces only the entry under the cursor', async () => {
    render();
    await type('a@x.example, bil');
    await key('ArrowDown');
    await key('Enter');

    expect(latest).toBe('a@x.example, billing@vendor.example');
    expect(options()).toEqual([]);
  });

  it('a click picks a suggestion too', async () => {
    render();
    await type('bil');
    await act(async () => {
      container
        .querySelector('[role="option"]')
        ?.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, cancelable: true }));
    });

    expect(latest).toBe('billing@vendor.example');
  });

  it('Enter with no suggestion highlighted keeps what was typed', async () => {
    render();
    await type('*vendor*');
    await key('Enter');

    expect(latest).toBe('*vendor*');
  });

  it('Escape closes the suggestions without reaching the dialog', async () => {
    const onDialogKey = vi.fn();
    document.addEventListener('keydown', onDialogKey);
    render();
    await type('bil');
    await key('Escape');
    document.removeEventListener('keydown', onDialogKey);

    expect(options()).toEqual([]);
    expect(onDialogKey).not.toHaveBeenCalled();
  });

  it('a slower answer for earlier text never replaces the latest one', async () => {
    let resolveFirst!: (v: (typeof VENDOR)[]) => void;
    vi.mocked(api.autocompleteSenders)
      .mockReturnValueOnce(new Promise((r) => (resolveFirst = r)))
      .mockResolvedValueOnce([{ email: 'billy@other.example', name: 'Billy' }]);
    render();
    await type('bi');
    await type('billy');
    await act(async () => resolveFirst([VENDOR]));

    expect(options()).toEqual(['billy@other.exampleBilly']);
  });

  it('a failed lookup goes to the output panel and shows nothing', async () => {
    vi.mocked(api.autocompleteSenders).mockRejectedValueOnce(new Error('db locked'));
    render();
    await type('bil');

    expect(options()).toEqual([]);
    expect(logs.addLog).toHaveBeenCalledWith('error', 'attachments', expect.stringContaining('db locked'));
  });
});
