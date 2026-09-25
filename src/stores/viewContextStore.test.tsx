import { act } from 'react';
import { createRoot } from 'react-dom/client';
import { beforeEach, describe, expect, it } from 'vitest';

import type { ChatViewContext } from '@/lib/api';

import { selectChatViewContext, useChatViewContext, useViewContextStore } from './viewContextStore';

describe('selectChatViewContext', () => {
  it('sends nothing when nothing is registered', () => {
    expect(selectChatViewContext({ base: null, form: null })).toBeNull();
  });

  it('sends the base view when no form is open', () => {
    expect(selectChatViewContext({ base: 'view/lenses', form: null })).toEqual({
      token: 'view/lenses',
      formValues: null,
    });
  });

  it('sends a Settings tab the same way', () => {
    expect(selectChatViewContext({ base: 'settings/ai', form: null })).toEqual({
      token: 'settings/ai',
      formValues: null,
    });
  });

  it('prefers the open form over the view behind it', () => {
    // "añade una columna" with the Create Lens dialog up is about the dialog,
    // not about the Lenses view it is sitting on.
    const ctx = selectChatViewContext({
      base: 'view/lenses',
      form: { token: 'form/lens.create', values: { name: 'Facturas' } },
    });
    expect(ctx?.token).toBe('form/lens.create');
  });

  it('sends the open form values so an edit builds on what is on screen', () => {
    const ctx = selectChatViewContext({
      base: 'view/lenses',
      form: { token: 'form/lens.create', values: { name: 'Facturas', columns: [{ key: 'amount' }] } },
    });
    expect(ctx?.formValues).toMatchObject({ name: 'Facturas' });
  });

  it('sends an open form even with no base view registered', () => {
    const ctx = selectChatViewContext({ base: null, form: { token: 'form/lens.create', values: {} } });
    expect(ctx).toEqual({ token: 'form/lens.create', formValues: {} });
  });
});

// Regression: the hook returned a fresh object on every call, so Zustand's
// `useSyncExternalStore` saw a new snapshot each render and React threw
// "The result of getSnapshot should be cached to avoid an infinite loop",
// crashing <ChatPanel> into the error boundary. Caught by the e2e sweep on
// 23/09/2026 — the store tests above all passed, because the bug is about
// reference identity across renders, not about the value.
describe('useChatViewContext', () => {
  beforeEach(() => {
    useViewContextStore.setState({ base: null, form: null });
  });

  function renderHookValue(): { values: (ChatViewContext | null)[]; rerender: () => void; unmount: () => void } {
    const values: (ChatViewContext | null)[] = [];
    function Probe() {
      values.push(useChatViewContext());
      return null;
    }
    const container = document.createElement('div');
    const root = createRoot(container);
    const render = () => act(() => root.render(<Probe />));
    render();
    return { values, rerender: render, unmount: () => act(() => root.unmount()) };
  }

  it('returns the same object across re-renders while nothing on screen changed', () => {
    useViewContextStore.setState({ base: 'view/lenses', form: null });
    const h = renderHookValue();
    h.rerender();
    h.rerender();
    expect(h.values.length).toBeGreaterThanOrEqual(3);
    expect(h.values[0]).toBe(h.values[h.values.length - 1]);
    h.unmount();
  });

  it('returns a new object once the view actually changes', () => {
    useViewContextStore.setState({ base: 'view/lenses', form: null });
    const h = renderHookValue();
    const before = h.values[h.values.length - 1];
    act(() => useViewContextStore.setState({ base: 'view/calendar', form: null }));
    const after = h.values[h.values.length - 1];
    expect(after).not.toBe(before);
    expect(after?.token).toBe('view/calendar');
    h.unmount();
  });
});
