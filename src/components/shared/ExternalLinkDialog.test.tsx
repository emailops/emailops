// The dialog that stands between a link and the browser closes like every other
// dialog. It opens from a click that can sit inside an email frame, whose key
// presses never reach the app, so it takes the focus to hear Escape at all.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const { openMock } = vi.hoisted(() => ({ openMock: vi.fn() }));
vi.mock('@tauri-apps/plugin-shell', () => ({ open: openMock }));
vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));

import { Modal } from '@/components/common/Modal';
import { ExternalLinkDialog } from './ExternalLinkDialog';

let container: HTMLDivElement;
let root: Root;

beforeEach(() => {
  openMock.mockReset();
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

const pressEscape = () =>
  act(async () => {
    (document.activeElement ?? document.body).dispatchEvent(
      new KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }),
    );
  });

describe('ExternalLinkDialog', () => {
  it('closes on Escape without opening anything, though the window consumes the key', async () => {
    // The app-wide shortcut handler registers first and consumes every Escape
    // that reaches the window.
    const consume = (e: KeyboardEvent) => e.preventDefault();
    window.addEventListener('keydown', consume);
    const onClose = vi.fn();
    await act(async () => root.render(<ExternalLinkDialog url="https://example.com/a" onClose={onClose} />));

    await pressEscape();
    window.removeEventListener('keydown', consume);

    expect(onClose).toHaveBeenCalledTimes(1);
    expect(openMock).not.toHaveBeenCalled();
  });

  it('takes the focus on Cancel, the choice that leaves nothing open', async () => {
    await act(async () => root.render(<ExternalLinkDialog url="https://example.com/a" onClose={() => {}} />));

    expect(document.activeElement?.textContent).toBe('common:actions.cancel');
  });

  it('closes alone when it sits on top of another dialog', async () => {
    const closeLink = vi.fn();
    const closeBelow = vi.fn();
    const below = (link: boolean) => (
      <Modal open onClose={closeBelow} title="Below">
        {link && <ExternalLinkDialog url="https://example.com/a" onClose={closeLink} />}
      </Modal>
    );
    await act(async () => root.render(below(false)));
    await act(async () => root.render(below(true)));

    await pressEscape();

    expect(closeLink).toHaveBeenCalledTimes(1);
    expect(closeBelow).not.toHaveBeenCalled();
  });
});
