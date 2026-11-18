// #260: a file copied in Finder and pasted into the body is attached, the way a
// dropped one is. Before, the editor inlined pasted images and ignored every
// other file. A drop that mixes images and other files lost the other files:
// the editor took the whole drop for its images.

import { Slice } from '@tiptap/pm/model';
import type { Editor } from '@tiptap/react';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

vi.mock('@/lib/api', () => ({ currentPlatform: () => 'macos' }));

import { initI18n } from '@/i18n';
import { RichTextEditor } from './RichTextEditor';

// A paste scrolls the caret into view; jsdom has no layout, so ranges get
// empty rectangles.
const rect = { x: 0, y: 0, top: 0, left: 0, right: 0, bottom: 0, width: 0, height: 0, toJSON: () => ({}) };
Range.prototype.getClientRects ??= () => Object.assign([], { item: () => null }) as unknown as DOMRectList;
Range.prototype.getBoundingClientRect ??= () => rect as DOMRect;

let container: HTMLDivElement;
let root: Root;

beforeAll(async () => {
  await initI18n('en');
});

beforeEach(() => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
});

const pdf = new File(['%PDF-1.7'], 'report.pdf', { type: 'application/pdf' });
const sheet = new File(['a,b'], 'budget.csv', { type: 'text/csv' });
const png = new File([new Uint8Array([137, 80, 78, 71])], 'chart.png', { type: 'image/png' });
const jpg = new File([new Uint8Array([255, 216, 255])], 'photo.jpg', { type: 'image/jpeg' });

function render(onAttachFiles?: (files: File[]) => void) {
  act(() => root.render(<RichTextEditor value="<p>Hi</p>" onChange={() => {}} onAttachFiles={onAttachFiles} />));
}

/** Tiptap hangs the editor off its contenteditable root. */
function editor(): Editor {
  const dom = container.querySelector('.ProseMirror') as (HTMLElement & { editor?: Editor }) | null;
  if (!dom?.editor) throw new Error('editor not found');
  return dom.editor;
}

/**
 * A paste event as WebKit sends it for a Finder copy: the files, and the file
 * names as plain text.
 */
function paste(files: File[], text = files.map((f) => f.name).join('\n')): Event {
  const event = new Event('paste', { bubbles: true, cancelable: true });
  Object.defineProperty(event, 'clipboardData', {
    value: {
      files,
      items: files.map((file) => ({ kind: 'file', type: file.type, getAsFile: () => file })),
      types: files.length > 0 ? ['Files', 'text/plain'] : ['text/plain'],
      getData: (type: string) => (type === 'text/plain' ? text : ''),
    },
  });
  return event;
}

async function dispatchPaste(event: Event) {
  await act(async () => {
    editor().view.dom.dispatchEvent(event);
  });
  // FileReader resolves the inline images on later ticks.
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 20));
  });
}

/**
 * Hand a drop to the editor's drop handler. ProseMirror finds the drop
 * position from the pointer first, which jsdom cannot lay out.
 */
async function drop(files: File[]): Promise<{ taken: boolean; event: Event }> {
  const event = new Event('drop', { bubbles: true, cancelable: true });
  Object.defineProperty(event, 'dataTransfer', { value: { files, types: ['Files'] } });
  const { view } = editor();
  let taken = false;
  await act(async () => {
    taken = Boolean(view.someProp('handleDrop', (f) => f(view, event as DragEvent, Slice.empty, false)));
  });
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 20));
  });
  return { taken, event };
}

const inlineImages = () => container.querySelectorAll('.ProseMirror img').length;

describe('RichTextEditor pasted and dropped files', () => {
  it('attaches pasted files that are not images, without typing their names into the body', async () => {
    const onAttachFiles = vi.fn();
    render(onAttachFiles);
    const event = paste([pdf, sheet]);
    await dispatchPaste(event);
    expect(onAttachFiles).toHaveBeenCalledTimes(1);
    expect((onAttachFiles.mock.calls[0][0] as File[]).map((f) => f.name)).toEqual(['report.pdf', 'budget.csv']);
    expect(event.defaultPrevented).toBe(true);
    expect(editor().getText()).toBe('Hi');
  });

  it('inlines every pasted image and attaches the other files', async () => {
    const onAttachFiles = vi.fn();
    render(onAttachFiles);
    await dispatchPaste(paste([png, pdf, jpg]));
    expect(inlineImages()).toBe(2);
    expect((onAttachFiles.mock.calls[0][0] as File[]).map((f) => f.name)).toEqual(['report.pdf']);
  });

  it('inlines pasted images and attaches nothing when the paste holds only images', async () => {
    const onAttachFiles = vi.fn();
    render(onAttachFiles);
    await dispatchPaste(paste([png]));
    expect(inlineImages()).toBe(1);
    expect(onAttachFiles).not.toHaveBeenCalled();
  });

  it('pastes text as text', async () => {
    const onAttachFiles = vi.fn();
    render(onAttachFiles);
    act(() => {
      editor().commands.focus('end');
    });
    await dispatchPaste(paste([], ' there'));
    expect(onAttachFiles).not.toHaveBeenCalled();
    expect(editor().getText()).toBe('Hi there');
  });

  it('leaves a pasted non-image file alone where the editor has no attachments (signatures)', async () => {
    render(undefined);
    const event = paste([pdf], '');
    await dispatchPaste(event);
    expect(inlineImages()).toBe(0);
    expect(editor().getText()).toBe('Hi');
  });

  it('inlines the images of a mixed drop and attaches the other files', async () => {
    const onAttachFiles = vi.fn();
    render(onAttachFiles);
    const { taken } = await drop([png, pdf]);
    expect(taken).toBe(true);
    expect(inlineImages()).toBe(1);
    expect((onAttachFiles.mock.calls[0][0] as File[]).map((f) => f.name)).toEqual(['report.pdf']);
  });

  it('leaves a drop with no images to the composer', async () => {
    const onAttachFiles = vi.fn();
    render(onAttachFiles);
    const { taken } = await drop([pdf]);
    expect(taken).toBe(false);
    expect(onAttachFiles).not.toHaveBeenCalled();
  });
});
