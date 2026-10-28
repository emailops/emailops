// Regression: with a PDF (or any iframe) open in the reading pane, dragging the
// dock's border left stopped as soon as the pointer crossed into the frame. An
// iframe is its own document, so the parent window gets no pointermove events
// while the pointer is over it and the resize froze. A drag now covers the
// window with a capture layer that sits above every frame.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));

vi.mock('@/hooks/usePersistedPref', async () => {
  const { useState } = await import('react');
  return { usePersistedPref: <T,>(_key: string, initial: T) => useState<T>(initial) };
});

import { RightDock } from './RightDock';

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

const separator = () => container.querySelector('[role="separator"]') as HTMLElement;
const captureLayer = () => document.querySelector('[data-testid="right-dock-drag-capture"]');

// jsdom has no PointerEvent; a MouseEvent with the pointer type name reaches
// React's onPointerDown and the window listeners the same way.
const pointer = (type: string, target: EventTarget, clientX = 0) =>
  act(() => {
    target.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, clientX }));
  });

describe('RightDock drag resize', () => {
  it('covers the window with a capture layer only while dragging', () => {
    act(() => root.render(<RightDock>content</RightDock>));
    expect(captureLayer()).toBeNull();

    pointer('pointerdown', separator());
    expect(captureLayer()).not.toBeNull();

    pointer('pointerup', window);
    expect(captureLayer()).toBeNull();
  });

  it('keeps resizing from moves over the capture layer', () => {
    act(() => root.render(<RightDock>content</RightDock>));
    pointer('pointerdown', separator());
    const layer = captureLayer() as HTMLElement;
    pointer('pointermove', layer, window.innerWidth - 600);
    expect(separator().getAttribute('aria-valuenow')).toBe('600');
  });
});
