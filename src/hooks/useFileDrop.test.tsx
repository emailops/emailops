// Dropping a file on a composer attaches it. Before, nothing handled the drop,
// so the webview did its default — it opened the PDF full screen.

import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { installFileDropGuard, useFileDrop } from './useFileDrop';

function Composer({ onFiles }: { onFiles: (files: FileList) => void }) {
  const { isOver, dropProps } = useFileDrop(onFiles);
  return (
    <div data-testid="zone" data-over={String(isOver)} {...dropProps}>
      <textarea />
    </div>
  );
}

/** A drag event carrying `types` (and `files` for a file drag), as WebKit sends it. */
function drag(type: string, types: string[], files: File[] = []): Event {
  const event = new Event(type, { bubbles: true, cancelable: true });
  Object.defineProperty(event, 'dataTransfer', {
    value: { types, files: files as unknown as FileList, dropEffect: 'none' },
  });
  return event;
}

const pdf = new File(['%PDF-1.7'], 'report.pdf', { type: 'application/pdf' });

let container: HTMLDivElement;
let root: Root;
const zone = () => container.querySelector('[data-testid="zone"]') as HTMLElement;
const field = () => container.querySelector('textarea') as HTMLElement;

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

describe('useFileDrop', () => {
  it('attaches files dropped anywhere on the composer, instead of opening them', () => {
    const onFiles = vi.fn();
    act(() => root.render(<Composer onFiles={onFiles} />));
    const drop = drag('drop', ['Files'], [pdf]);
    act(() => {
      field().dispatchEvent(drop);
    });
    expect(onFiles).toHaveBeenCalledTimes(1);
    expect(Array.from(onFiles.mock.calls[0][0] as File[]).map((f) => f.name)).toEqual(['report.pdf']);
    expect(drop.defaultPrevented).toBe(true);
  });

  it('does not attach again a drop the editor already took (an inline image)', () => {
    const onFiles = vi.fn();
    act(() => root.render(<Composer onFiles={onFiles} />));
    field().addEventListener('drop', (e) => e.preventDefault());
    act(() => {
      zone().dispatchEvent(drag('dragenter', ['Files']));
      field().dispatchEvent(drag('drop', ['Files'], [pdf]));
    });
    expect(onFiles).not.toHaveBeenCalled();
    expect(zone().dataset.over).toBe('false');
  });

  it('accepts a file drag while it is over the composer, and shows it', () => {
    act(() => root.render(<Composer onFiles={vi.fn()} />));
    const over = drag('dragover', ['Files']);
    act(() => {
      zone().dispatchEvent(drag('dragenter', ['Files']));
      zone().dispatchEvent(over);
    });
    expect(over.defaultPrevented).toBe(true);
    expect(zone().dataset.over).toBe('true');
    act(() => {
      zone().dispatchEvent(drag('dragleave', ['Files']));
    });
    expect(zone().dataset.over).toBe('false');
  });

  it('leaves drags that carry no files alone (text, an email row)', () => {
    const onFiles = vi.fn();
    act(() => root.render(<Composer onFiles={onFiles} />));
    const over = drag('dragover', ['text/plain']);
    const drop = drag('drop', ['text/plain']);
    act(() => {
      field().dispatchEvent(over);
      field().dispatchEvent(drop);
    });
    expect(onFiles).not.toHaveBeenCalled();
    expect(over.defaultPrevented).toBe(false);
    expect(drop.defaultPrevented).toBe(false);
    expect(zone().dataset.over).toBe('false');
  });
});

describe('installFileDropGuard', () => {
  it('stops a file dropped outside any composer from replacing the app', () => {
    const remove = installFileDropGuard(window);
    try {
      const over = drag('dragover', ['Files']);
      const drop = drag('drop', ['Files'], [pdf]);
      document.body.dispatchEvent(over);
      document.body.dispatchEvent(drop);
      expect(over.defaultPrevented).toBe(true);
      expect(drop.defaultPrevented).toBe(true);
      // Other drags keep their default.
      const text = drag('drop', ['text/plain']);
      document.body.dispatchEvent(text);
      expect(text.defaultPrevented).toBe(false);
    } finally {
      remove();
    }
  });
});
