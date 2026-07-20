import { beforeEach, describe, expect, it } from 'vitest';
import { useToastStore } from './toastStore';

describe('toastStore', () => {
  beforeEach(() => {
    useToastStore.setState({ toasts: [], nextId: 1 });
  });

  it('adds a toast and returns its id', () => {
    const id = useToastStore.getState().addToast({ message: 'Saved report.pdf' });
    const { toasts } = useToastStore.getState();
    expect(toasts).toHaveLength(1);
    expect(toasts[0].id).toBe(id);
    expect(toasts[0].message).toBe('Saved report.pdf');
  });

  it('keeps the optional action on the toast', () => {
    let clicked = false;
    useToastStore.getState().addToast({
      message: 'Saved',
      actionLabel: 'Show in Finder',
      onAction: () => {
        clicked = true;
      },
    });
    const toast = useToastStore.getState().toasts[0];
    expect(toast.actionLabel).toBe('Show in Finder');
    toast.onAction?.();
    expect(clicked).toBe(true);
  });

  it('dismisses a toast by id', () => {
    const store = useToastStore.getState();
    const a = store.addToast({ message: 'a' });
    const b = store.addToast({ message: 'b' });
    useToastStore.getState().dismissToast(a);
    const { toasts } = useToastStore.getState();
    expect(toasts.map((t) => t.id)).toEqual([b]);
  });

  it('caps the stack at 5 toasts, dropping the oldest', () => {
    const store = useToastStore.getState();
    for (let i = 1; i <= 6; i++) store.addToast({ message: `t${i}` });
    const { toasts } = useToastStore.getState();
    expect(toasts).toHaveLength(5);
    expect(toasts[0].message).toBe('t2');
    expect(toasts[4].message).toBe('t6');
  });
});
