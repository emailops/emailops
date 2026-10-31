// The doc editor is bound to the shared Y.Doc: text another person wrote
// appears in the open editor, and typing here becomes a Yjs change.

import type { Editor } from '@tiptap/react';
import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import * as Y from 'yjs';

vi.mock('react-i18next', () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));
vi.mock('@tauri-apps/plugin-shell', () => ({ open: vi.fn() }));

import { DOC_FIELD, DocEditor } from './DocEditor';

/** A paragraph written by someone else, as a Yjs update. */
function remoteParagraph(base: Y.Doc, text: string): Uint8Array {
  const other = new Y.Doc();
  Y.applyUpdate(other, Y.encodeStateAsUpdate(base));
  const p = new Y.XmlElement('paragraph');
  p.insert(0, [new Y.XmlText(text)]);
  other.getXmlFragment(DOC_FIELD).insert(other.getXmlFragment(DOC_FIELD).length, [p]);
  return Y.encodeStateAsUpdate(other, Y.encodeStateVector(base));
}

/** A document holding one Yjs element, as a message from someone else would. */
function docWith(element: Y.XmlElement): Y.Doc {
  const doc = new Y.Doc();
  doc.getXmlFragment(DOC_FIELD).insert(0, [element]);
  return doc;
}

// The toolbar focuses the editor, which scrolls the caret into view; jsdom has
// no layout, so ranges get empty rectangles.
const rect = { x: 0, y: 0, top: 0, left: 0, right: 0, bottom: 0, width: 0, height: 0, toJSON: () => ({}) };
Range.prototype.getClientRects ??= () => Object.assign([], { item: () => null }) as unknown as DOMRectList;
Range.prototype.getBoundingClientRect ??= () => rect as DOMRect;

describe('DocEditor', () => {
  let container: HTMLDivElement;
  let root: Root;
  let editor: Editor | null;

  beforeEach(() => {
    (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
    editor = null;
  });

  afterEach(() => {
    act(() => root.unmount());
    container.remove();
  });

  it('asks before leaving through a link in a read-only document', async () => {
    const paragraph = new Y.XmlElement('paragraph');
    const text = new Y.XmlText();
    paragraph.insert(0, [text]);
    text.insert(0, 'read this', { link: { href: 'https://example.com/a', target: '_self' } });

    await act(async () => {
      root.render(<DocEditor doc={docWith(paragraph)} editable={false} onEditor={(e) => (editor = e)} />);
    });
    const event = new MouseEvent('click', { bubbles: true, cancelable: true });
    await act(async () => {
      container.querySelector('a')?.dispatchEvent(event);
    });

    expect(event.defaultPrevented).toBe(true);
    expect(container.querySelector('[data-testid="external-link-open"]')).not.toBeNull();
  });

  it('never gives a remote image in a document an address to fetch', async () => {
    const image = new Y.XmlElement('image');
    image.setAttribute('src', 'https://example.com/pixel.png');

    await act(async () => {
      root.render(<DocEditor doc={docWith(image)} editable={false} onEditor={(e) => (editor = e)} />);
    });

    expect(container.querySelector('img')?.hasAttribute('src')).toBe(false);
  });

  it('shows an image carried in the document', async () => {
    const data = 'data:image/png;base64,iVBORw0KGgo=';
    const image = new Y.XmlElement('image');
    image.setAttribute('src', data);

    await act(async () => {
      root.render(<DocEditor doc={docWith(image)} editable={false} onEditor={(e) => (editor = e)} />);
    });

    expect(container.querySelector('img')?.getAttribute('src')).toBe(data);
  });

  it('shows what another person wrote and records local typing in the doc', async () => {
    const doc = new Y.Doc();
    await act(async () => {
      root.render(<DocEditor doc={doc} editable onEditor={(e) => (editor = e)} />);
    });

    await act(async () => Y.applyUpdate(doc, remoteParagraph(doc, 'From a colleague')));
    expect(container.textContent).toContain('From a colleague');

    await act(async () => {
      editor?.commands.insertContentAt(editor.state.doc.content.size, '<p>Mine</p>');
    });
    expect(doc.getXmlFragment(DOC_FIELD).toString()).toContain('Mine');
  });

  it('a read-only editor has no toolbar and cannot be typed in', async () => {
    const doc = new Y.Doc();
    await act(async () => {
      root.render(<DocEditor doc={doc} editable={false} onEditor={(e) => (editor = e)} />);
    });
    expect(container.querySelector('[aria-label="documents:toolbar.bold"]')).toBeNull();
    expect(editor?.isEditable).toBe(false);
  });

  it('undoes and redoes local typing from the toolbar, never what others wrote', async () => {
    const doc = new Y.Doc();
    await act(async () => {
      root.render(<DocEditor doc={doc} editable onEditor={(e) => (editor = e)} />);
    });
    await act(async () => Y.applyUpdate(doc, remoteParagraph(doc, 'From a colleague')));
    await act(async () => {
      editor?.commands.insertContentAt(editor.state.doc.content.size, '<p>Mine</p>');
    });

    const button = (label: string) => container.querySelector(`[aria-label="${label}"]`) as HTMLButtonElement;
    await act(async () => button('documents:toolbar.undo').click());
    expect(container.textContent).not.toContain('Mine');
    expect(container.textContent).toContain('From a colleague');
    await act(async () => button('documents:toolbar.redo').click());
    expect(container.textContent).toContain('Mine');
  });

  it('undoes on the Edit menu, which arrives as a historyUndo input event', async () => {
    const doc = new Y.Doc();
    await act(async () => {
      root.render(<DocEditor doc={doc} editable onEditor={(e) => (editor = e)} />);
    });
    await act(async () => {
      editor?.commands.insertContentAt(editor.state.doc.content.size, '<p>Mine</p>');
    });
    const target = container.querySelector('[data-testid="shared-doc-editor"]') as HTMLElement;
    await act(async () => {
      target.dispatchEvent(
        new InputEvent('beforeinput', { inputType: 'historyUndo', bubbles: true, cancelable: true }),
      );
    });
    expect(container.textContent).not.toContain('Mine');
  });
});
