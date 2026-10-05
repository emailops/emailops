// Deleting a document asks first, says what happens to a shared one, and
// keeps a failure on screen without closing.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, vars?: Record<string, unknown>) => (vars ? `${key} ${JSON.stringify(vars)}` : key),
  }),
}));

import { DeleteDocDialog } from './DeleteDocDialog';

describe('DeleteDocDialog', () => {
  let container: HTMLDivElement;
  let root: Root;

  beforeEach(() => {
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
    document.body.innerHTML = '';
  });

  const text = () => document.body.textContent ?? '';
  const confirmButton = () => document.querySelector('[data-testid="delete-doc-confirm"]') as HTMLButtonElement;

  it('deletes only once confirmed, then closes', async () => {
    const onDelete = vi.fn(() => Promise.resolve());
    const onClose = vi.fn();
    await act(async () => {
      root.render(<DeleteDocDialog title="Plan" sharedWith={0} onDelete={onDelete} onClose={onClose} />);
    });
    expect(onDelete).not.toHaveBeenCalled();
    expect(text()).not.toContain('documents:deleteDialog.shared');

    await act(async () => confirmButton().click());
    expect(onDelete).toHaveBeenCalledTimes(1);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('tells that the others keep their copy of a shared document', async () => {
    await act(async () => {
      root.render(<DeleteDocDialog title="Plan" sharedWith={2} onDelete={vi.fn()} onClose={vi.fn()} />);
    });
    expect(text()).toContain('documents:deleteDialog.shared {"count":2}');
  });

  it('keeps the dialog open with the error when deleting fails', async () => {
    const onClose = vi.fn();
    const onDelete = vi.fn(() => Promise.reject(new Error('disk full')));
    await act(async () => {
      root.render(<DeleteDocDialog title="Plan" sharedWith={0} onDelete={onDelete} onClose={onClose} />);
    });
    await act(async () => confirmButton().click());
    expect(document.querySelector('[data-testid="delete-doc-error"]')?.textContent).toBe('disk full');
    expect(onClose).not.toHaveBeenCalled();
  });
});
