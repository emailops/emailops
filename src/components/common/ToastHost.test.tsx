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
});
