// The ✕ that clears the search had no accessible name: screen readers read
// "button" and the verification driver could not address it. It must carry the
// existing `inbox:header.clearSearch` label.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { InboxSearchBox } from './InboxSearchBox';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));
vi.mock('@/lib/api', () => ({
  suggestSenders: vi.fn(() => Promise.resolve([])),
}));

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
});

describe('InboxSearchBox clear button', () => {
  it('labels the ✕ with the clear-search string and clears on click', () => {
    const onClear = vi.fn();
    act(() =>
      root.render(<InboxSearchBox accountId="acct-1" externalQuery="Ollama" onSubmit={() => {}} onClear={onClear} />),
    );
    const clear = container.querySelector('button[aria-label="inbox:header.clearSearch"]');
    expect(clear, 'clear button with an accessible name').not.toBeNull();
    act(() => (clear as HTMLButtonElement).click());
    expect(onClear).toHaveBeenCalledTimes(1);
  });
});
