// The collapsed chat leaves a slim rail on the right edge with one button to
// bring it back, so reopening never depends on the view having its own chat
// button.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));

import { ChatPanelRail } from './ChatPanelRail';

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

describe('ChatPanelRail', () => {
  it('reopens the chat from its labelled button', () => {
    const onOpen = vi.fn();
    act(() => root.render(<ChatPanelRail onOpen={onOpen} />));
    const button = container.querySelector('button[aria-label="panel.open"]') as HTMLButtonElement | null;
    expect(button).not.toBeNull();
    act(() => button?.click());
    expect(onOpen).toHaveBeenCalledTimes(1);
  });
});
