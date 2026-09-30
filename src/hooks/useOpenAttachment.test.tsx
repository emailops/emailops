// Opening an attachment with the default app: a type that can run code is
// refused by the backend until the user confirms in a dialog that names the
// file, says what it is and that it came by email.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({
    t: (key: string, vars?: Record<string, unknown>) => (vars ? `${key} ${JSON.stringify(vars)}` : key),
  }),
}));

import { useLogStore } from '@/stores/logStore';
import { useOpenAttachment } from './useOpenAttachment';

const needsConfirmation = (kind: string) => ({
  code: 'attachment_confirmation_required',
  params: { filename: 'run.command', kind },
  message: 'Opening run.command needs confirmation',
});

function Harness({ open, onError }: { open: (confirmed: boolean) => Promise<void>; onError: (err: unknown) => void }) {
  const { openAttachment, confirmDialog } = useOpenAttachment();
  return (
    <>
      <button type="button" data-testid="open" onClick={() => void openAttachment(open).catch(onError)}>
        open
      </button>
      {confirmDialog}
    </>
  );
}

describe('useOpenAttachment', () => {
  let container: HTMLDivElement;
  let root: Root;
  const onError = vi.fn();

  beforeEach(() => {
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    vi.clearAllMocks();
    useLogStore.setState({ entries: [] });
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
  });

  async function mountAndOpen(open: (confirmed: boolean) => Promise<void>) {
    await act(async () => {
      root.render(<Harness open={open} onError={onError} />);
    });
    await click('[data-testid="open"]');
  }

  async function click(selector: string) {
    const el = container.querySelector<HTMLButtonElement>(selector);
    if (!el) throw new Error(`no element for ${selector}`);
    await act(async () => {
      el.click();
    });
  }

  const dialog = () => container.querySelector('[role="dialog"]');

  it('opens a benign attachment without asking', async () => {
    const open = vi.fn(() => Promise.resolve());
    await mountAndOpen(open);

    expect(open).toHaveBeenCalledTimes(1);
    expect(open).toHaveBeenCalledWith(false);
    expect(dialog()).toBeNull();
  });

  it('asks before opening a dangerous attachment, naming the file, its kind and that it came by email', async () => {
    const open = vi.fn((confirmed: boolean) =>
      confirmed ? Promise.resolve() : Promise.reject(needsConfirmation('script')),
    );
    await mountAndOpen(open);

    expect(open).toHaveBeenCalledTimes(1);
    const text = dialog()?.textContent ?? '';
    expect(text).toContain('attachments:openConfirm.fromEmail {"filename":"run.command"}');
    expect(text).toContain('attachments:openConfirm.kinds.script');
    expect(onError).not.toHaveBeenCalled();
  });

  it('opens the attachment as confirmed once the user accepts', async () => {
    const open = vi.fn((confirmed: boolean) =>
      confirmed ? Promise.resolve() : Promise.reject(needsConfirmation('program')),
    );
    await mountAndOpen(open);
    await click('[data-testid="confirm-open-attachment"]');

    expect(open).toHaveBeenLastCalledWith(true);
    expect(open).toHaveBeenCalledTimes(2);
    expect(dialog()).toBeNull();
  });

  it('opens nothing when the user cancels', async () => {
    const open = vi.fn(() => Promise.reject(needsConfirmation('program')));
    await mountAndOpen(open);
    await click('[data-testid="cancel-open-attachment"]');

    expect(open).toHaveBeenCalledTimes(1);
    expect(dialog()).toBeNull();
  });

  it('hands any other failure back to the caller without a dialog', async () => {
    const failure = { code: 'not_found', params: { detail: 'gone' }, message: 'Not found: gone' };
    const open = vi.fn(() => Promise.reject(failure));
    await mountAndOpen(open);

    expect(onError).toHaveBeenCalledWith(failure);
    expect(dialog()).toBeNull();
  });

  it('keeps the dialog open with the error on top when the confirmed open fails, and logs it', async () => {
    const open = vi.fn((confirmed: boolean) =>
      confirmed ? Promise.reject(new Error('no default app')) : Promise.reject(needsConfirmation('installer')),
    );
    await mountAndOpen(open);
    await click('[data-testid="confirm-open-attachment"]');

    const alert = dialog()?.querySelector('[role="alert"]');
    expect(alert?.textContent).toContain('no default app');
    // Pinned above the explanation, not after it.
    expect(alert?.compareDocumentPosition(dialog()?.querySelector('p') as Node)).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
    const entries = useLogStore.getState().entries;
    expect(
      entries.some((e) => e.level === 'error' && e.source === 'system' && e.message.includes('no default app')),
    ).toBe(true);
  });

  it('describes a kind it does not know generically', async () => {
    const open = vi.fn(() => Promise.reject(needsConfirmation('toString')));
    await mountAndOpen(open);

    expect(dialog()?.textContent).toContain('attachments:openConfirm.kinds.other');
  });
});
