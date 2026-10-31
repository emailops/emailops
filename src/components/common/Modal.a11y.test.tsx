// A dialog takes the keyboard while it is open: it has a name, focus moves into
// it and stays there, focus returns to where it was when it closes, and Escape
// closes only the dialog on top, not the one underneath.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: 'en' } }),
}));

import { Modal } from './Modal';

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

const render = (ui: React.ReactNode) => act(async () => root.render(ui));
const overlays = () => [...container.querySelectorAll<HTMLElement>('[data-testid="modal-overlay"]')];
const pressKey = (target: EventTarget, key: string, shiftKey = false) =>
  act(async () => {
    target.dispatchEvent(new KeyboardEvent('keydown', { key, shiftKey, bubbles: true, cancelable: true }));
  });

describe('Modal accessibility', () => {
  it('is named by its title', async () => {
    await render(
      <Modal open onClose={() => {}} title="Delete rule">
        <p>Body</p>
      </Modal>,
    );

    const id = overlays()[0].getAttribute('aria-labelledby');
    expect(id).toBeTruthy();
    expect(document.getElementById(id as string)?.textContent).toBe('Delete rule');
  });

  it('moves focus in on open and gives it back on close', async () => {
    const opener = document.createElement('button');
    document.body.appendChild(opener);
    opener.focus();

    await render(
      <Modal open onClose={() => {}} title="Delete rule">
        <p>Body</p>
      </Modal>,
    );
    expect(overlays()[0].contains(document.activeElement)).toBe(true);

    await render(
      <Modal open={false} onClose={() => {}} title="Delete rule">
        <p>Body</p>
      </Modal>,
    );
    expect(document.activeElement).toBe(opener);
    opener.remove();
  });

  it('leaves focus on the field that asked for it', async () => {
    await render(
      <Modal open onClose={() => {}} title="Rename">
        {/* biome-ignore lint/a11y/noAutofocus: what this test pins */}
        <input data-testid="field" autoFocus />
      </Modal>,
    );

    expect(document.activeElement).toBe(container.querySelector('[data-testid="field"]'));
  });

  it('gives focus back to the opener even when a field took it on open', async () => {
    const opener = document.createElement('button');
    document.body.appendChild(opener);
    opener.focus();

    await render(
      <Modal open onClose={() => {}} title="Rename">
        {/* biome-ignore lint/a11y/noAutofocus: what this test pins */}
        <input autoFocus />
      </Modal>,
    );
    await render(
      <Modal open={false} onClose={() => {}} title="Rename">
        <input />
      </Modal>,
    );

    expect(document.activeElement).toBe(opener);
    opener.remove();
  });

  it('skips a hidden file input when it finds the last stop', async () => {
    await render(
      <Modal open onClose={() => {}} title={null}>
        <button type="button">Only</button>
        <input type="file" className="hidden" />
      </Modal>,
    );
    const buttons = [...overlays()[0].querySelectorAll('button')];
    const first = buttons[0];
    const last = buttons[buttons.length - 1];

    last.focus();
    await pressKey(last, 'Tab');

    expect(document.activeElement).toBe(first);
  });

  it('keeps Tab inside the dialog at both ends', async () => {
    await render(
      <Modal open onClose={() => {}} title={null} footer={<button type="button">Last</button>}>
        <button type="button">First</button>
      </Modal>,
    );
    const buttons = [...overlays()[0].querySelectorAll('button')];
    const first = buttons[0];
    const last = buttons[buttons.length - 1];
    expect(last.textContent).toBe('Last');

    last.focus();
    await pressKey(last, 'Tab');
    expect(document.activeElement).toBe(first);

    await pressKey(first, 'Tab', true);
    expect(document.activeElement).toBe(last);
  });

  it('leaves the keyboard alone when it is non-blocking: the chat keeps it', async () => {
    const chat = document.createElement('textarea');
    document.body.appendChild(chat);
    chat.focus();

    await render(
      <Modal open nonBlocking onClose={() => {}} title="Fill the form">
        <button type="button">Save</button>
      </Modal>,
    );

    expect(document.activeElement).toBe(chat);
    chat.remove();
  });
});

describe('Escape with dialogs on top of each other', () => {
  it('closes only the top one', async () => {
    const closeUnder = vi.fn();
    const closeTop = vi.fn();
    await render(
      <>
        <Modal open onClose={closeUnder} title="Under">
          <p>Under</p>
        </Modal>
        <Modal open onClose={closeTop} title="Top" zIndex={60}>
          <p>Top</p>
        </Modal>
      </>,
    );

    await pressKey(window, 'Escape');

    expect(closeTop).toHaveBeenCalledTimes(1);
    expect(closeUnder).not.toHaveBeenCalled();
  });

  it('closes the one underneath once it is on top again', async () => {
    const closeUnder = vi.fn();
    await render(
      <>
        <Modal open onClose={closeUnder} title="Under">
          <p>Under</p>
        </Modal>
        <Modal open={false} onClose={() => {}} title="Top">
          <p>Top</p>
        </Modal>
      </>,
    );

    await pressKey(window, 'Escape');

    expect(closeUnder).toHaveBeenCalledTimes(1);
  });
});
