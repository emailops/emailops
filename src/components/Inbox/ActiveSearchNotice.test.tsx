// An empty list under an active search says the search is active and offers
// clearing it — after switching accounts the query carries over, and an empty
// list alone reads as an empty mailbox.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { initI18n } from '@/i18n';
import { ActiveSearchNotice } from './ActiveSearchNotice';

let container: HTMLDivElement;
let root: Root;

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe('ActiveSearchNotice', () => {
  it('names the active search and clears it on request', () => {
    const onClear = vi.fn();
    act(() => root.render(<ActiveSearchNotice query="tag:priority=urgent" onClear={onClear} />));

    expect(container.textContent).toContain('Searching: "tag:priority=urgent"');
    const clear = Array.from(container.querySelectorAll('button')).find((b) => b.textContent === 'Clear search');
    act(() => clear?.click());
    expect(onClear).toHaveBeenCalledOnce();
  });
});
