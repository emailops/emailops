// Toast stack behavior: regular toasts auto-dismiss; sticky toasts (e.g. the
// app-update notification) stay until the user closes them.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { initI18n } from '@/i18n';
import { useToastStore } from '@/stores/toastStore';
import { ToastHost } from './ToastHost';

let container: HTMLDivElement;
let root: Root;

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  useToastStore.setState({ toasts: [], nextId: 1 });
  vi.useFakeTimers();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  act(() => {
    root.render(<ToastHost />);
  });
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.useRealTimers();
});

describe('ToastHost', () => {
  it('stacks above the Output bar, whose height it reads from a CSS variable', () => {
    act(() => {
      useToastStore.getState().addToast({ message: 'Saved report.pdf' });
    });
    const stack = container.querySelector<HTMLElement>('[data-testid="toast-stack"]');
    expect(stack?.style.bottom).toBe('calc(var(--log-panel-height, 0px) + 1rem)');
  });

  it('auto-dismisses a regular toast after the timeout', () => {
    act(() => {
      useToastStore.getState().addToast({ message: 'Saved report.pdf' });
    });
    expect(container.textContent).toContain('Saved report.pdf');

    act(() => {
      vi.advanceTimersByTime(8_001);
    });
    expect(useToastStore.getState().toasts).toHaveLength(0);
  });

  it('keeps a sticky toast until the user closes it', () => {
    act(() => {
      useToastStore.getState().addToast({ message: 'EmailOps 0.7.0 is available', sticky: true });
    });

    act(() => {
      vi.advanceTimersByTime(60 * 60 * 1000);
    });
    expect(useToastStore.getState().toasts).toHaveLength(1);
    expect(container.textContent).toContain('EmailOps 0.7.0 is available');

    const close = container.querySelector<HTMLButtonElement>('button[aria-label="Close"]');
    expect(close).not.toBeNull();
    act(() => {
      close?.click();
    });
    expect(useToastStore.getState().toasts).toHaveLength(0);
  });

  // Reported once in a demo recording: right after closing a sticky "could
  // not be sent" toast, a bulk delete's "Deleted … · Undo" toast never showed.
  // Dismissing A and adding B in the same tick must leave B up for its full
  // duration — no id reuse, no timer of A's reaching B.
  it('shows a toast added right after a sticky one is dismissed, for its whole duration', () => {
    act(() => {
      const sticky = useToastStore.getState().addToast({ message: 'Message could not be sent', sticky: true });
      useToastStore.getState().dismissToast(sticky);
      useToastStore.getState().addToast({ message: 'Deleted 3 conversations', actionLabel: 'Undo', durationMs: 6000 });
    });
    expect(container.textContent).toContain('Deleted 3 conversations');

    act(() => {
      vi.advanceTimersByTime(5_999);
    });
    expect(container.textContent).toContain('Deleted 3 conversations');

    act(() => {
      vi.advanceTimersByTime(2);
    });
    expect(useToastStore.getState().toasts).toHaveLength(0);
  });

  it('the timer of a dismissed toast does not remove a later one', () => {
    act(() => {
      const first = useToastStore.getState().addToast({ message: 'First' });
      vi.advanceTimersByTime(7_000);
      useToastStore.getState().dismissToast(first);
      useToastStore.getState().addToast({ message: 'Second', durationMs: 6000 });
    });
    act(() => {
      vi.advanceTimersByTime(1_500);
    });
    expect(container.textContent).toContain('Second');
  });

  it('wraps a long message instead of cutting it off, so a fix it explains stays readable', () => {
    const message = 'EmailOps cannot find the CUDA Toolkit. Install it or set CUDA_PATH, then restart EmailOps.';
    act(() => {
      useToastStore.getState().addToast({ message, sticky: true });
    });
    const text = Array.from(container.querySelectorAll('span')).find((s) => s.textContent === message);
    expect(text).toBeDefined();
    expect(text?.className).not.toContain('truncate');
    expect(text?.className).toContain('break-words');
  });
});
