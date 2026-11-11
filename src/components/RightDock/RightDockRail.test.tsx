// The collapsed dock leaves a slim rail on the right edge: the chat button
// first and the calendar button directly below it, each only when that
// content can be docked, so reopening never depends on the current view.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));

import { RightDockRail } from './RightDockRail';

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

const labels = () => [...container.querySelectorAll('button')].map((b) => b.getAttribute('aria-label'));

describe('RightDockRail', () => {
  it('renders the chat button above the calendar button and opens each', () => {
    const onOpen = vi.fn();
    act(() => root.render(<RightDockRail buttons={['chat', 'calendar']} onOpen={onOpen} />));
    expect(labels()).toEqual(['chat:panel.open', 'calendar:panel.open']);

    act(() => container.querySelectorAll('button')[1].click());
    expect(onOpen).toHaveBeenCalledWith('calendar');
    act(() => container.querySelectorAll('button')[0].click());
    expect(onOpen).toHaveBeenLastCalledWith('chat');
  });

  it('renders only the calendar button when chat cannot be docked', () => {
    act(() => root.render(<RightDockRail buttons={['calendar']} onOpen={vi.fn()} />));
    expect(labels()).toEqual(['calendar:panel.open']);
  });
});
