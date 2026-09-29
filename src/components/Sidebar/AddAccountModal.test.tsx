// The add-account dialog's confirm button. A failed OAuth add (e.g. a timed-out
// callback) must leave the dialog usable for a retry, and must not escape as an
// unhandled rejection — the caller already shows the error in the dialog.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { initI18n } from '@/i18n';
import { AddAccountModal } from './AddAccountModal';

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

function confirmButton(): HTMLButtonElement {
  const button = Array.from(container.querySelectorAll('button')).find((b) => b.textContent === 'Connect Gmail');
  if (!button) throw new Error('confirm button not found');
  return button;
}

describe('AddAccountModal', () => {
  it('keeps a failed sign-in contained and re-enables the button for a retry', async () => {
    const onConfirm = vi.fn(async () => {
      throw new Error('Timed out waiting for OAuth callback.');
    });
    act(() => {
      root.render(<AddAccountModal onClose={() => {}} onConfirm={onConfirm} providerLabel="Gmail" />);
    });

    await act(async () => {
      confirmButton().click();
    });

    expect(onConfirm).toHaveBeenCalledTimes(1);
    expect(confirmButton().disabled).toBe(false);
  });
});
